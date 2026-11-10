//! Optional diagnostics. Producers never wait for the writer or touch files.
use std::{
    collections::VecDeque,
    fmt::{self, Write as _},
    fs::{File, OpenOptions},
    io::{self, Write},
    os::windows::fs::OpenOptionsExt,
    path::{Path, PathBuf},
    sync::{
        Arc, Mutex, OnceLock, TryLockError,
        atomic::{AtomicBool, AtomicUsize, Ordering},
    },
    thread::{self, JoinHandle, Thread},
    time::{Duration, Instant, SystemTime, UNIX_EPOCH},
};
use windows_sys::Win32::{
    Foundation::{FILETIME, SYSTEMTIME},
    System::{
        Threading::{GetCurrentThread, SetThreadPriority, THREAD_PRIORITY_BELOW_NORMAL},
        Time::FileTimeToSystemTime,
    },
};

const CAPACITY: usize = 128;
const MESSAGE_BYTES: usize = 1024;
const LINE_BYTES: usize = 2048;
const FILE_BYTES: u64 = 1024 * 1024;
const REPEAT_WINDOW: Duration = Duration::from_secs(5);
static LOGGER: OnceLock<Logger> = OnceLock::new();

#[derive(Clone, Copy, PartialEq, Eq)]
enum Level {
    Info,
    Warning,
    Error,
}
impl Level {
    fn name(self) -> &'static str {
        match self {
            Self::Info => "INFO",
            Self::Warning => "WARN",
            Self::Error => "ERROR",
        }
    }
}

// Reserve one byte for a line terminator. Stop formatting as soon as full.
struct Text<const N: usize> {
    bytes: [u8; N],
    len: usize,
    truncated: bool,
}
impl<const N: usize> Text<N> {
    fn new() -> Self {
        Self {
            bytes: [0; N],
            len: 0,
            truncated: false,
        }
    }
    fn text(&self) -> &str {
        std::str::from_utf8(&self.bytes[..self.len]).unwrap_or("")
    }
    fn finish(&mut self) {
        if self.truncated {
            let mut end = self.len.min(N.saturating_sub(4));
            while !self.text().is_char_boundary(end) {
                end -= 1;
            }
            self.len = end;
            self.bytes[end..end + 3].copy_from_slice(b"...");
            self.len += 3;
        }
    }
    fn end_line(&mut self) {
        self.finish();
        self.bytes[self.len] = b'\n';
        self.len += 1;
    }
}
impl<const N: usize> fmt::Write for Text<N> {
    fn write_str(&mut self, text: &str) -> fmt::Result {
        for ch in text.chars() {
            let mut encoded = [0; 4];
            let bytes = match ch {
                '\n' => "\\n",
                '\r' => "\\r",
                '\t' => "\\t",
                ch if ch.is_control() => "?",
                ch => ch.encode_utf8(&mut encoded),
            }
            .as_bytes();
            if self.len + bytes.len() >= N {
                self.truncated = true;
                return Err(fmt::Error);
            }
            self.bytes[self.len..self.len + bytes.len()].copy_from_slice(bytes);
            self.len += bytes.len();
        }
        Ok(())
    }
}

struct Record {
    level: Level,
    component: &'static str,
    utc_ms: u64,
    elapsed_ms: u128,
    message: Text<{ MESSAGE_BYTES + 1 }>,
}
impl Record {
    fn new(state: &State, level: Level, component: &'static str, args: fmt::Arguments<'_>) -> Self {
        let mut message = Text::new();
        let _ = message.write_fmt(args);
        message.finish();
        Self {
            level,
            component,
            utc_ms: utc_ms(),
            elapsed_ms: state.started.elapsed().as_millis(),
            message,
        }
    }
    fn same_error(&self, other: &Self) -> bool {
        self.level != Level::Info
            && self.level == other.level
            && self.component == other.component
            && self.message.text() == other.message.text()
    }
    fn line(&self, state: &State, repeats: usize) -> io::Result<Text<LINE_BYTES>> {
        let ticks = self
            .utc_ms
            .checked_mul(10_000)
            .and_then(|value| value.checked_add(116_444_736_000_000_000))
            .ok_or_else(|| io::Error::other("invalid diagnostic time"))?;
        let file_time = FILETIME {
            dwLowDateTime: ticks as u32,
            dwHighDateTime: (ticks >> 32) as u32,
        };
        let mut time: SYSTEMTIME = unsafe { std::mem::zeroed() };
        if unsafe { FileTimeToSystemTime(&file_time, &mut time) } == 0 {
            return Err(io::Error::last_os_error());
        }
        let mut line = Text::new();
        let _ = write!(
            line,
            "{:04}-{:02}-{:02}T{:02}:{:02}:{:02}.{:03}Z elapsed_ms={} session={:x}-{:x} pid={} {} {}: {}",
            time.wYear,
            time.wMonth,
            time.wDay,
            time.wHour,
            time.wMinute,
            time.wSecond,
            time.wMilliseconds,
            self.elapsed_ms,
            state.pid,
            state.session,
            state.pid,
            self.level.name(),
            self.component,
            self.message.text()
        );
        if repeats > 0 {
            let _ = write!(line, " [repeated {repeats} times]");
        }
        line.end_line();
        Ok(line)
    }
}
fn utc_ms() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_millis()
        .min(u64::MAX as u128) as u64
}

struct State {
    queue: Mutex<VecDeque<Record>>,
    active: AtomicBool,
    stopping: AtomicBool,
    dropped: AtomicUsize,
    writer: OnceLock<Thread>,
    started: Instant,
    session: u64,
    pid: u32,
}
impl State {
    fn wake(&self) {
        if let Some(writer) = self.writer.get() {
            writer.unpark();
        }
    }
}
struct Logger {
    state: Arc<State>,
}
impl Logger {
    fn spawn<F>(open: F) -> (Self, io::Result<JoinHandle<()>>)
    where
        F: FnOnce() -> io::Result<Box<dyn Sink>> + Send + 'static,
    {
        let mut queue = VecDeque::new();
        let available = queue.try_reserve_exact(CAPACITY).is_ok();
        let state = Arc::new(State {
            queue: Mutex::new(queue),
            active: AtomicBool::new(available),
            stopping: AtomicBool::new(false),
            dropped: AtomicUsize::new(0),
            writer: OnceLock::new(),
            started: Instant::now(),
            session: utc_ms(),
            pid: std::process::id(),
        });
        if !available {
            return (
                Self { state },
                Err(io::Error::other("diagnostic queue unavailable")),
            );
        }
        let worker = state.clone();
        let handle = thread::Builder::new()
            .name("typing-log".into())
            .spawn(move || {
                // Priority failure is diagnostic-only and needs no recovery.
                unsafe {
                    SetThreadPriority(GetCurrentThread(), THREAD_PRIORITY_BELOW_NORMAL);
                }
                struct Disable(Arc<State>);
                impl Drop for Disable {
                    fn drop(&mut self) {
                        self.0.active.store(false, Ordering::Release);
                        if let Ok(mut queue) = self.0.queue.lock() {
                            queue.clear();
                        }
                    }
                }
                let _disable = Disable(worker.clone());
                if let Ok(mut sink) = open() {
                    let _ = drain(&worker, &mut *sink);
                }
            });
        match &handle {
            Ok(handle) => {
                let _ = state.writer.set(handle.thread().clone());
            }
            Err(_) => state.active.store(false, Ordering::Release),
        }
        (Self { state }, handle)
    }
    fn submit(&self, level: Level, component: &'static str, args: fmt::Arguments<'_>) {
        if !self.state.active.load(Ordering::Acquire) || self.state.stopping.load(Ordering::Relaxed)
        {
            return;
        }
        let record = Record::new(&self.state, level, component, args);
        match self.state.queue.try_lock() {
            Ok(mut queue) => {
                if self.state.stopping.load(Ordering::Relaxed)
                    || !self.state.active.load(Ordering::Acquire)
                {
                    return;
                }
                if queue.len() < CAPACITY {
                    queue.push_back(record);
                } else {
                    self.state.dropped.fetch_add(1, Ordering::Relaxed);
                }
            }
            Err(TryLockError::WouldBlock) => {
                self.state.dropped.fetch_add(1, Ordering::Relaxed);
            }
            Err(TryLockError::Poisoned(_)) => self.state.active.store(false, Ordering::Release),
        }
        self.state.wake();
    }
    fn stop(&self) {
        self.state.stopping.store(true, Ordering::Release);
        self.state.active.store(false, Ordering::Release);
        self.state.wake();
    }
}

pub fn start(path: PathBuf, version: &'static str) {
    let (logger, _detached) =
        Logger::spawn(move || Ok(Box::new(RotatingFile::open(&path, FILE_BYTES)?)));
    if LOGGER.set(logger).is_ok() {
        info(
            "session",
            format_args!("start version={version} arch={}", std::env::consts::ARCH),
        );
    }
}
pub fn info(component: &'static str, args: fmt::Arguments<'_>) {
    submit(Level::Info, component, args);
}
pub fn warning(component: &'static str, args: fmt::Arguments<'_>) {
    submit(Level::Warning, component, args);
}
pub fn error(component: &'static str, args: fmt::Arguments<'_>) {
    submit(Level::Error, component, args);
}
fn submit(level: Level, component: &'static str, args: fmt::Arguments<'_>) {
    if let Some(logger) = LOGGER.get() {
        logger.submit(level, component, args);
    }
}
pub fn shutdown() {
    if let Some(logger) = LOGGER.get() {
        logger.stop();
    }
}

trait Sink: Send {
    fn write(&mut self, line: &[u8]) -> io::Result<()>;
}
struct Repeats {
    last: Option<Record>,
    count: usize,
    emitted: Instant,
}
impl Repeats {
    fn new() -> Self {
        Self {
            last: None,
            count: 0,
            emitted: Instant::now(),
        }
    }
    fn flush(&mut self, state: &State, sink: &mut dyn Sink, now: Instant) -> io::Result<()> {
        if self.count > 0 {
            if let Some(last) = &mut self.last {
                last.utc_ms = utc_ms();
                last.elapsed_ms = state.started.elapsed().as_millis();
                let line = last.line(state, self.count)?;
                sink.write(&line.bytes[..line.len])?;
            }
            self.count = 0;
        }
        self.emitted = now;
        Ok(())
    }
    fn record(
        &mut self,
        record: Record,
        state: &State,
        sink: &mut dyn Sink,
        now: Instant,
    ) -> io::Result<()> {
        if self
            .last
            .as_ref()
            .is_some_and(|last| last.same_error(&record))
        {
            self.count = self.count.saturating_add(1);
            if now.duration_since(self.emitted) >= REPEAT_WINDOW {
                self.flush(state, sink, now)?;
            }
        } else {
            self.flush(state, sink, now)?;
            let line = record.line(state, 0)?;
            sink.write(&line.bytes[..line.len])?;
            self.last = Some(record);
        }
        Ok(())
    }
}
fn drain(state: &State, sink: &mut dyn Sink) -> io::Result<()> {
    let mut repeats = Repeats::new();
    loop {
        let record = state
            .queue
            .lock()
            .map_err(|_| io::Error::other("diagnostic queue poisoned"))?
            .pop_front();
        let dropped = state.dropped.swap(0, Ordering::Relaxed);
        if dropped > 0 {
            repeats.record(
                Record::new(
                    state,
                    Level::Warning,
                    "logger",
                    format_args!("dropped {dropped} records"),
                ),
                state,
                sink,
                Instant::now(),
            )?;
        }
        if let Some(record) = record {
            repeats.record(record, state, sink, Instant::now())?;
            continue;
        }
        let now = Instant::now();
        if state.stopping.load(Ordering::Acquire) || !state.active.load(Ordering::Acquire) {
            repeats.flush(state, sink, now)?;
            return Ok(());
        }
        if repeats.count > 0 {
            let remaining = REPEAT_WINDOW.saturating_sub(now.duration_since(repeats.emitted));
            if remaining.is_zero() {
                repeats.flush(state, sink, now)?;
            } else {
                thread::park_timeout(remaining);
            }
        } else {
            thread::park();
        }
    }
}

struct RotatingFile {
    path: PathBuf,
    // Close output before releasing destination ownership during teardown.
    file: Option<File>,
    _ownership: File,
    size: u64,
    limit: u64,
}
fn suffixed(path: &Path, suffix: &str) -> PathBuf {
    let mut name = path.as_os_str().to_os_string();
    name.push(suffix);
    name.into()
}
fn managed_size(path: &Path) -> io::Result<Option<u64>> {
    match std::fs::symlink_metadata(path) {
        Ok(metadata) if metadata.file_type().is_file() => Ok(Some(metadata.len())),
        Ok(_) => Err(io::Error::other(
            "diagnostic destination is not a regular file",
        )),
        Err(error) if error.kind() == io::ErrorKind::NotFound => Ok(None),
        Err(error) => Err(error),
    }
}
impl RotatingFile {
    fn open(path: &Path, limit: u64) -> io::Result<Self> {
        if let Some(parent) = path.parent().filter(|path| !path.as_os_str().is_empty()) {
            std::fs::create_dir_all(parent)?;
        }
        let ownership_path = suffixed(path, ".lock");
        managed_size(&ownership_path)?;
        let ownership = OpenOptions::new()
            .read(true)
            .write(true)
            .create(true)
            .truncate(false)
            .share_mode(0)
            .open(ownership_path)?;
        ownership.set_len(0)?;
        for suffix in ["", ".1", ".2", ".3"] {
            let file = suffixed(path, suffix);
            if managed_size(&file)?.is_some_and(|size| size > limit) {
                std::fs::remove_file(file)?;
            }
        }
        let file = Self::append_file(path)?;
        let mut size = file.metadata()?.len();
        if size > limit {
            file.set_len(0)?;
            size = 0;
        }
        let mut this = Self {
            path: path.to_owned(),
            _ownership: ownership,
            file: Some(file),
            size,
            limit,
        };
        if size > 0 {
            this.write(b"\n")?;
        }
        Ok(this)
    }
    fn append_file(path: &Path) -> io::Result<File> {
        // FILE_SHARE_READ: allow inspection, exclude other writers/deleters.
        OpenOptions::new()
            .append(true)
            .create(true)
            .share_mode(1)
            .open(path)
    }
    fn rotate(&mut self) -> io::Result<()> {
        self.file.take();
        let oldest = suffixed(&self.path, ".3");
        if managed_size(&oldest)?.is_some() {
            std::fs::remove_file(oldest)?;
        }
        for (from, to) in [(".2", ".3"), (".1", ".2"), ("", ".1")] {
            let source = suffixed(&self.path, from);
            if managed_size(&source)?.is_some() {
                std::fs::rename(source, suffixed(&self.path, to))?;
            }
        }
        self.file = Some(Self::append_file(&self.path)?);
        self.size = 0;
        Ok(())
    }
}
impl Sink for RotatingFile {
    fn write(&mut self, line: &[u8]) -> io::Result<()> {
        if line.len() as u64 > self.limit {
            return Err(io::Error::other("diagnostic record exceeds file limit"));
        }
        if self.size + line.len() as u64 > self.limit {
            self.rotate()?;
        }
        self.file
            .as_mut()
            .ok_or_else(|| io::Error::other("diagnostic file unavailable"))?
            .write_all(line)?;
        self.size += line.len() as u64;
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::mpsc;

    static NEXT: AtomicUsize = AtomicUsize::new(0);
    struct Directory(PathBuf);
    impl Directory {
        fn new() -> Self {
            let path = std::env::current_dir()
                .unwrap()
                .join("target/logging-tests")
                .join(format!(
                    "{}-{}",
                    std::process::id(),
                    NEXT.fetch_add(1, Ordering::Relaxed)
                ));
            std::fs::create_dir_all(&path).unwrap();
            Self(path)
        }
        fn log(&self) -> PathBuf {
            self.0.join("typing.log")
        }
    }
    impl Drop for Directory {
        fn drop(&mut self) {
            let root = std::env::current_dir()
                .unwrap()
                .join("target/logging-tests");
            assert!(self.0.is_absolute() && self.0.starts_with(root));
            let _ = std::fs::remove_dir_all(&self.0);
        }
    }
    #[derive(Clone, Default)]
    struct Memory(Arc<Mutex<Vec<u8>>>);
    impl Sink for Memory {
        fn write(&mut self, line: &[u8]) -> io::Result<()> {
            self.0.lock().unwrap().extend_from_slice(line);
            Ok(())
        }
    }
    fn state() -> State {
        State {
            queue: Mutex::new(VecDeque::with_capacity(CAPACITY)),
            active: AtomicBool::new(true),
            stopping: AtomicBool::new(false),
            dropped: AtomicUsize::new(0),
            writer: OnceLock::new(),
            started: Instant::now(),
            session: 123,
            pid: 42,
        }
    }

    #[test]
    fn records_have_utc_identity_levels_and_bounded_single_line_utf8() {
        let state = state();
        let mut record = Record::new(
            &state,
            Level::Error,
            "output",
            format_args!("bad\ntext\r\t{}", "界".repeat(2000)),
        );
        record.utc_ms = 0;
        let line = record.line(&state, 0).unwrap();
        let text = std::str::from_utf8(&line.bytes[..line.len]).unwrap();
        assert!(text.starts_with("1970-01-01T00:00:00.000Z elapsed_ms="));
        assert!(text.contains("session=2a-7b pid=42 ERROR output: bad\\ntext\\r\\t"));
        assert_eq!(text.matches('\n').count(), 1);
        assert!(text.ends_with("...\n"));
        assert!(record.message.len <= MESSAGE_BYTES && line.len <= LINE_BYTES);
        record.utc_ms = 1_709_251_199_123;
        assert!(
            record
                .line(&state, 0)
                .unwrap()
                .text()
                .starts_with("2024-02-29T23:59:59.123Z")
        );
    }

    #[test]
    fn restart_appends_and_rotation_keeps_only_three_bounded_archives() {
        let directory = Directory::new();
        let path = directory.log();
        {
            let mut file = RotatingFile::open(&path, 64).unwrap();
            file.write(b"session one\n").unwrap();
        }
        {
            let mut file = RotatingFile::open(&path, 64).unwrap();
            file.write(b"session two\n").unwrap();
        }
        let text = std::fs::read_to_string(&path).unwrap();
        assert!(text.contains("session one\n\nsession two"));
        let mut file = RotatingFile::open(&path, 64).unwrap();
        for i in 0..30 {
            file.write(format!("record {i:02} 012345678901234567890\n").as_bytes())
                .unwrap();
        }
        drop(file);
        let mut total = 0;
        for suffix in ["", ".1", ".2", ".3"] {
            let metadata = std::fs::metadata(suffixed(&path, suffix)).unwrap();
            assert!(metadata.len() <= 64);
            total += metadata.len();
        }
        assert!(total <= 256);
        assert_eq!(std::fs::read_dir(&directory.0).unwrap().count(), 5);
        assert!(
            std::fs::read_to_string(&path)
                .unwrap()
                .contains("record 29")
        );
        assert!(
            !std::fs::read_to_string(suffixed(&path, ".3"))
                .unwrap()
                .contains("record 00")
        );
    }

    #[test]
    fn oversized_owned_files_are_discarded_but_other_files_survive() {
        let directory = Directory::new();
        let path = directory.log();
        for suffix in ["", ".1", ".2", ".3"] {
            std::fs::write(suffixed(&path, suffix), vec![b'x'; 100]).unwrap();
        }
        let unrelated = directory.0.join("notes.txt");
        std::fs::write(&unrelated, "keep").unwrap();
        let mut file = RotatingFile::open(&path, 64).unwrap();
        file.write(b"new session\n").unwrap();
        drop(file);
        assert_eq!(std::fs::read(&path).unwrap(), b"new session\n");
        for suffix in [".1", ".2", ".3"] {
            assert!(!suffixed(&path, suffix).exists());
        }
        assert_eq!(std::fs::read_to_string(unrelated).unwrap(), "keep");
    }

    #[test]
    fn destination_ownership_excludes_competitors_without_truncating() {
        let directory = Directory::new();
        let path = directory.log();
        let mut first = RotatingFile::open(&path, 64).unwrap();
        first.write(b"first\n").unwrap();
        assert!(RotatingFile::open(&path, 64).is_err());
        assert_eq!(std::fs::read(&path).unwrap(), b"first\n");
        drop(first);
        let mut second = RotatingFile::open(&path, 64).unwrap();
        second.write(b"second\n").unwrap();
        drop(second);
        assert_eq!(std::fs::read(&path).unwrap(), b"first\n\nsecond\n");
    }

    #[test]
    fn creation_and_rotation_fail_without_touching_unowned_directories() {
        let directory = Directory::new();
        let blocked = directory.0.join("not-a-directory");
        std::fs::write(&blocked, "keep").unwrap();
        let (logger, worker) =
            Logger::spawn(move || Ok(Box::new(RotatingFile::open(&blocked.join("log"), 64)?)));
        worker.unwrap().join().unwrap();
        assert!(!logger.state.active.load(Ordering::Acquire));
        let path = directory.log();
        let mut file = RotatingFile::open(&path, 8).unwrap();
        file.write(b"12345678").unwrap();
        std::fs::create_dir(suffixed(&path, ".3")).unwrap();
        assert!(file.write(b"x").is_err());
        assert!(suffixed(&path, ".3").is_dir());
        assert_eq!(std::fs::read(&path).unwrap(), b"12345678");
    }

    #[test]
    fn blocked_startup_full_queue_contention_and_stop_never_wait_for_writer() {
        let memory = Memory::default();
        let output = memory.clone();
        let (ready, waiting) = mpsc::channel();
        let (release, gate) = mpsc::channel();
        let (logger, worker) = Logger::spawn(move || {
            ready.send(()).unwrap();
            gate.recv().unwrap();
            Ok(Box::new(output))
        });
        waiting.recv_timeout(Duration::from_secs(2)).unwrap();
        let begin = Instant::now();
        for i in 0..10_000 {
            logger.submit(Level::Info, "test", format_args!("record {i}"));
        }
        assert_eq!(logger.state.queue.lock().unwrap().len(), CAPACITY);
        assert!(logger.state.dropped.load(Ordering::Relaxed) > 0);
        {
            let _held = logger.state.queue.lock().unwrap();
            logger.submit(Level::Error, "test", format_args!("contention"));
            logger.stop();
        }
        let elapsed = begin.elapsed();
        release.send(()).unwrap();
        worker.unwrap().join().unwrap();
        assert!(
            elapsed < Duration::from_secs(1),
            "producer/stop time: {elapsed:?}"
        );
        let bytes = memory.0.lock().unwrap();
        let text = std::str::from_utf8(&bytes).unwrap();
        assert!(text.contains("dropped "));
        assert!(text.lines().count() <= CAPACITY + 1);
        println!("10,000 submissions plus contended submit/stop with blocked writer: {elapsed:?}");
    }

    #[test]
    fn a_blocked_disk_write_does_not_block_producers_or_stop() {
        struct Blocked {
            entered: mpsc::Sender<()>,
            release: mpsc::Receiver<()>,
        }
        impl Sink for Blocked {
            fn write(&mut self, _: &[u8]) -> io::Result<()> {
                let _ = self.entered.send(());
                let _ = self.release.recv();
                Err(io::Error::other("injected storage failure"))
            }
        }
        let (entered, waiting) = mpsc::channel();
        let (release, gate) = mpsc::channel();
        let (logger, worker) = Logger::spawn(move || {
            Ok(Box::new(Blocked {
                entered,
                release: gate,
            }))
        });
        logger.submit(Level::Info, "test", format_args!("first"));
        waiting.recv_timeout(Duration::from_secs(2)).unwrap();
        let (finished, completion) = mpsc::channel();
        let producer = thread::spawn(move || {
            let begin = Instant::now();
            for _ in 0..1000 {
                logger.submit(Level::Error, "test", format_args!("while blocked"));
            }
            logger.stop();
            finished.send(begin.elapsed()).unwrap();
        });
        let elapsed = completion.recv_timeout(Duration::from_secs(1));
        release.send(()).unwrap();
        producer.join().unwrap();
        worker.unwrap().join().unwrap();
        assert!(elapsed.is_ok(), "producer or shutdown waited for disk I/O");
        println!(
            "1,000 submissions and stop during blocked disk write: {:?}",
            elapsed.unwrap()
        );
    }

    #[test]
    fn writer_failure_disables_and_discards_without_formatting_more_records() {
        struct Failed;
        impl Sink for Failed {
            fn write(&mut self, _: &[u8]) -> io::Result<()> {
                Err(io::Error::other("disk full"))
            }
        }
        let (logger, worker) = Logger::spawn(|| Ok(Box::new(Failed)));
        logger.submit(Level::Error, "test", format_args!("failure"));
        worker.unwrap().join().unwrap();
        assert!(!logger.state.active.load(Ordering::Acquire));
        assert!(logger.state.queue.lock().unwrap().is_empty());
        struct Unreachable;
        impl fmt::Display for Unreachable {
            fn fmt(&self, _: &mut fmt::Formatter<'_>) -> fmt::Result {
                panic!("disabled logger formatted an argument")
            }
        }
        logger.submit(Level::Error, "test", format_args!("{}", Unreachable));
        logger.stop();
    }

    #[test]
    fn a_record_formatted_during_writer_failure_is_not_enqueued_after_disable() {
        struct Failed;
        impl Sink for Failed {
            fn write(&mut self, _: &[u8]) -> io::Result<()> {
                Err(io::Error::other("storage failed"))
            }
        }
        struct PausedFormat {
            entered: mpsc::Sender<()>,
            release: mpsc::Receiver<()>,
        }
        impl fmt::Display for PausedFormat {
            fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
                self.entered.send(()).unwrap();
                self.release.recv().unwrap();
                formatter.write_str("late record")
            }
        }
        let (logger, worker) = Logger::spawn(|| Ok(Box::new(Failed)));
        let logger = Arc::new(logger);
        let producer_logger = logger.clone();
        let (entered, waiting) = mpsc::channel();
        let (release, gate) = mpsc::channel();
        let producer = thread::spawn(move || {
            producer_logger.submit(
                Level::Info,
                "test",
                format_args!(
                    "{}",
                    PausedFormat {
                        entered,
                        release: gate
                    }
                ),
            );
        });
        waiting.recv_timeout(Duration::from_secs(2)).unwrap();
        logger.submit(Level::Error, "test", format_args!("fail writer now"));
        worker.unwrap().join().unwrap();
        release.send(()).unwrap();
        producer.join().unwrap();
        assert!(!logger.state.active.load(Ordering::Acquire));
        assert!(logger.state.queue.lock().unwrap().is_empty());
    }

    #[test]
    fn a_poisoned_logging_queue_is_abandoned_without_producer_panic() {
        let (logger, worker) = Logger::spawn(|| Ok(Box::new(Memory::default())));
        let _ = std::panic::catch_unwind(|| {
            let _held = logger.state.queue.lock().unwrap();
            panic!("injected logging queue poison");
        });
        logger.submit(Level::Error, "test", format_args!("after poison"));
        worker.unwrap().join().unwrap();
        assert!(!logger.state.active.load(Ordering::Acquire));
        logger.stop();
    }

    #[test]
    fn repeated_errors_preserve_first_record_and_bounded_summaries() {
        let state = state();
        let mut sink = Memory::default();
        let mut repeats = Repeats::new();
        let now = Instant::now();
        for _ in 0..100 {
            repeats
                .record(
                    Record::new(&state, Level::Error, "output", format_args!("same failure")),
                    &state,
                    &mut sink,
                    now,
                )
                .unwrap();
        }
        assert_eq!(
            sink.0
                .lock()
                .unwrap()
                .iter()
                .filter(|&&byte| byte == b'\n')
                .count(),
            1
        );
        repeats
            .flush(&state, &mut sink, now + REPEAT_WINDOW)
            .unwrap();
        repeats
            .record(
                Record::new(&state, Level::Info, "lifecycle", format_args!("changed")),
                &state,
                &mut sink,
                now + REPEAT_WINDOW,
            )
            .unwrap();
        let bytes = sink.0.lock().unwrap();
        let text = std::str::from_utf8(&bytes).unwrap();
        assert_eq!(text.lines().count(), 3);
        assert!(text.contains("same failure [repeated 99 times]"));
        assert!(text.contains("INFO lifecycle: changed"));
    }
}
