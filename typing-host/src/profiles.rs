//! Profile documents and editing policy; interpretation remains in typing-core.
use std::{
    collections::BTreeSet,
    fs,
    io::Write,
    path::{Path, PathBuf},
    sync::atomic::{AtomicU64, Ordering},
};
use typing_core::{BindingGroup, Chord, Command, Config, validate_binding};

#[derive(Clone, Debug)]
pub struct Entry {
    pub line: usize,
    pub enabled: bool,
    pub section: String,
    pub input: String,
    pub action: String,
    pub chord: Chord,
    pub command: Command,
}

#[derive(Clone, Debug)]
pub struct Profile {
    pub name: String,
    pub config: Config,
    pub entries: Vec<Entry>,
    pub warnings: Vec<String>,
}

fn diagnostics(errors: Vec<typing_core::ConfigError>) -> String {
    errors
        .into_iter()
        .map(|e| format!("Line {}: {}", e.line, e.message))
        .collect::<Vec<_>>()
        .join("\r\n")
}

fn global_collision(input: Chord, global: Chord) -> bool {
    use typing_core::Modifiers as M;
    let families = |chord: Chord| {
        [
            (M::CONTROL, M::LEFT_CONTROL, M::RIGHT_CONTROL),
            (M::ALT, M::LEFT_ALT, M::RIGHT_ALT),
            (M::SHIFT, M::LEFT_SHIFT, M::RIGHT_SHIFT),
        ]
        .map(|(generic, left, right)| {
            chord.modifiers.contains(generic)
                || chord.modifiers.contains(left)
                || chord.modifiers.contains(right)
        })
    };
    input.key == global.key && families(input) == families(global)
}

impl Profile {
    pub fn parse(text: &str, global_hotkey: Option<Chord>) -> Result<Self, String> {
        let mut sections = BTreeSet::new();
        let mut section = "";
        let mut metadata = BTreeSet::new();
        let mut chords = BTreeSet::new();
        let mut name = None;
        let mut version = None;
        let mut entries = Vec::new();
        let mut errors = Vec::new();
        // Keep original line numbers while excluding disabled entries from core
        // resolution. The core still validates every entry below.
        let mut enabled_source = String::new();
        for (index, source) in text.trim_start_matches('\u{feff}').lines().enumerate() {
            let line = source.trim();
            let number = index + 1;
            let mut error = |message: String| errors.push(format!("Line {number}: {message}"));
            if line.is_empty() || line.starts_with(';') {
                enabled_source.push('\n');
                continue;
            }
            if line.starts_with('[') && line.ends_with(']') {
                enabled_source.push_str(source);
                enabled_source.push('\n');
                section = &line[1..line.len() - 1];
                if !["profile", "modekeys", "hotkeys"].contains(&section) {
                    error(format!("unsupported section [{section}]"));
                }
                if !sections.insert(section.to_owned()) {
                    error(format!("repeated section [{section}]"));
                }
                continue;
            }
            let Some((input, action)) = line.split_once('=') else {
                error("expected key=value".into());
                continue;
            };
            let (input, action) = (input.trim(), action.trim());
            let enabled = !input.starts_with('!');
            if enabled || section == "profile" {
                enabled_source.push_str(source);
            }
            enabled_source.push('\n');
            match section {
                "profile" => {
                    if !metadata.insert(input.to_owned()) {
                        error(format!("repeated metadata {input}"));
                    }
                    match input {
                        "name" if !action.is_empty() => name = Some(action.to_owned()),
                        "version" => version = Some(action.to_owned()),
                        _ => error(format!("unknown or empty metadata {input}")),
                    }
                }
                "modekeys" | "hotkeys" => {
                    match validate_binding(input.strip_prefix('!').unwrap_or(input), action) {
                        Ok((chord, command)) => {
                            if !chords.insert((section.to_owned(), chord)) {
                                error(format!("duplicate trigger {chord} in [{section}]"));
                            }
                            if section == "hotkeys" && command == Command::ModeKey {
                                error("mode declarations belong in [modekeys]".into());
                            }
                            entries.push(Entry {
                                line: number,
                                enabled,
                                section: section.into(),
                                input: input.strip_prefix('!').unwrap_or(input).into(),
                                action: action.into(),
                                chord,
                                command,
                            });
                        }
                        Err(message) => error(message),
                    }
                }
                _ => error("entry outside a supported section".into()),
            }
        }
        for required in ["profile", "modekeys", "hotkeys"] {
            if !sections.contains(required) {
                errors.push(format!("Missing [{required}] section"));
            }
        }
        if name.is_none() {
            errors.push("Missing profile name".into());
        }
        if !matches!(version.as_deref(), Some("1" | "2")) {
            errors.push(
                "Expected profile version=1 or version=2; unsupported versions are not rewritten"
                    .into(),
            );
        }
        if version.as_deref() == Some("1") {
            for entry in entries.iter().filter(|entry| !entry.enabled) {
                errors.push(format!(
                    "Line {}: disabled mappings require profile version=2",
                    entry.line
                ));
            }
        }
        if !errors.is_empty() {
            return Err(errors.join("\r\n"));
        }
        let config = Config::parse_explicit(&enabled_source).map_err(diagnostics)?;
        let mut warnings = Vec::new();
        for entry in &entries {
            if !entry.enabled {
                continue;
            }
            if entry.section == "modekeys"
                && config.mode_keys().is_empty()
                && entry.command != Command::ModeKey
            {
                warnings.push(format!("Line {}: command mode has no mode key", entry.line));
            }
            if global_hotkey.is_some_and(|global| global_collision(entry.chord, global)) {
                warnings.push(format!(
                    "Line {}: conflicts with the global toggle",
                    entry.line
                ));
            }
            if entries.iter().any(|other| {
                other.enabled && other.section != entry.section && other.chord == entry.chord
            }) {
                let effective = config.binding(entry.chord).unwrap();
                warnings.push(format!(
                    "Line {}: shared trigger {}; effective {:?}: {}",
                    entry.line,
                    entry.chord,
                    effective.group,
                    effective.command.label()
                ));
            }
        }
        Ok(Self {
            name: name.unwrap(),
            config,
            entries,
            warnings,
        })
    }

    pub fn import_legacy(text: &str, name: &str) -> Result<String, String> {
        let legacy = Config::parse(text).map_err(diagnostics)?;
        let mut source = format!(
            "[profile]\r\nversion=1\r\nname={}\r\n\r\n[modekeys]\r\n",
            clean_name(name)?
        );
        for line in text
            .lines()
            .filter(|line| line.trim_start().starts_with(';'))
        {
            source.push_str(line);
            source.push_str("\r\n");
        }
        for key in legacy.mode_keys() {
            source.push_str(&format!("{key}=mode\r\n"));
        }
        for (chord, binding) in legacy
            .bindings()
            .filter(|(_, b)| b.group == BindingGroup::CommandMode && b.command != Command::ModeKey)
        {
            source.push_str(&format!(
                "{chord}={}\r\n",
                binding.command.configuration_value()
            ));
        }
        source.push_str("\r\n[hotkeys]\r\n");
        for (chord, binding) in legacy
            .bindings()
            .filter(|(_, b)| b.group == BindingGroup::Always && b.command != Command::ModeKey)
        {
            source.push_str(&format!(
                "{chord}={}\r\n",
                binding.command.configuration_value()
            ));
        }
        let imported = Self::parse(&source, None)?;
        let equivalent = legacy.same_bindings(&imported.config);
        if !equivalent {
            let chords = legacy
                .bindings()
                .map(|(chord, _)| chord)
                .chain(imported.config.bindings().map(|(chord, _)| chord))
                .collect::<BTreeSet<_>>();
            let describe = |binding: Option<&typing_core::Binding>| {
                binding
                    .map(|binding| {
                        let scope = match binding.group {
                            BindingGroup::CommandMode => "command mode",
                            BindingGroup::Always => "both enabled modes",
                        };
                        format!("{scope}: {}", binding.command.label())
                    })
                    .unwrap_or_else(|| "no mapping".into())
            };
            let differences = chords
                .into_iter()
                .filter(|chord| legacy.binding(*chord) != imported.config.binding(*chord))
                .map(|chord| {
                    format!(
                        "{chord}: legacy [{}]; profile [{}]",
                        describe(legacy.binding(chord)),
                        describe(imported.config.binding(chord))
                    )
                })
                .collect::<Vec<_>>()
                .join("\r\n");
            return Err(format!(
                "Legacy mappings cannot be represented equivalently:\r\n{differences}\r\nResolve these entries in the legacy file before importing. The original file was not changed."
            ));
        }
        Ok(source)
    }
}

fn clean_name(name: &str) -> Result<&str, String> {
    if name.trim().is_empty() || name.contains(['\r', '\n']) {
        Err("Profile name must be nonempty and on one line".into())
    } else {
        Ok(name.trim())
    }
}

#[derive(Clone, Debug)]
pub struct Document {
    pub text: String,
    pub path: Option<PathBuf>,
    pub baseline: String,
    pub fingerprint: Option<Vec<u8>>,
}

impl Document {
    pub fn new(name: &str) -> Self {
        Self::untitled(format!(
            "[profile]\r\nversion=1\r\nname={name}\r\n\r\n[modekeys]\r\n\r\n[hotkeys]\r\n"
        ))
    }
    pub fn untitled(text: String) -> Self {
        Self {
            baseline: text.clone(),
            text,
            path: None,
            fingerprint: None,
        }
    }
    pub fn open(path: &Path, hotkey: Option<Chord>) -> Result<Self, String> {
        let path = std::path::absolute(path).map_err(|e| e.to_string())?;
        let bytes = fs::read(&path).map_err(|e| format!("Read {}: {e}", path.display()))?;
        let text = String::from_utf8(bytes.clone())
            .map_err(|e| format!("Read {}: {e}", path.display()))?;
        Profile::parse(&text, hotkey)
            .map_err(|e| format!("Validate {}:\r\n{e}", path.display()))?;
        Ok(Self {
            baseline: text.clone(),
            text,
            path: Some(path),
            fingerprint: Some(bytes),
        })
    }
    pub fn dirty(&self) -> bool {
        self.text != self.baseline
    }
    pub fn discard(&mut self) {
        self.text = self.baseline.clone();
    }
    pub fn rename(&mut self, name: &str) -> Result<(), String> {
        let name = clean_name(name)?;
        let mut section = "";
        let line = self
            .text
            .trim_start_matches('\u{feff}')
            .lines()
            .position(|line| {
                let value = line.trim();
                if value.starts_with('[') && value.ends_with(']') {
                    section = &value[1..value.len() - 1];
                }
                section == "profile"
                    && value
                        .split_once('=')
                        .is_some_and(|(key, _)| key.trim() == "name")
            })
            .ok_or("Missing profile name")?;
        self.replace_line(line + 1, Some(&format!("name={name}")));
        Ok(())
    }
    fn replace_line(&mut self, number: usize, value: Option<&str>) {
        let mut offset = 0;
        for (index, line) in self.text.split_inclusive('\n').enumerate() {
            if index + 1 == number {
                let ending = if line.ends_with("\r\n") {
                    "\r\n"
                } else if line.ends_with('\n') {
                    "\n"
                } else {
                    ""
                };
                let replacement = value.map(|v| format!("{v}{ending}")).unwrap_or_default();
                self.text
                    .replace_range(offset..offset + line.len(), &replacement);
                return;
            }
            offset += line.len();
        }
    }
    pub fn delete(&mut self, number: usize) {
        self.replace_line(number, None);
    }
    pub fn set_enabled(
        &mut self,
        number: usize,
        enabled: bool,
        hotkey: Option<Chord>,
    ) -> Result<(), String> {
        let profile = Profile::parse(&self.text, hotkey)?;
        let entry = profile
            .entries
            .iter()
            .find(|e| e.line == number)
            .ok_or("Select a mapping to change its enabled state")?;
        if entry.enabled == enabled {
            return Ok(());
        }
        let mut candidate = self.clone();
        let source = candidate.text.lines().nth(number - 1).unwrap();
        let mut replacement = source.to_owned();
        let start = source.len() - source.trim_start().len();
        if enabled {
            replacement.remove(start);
        } else {
            replacement.insert(start, '!');
        }
        candidate.replace_line(number, Some(&replacement));
        if !enabled {
            let mut section = "";
            let version_line = candidate
                .text
                .trim_start_matches('\u{feff}')
                .lines()
                .position(|line| {
                    let line = line.trim();
                    if line.starts_with('[') && line.ends_with(']') {
                        section = &line[1..line.len() - 1];
                    }
                    section == "profile"
                        && line.split_once('=').is_some_and(|(key, value)| {
                            key.trim() == "version" && value.trim() == "1"
                        })
                });
            if let Some(line) = version_line {
                candidate.replace_line(line + 1, Some("version=2"));
            }
        }
        Profile::parse(&candidate.text, hotkey)?;
        self.text = candidate.text;
        Ok(())
    }
    pub fn edit(
        &mut self,
        old_line: Option<usize>,
        section: &str,
        input: &str,
        action: &str,
        hotkey: Option<Chord>,
    ) -> Result<usize, String> {
        if !["modekeys", "hotkeys"].contains(&section) {
            return Err("Invalid mapping type".into());
        }
        let (chord, command) = validate_binding(input.trim(), action.trim())?;
        let mut candidate = self.clone();
        let old = Profile::parse(&candidate.text, hotkey)?
            .entries
            .into_iter()
            .find(|e| Some(e.line) == old_line);
        let prefix = if old.as_ref().is_none_or(|e| e.enabled) {
            ""
        } else {
            "!"
        };
        let entry = format!("{prefix}{chord}={}", command.configuration_value());
        if old.as_ref().map(|e| e.section.as_str()) == Some(section) {
            candidate.replace_line(old_line.unwrap(), Some(&entry));
        } else {
            if let Some(line) = old_line {
                candidate.delete(line);
            }
            let marker = format!("[{section}]");
            let mut offset = 0;
            let mut insert = None;
            for line in candidate.text.split_inclusive('\n') {
                offset += line.len();
                if line.trim() == marker {
                    insert = Some(offset);
                    break;
                }
            }
            let offset = insert.ok_or("Missing mapping section")?;
            let eol = if candidate.text.contains("\r\n") {
                "\r\n"
            } else {
                "\n"
            };
            let prefix = if offset > 0 && !candidate.text[..offset].ends_with('\n') {
                eol
            } else {
                ""
            };
            candidate
                .text
                .insert_str(offset, &format!("{prefix}{entry}{eol}"));
        }
        let line = Profile::parse(&candidate.text, hotkey)?
            .entries
            .into_iter()
            .find(|entry| entry.section == section && entry.chord == chord)
            .unwrap()
            .line;
        self.text = candidate.text;
        Ok(line)
    }
    pub fn save(
        &mut self,
        path: &Path,
        overwrite: bool,
        hotkey: Option<Chord>,
    ) -> Result<(), String> {
        Profile::parse(&self.text, hotkey)?;
        let path = std::path::absolute(path).map_err(|e| e.to_string())?;
        let expected = if self
            .path
            .as_ref()
            .is_some_and(|current| same_file(current, &path))
        {
            self.fingerprint.as_deref()
        } else {
            None
        };
        guarded_write(&path, self.text.as_bytes(), expected, overwrite)?;
        self.path = Some(path);
        self.baseline = self.text.clone();
        self.fingerprint = Some(self.text.as_bytes().to_vec());
        Ok(())
    }
}

#[derive(Clone, Debug)]
pub struct AvailableProfile {
    pub name: String,
    pub path: PathBuf,
}

#[derive(Default)]
pub struct Catalog {
    pub profiles: Vec<AvailableProfile>,
    pub diagnostics: Vec<String>,
}

/// A single, nonrecursive profile directory. External Save As destinations never
/// change this directory or become catalog entries.
pub struct Library {
    pub directory: PathBuf,
}

pub fn same_file(left: &Path, right: &Path) -> bool {
    let normalize = |path: &Path| {
        path.canonicalize()
            .or_else(|_| std::path::absolute(path))
            .unwrap_or_else(|_| path.to_path_buf())
    };
    let (left, right) = (normalize(left), normalize(right));
    #[cfg(windows)]
    {
        left.as_os_str().to_string_lossy().to_lowercase()
            == right.as_os_str().to_string_lossy().to_lowercase()
    }
    #[cfg(not(windows))]
    {
        left == right
    }
}

impl Library {
    pub fn scan(&self, hotkey: Option<Chord>) -> Catalog {
        let mut catalog = Catalog::default();
        let files = match fs::read_dir(&self.directory) {
            Ok(files) => files,
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => return catalog,
            Err(error) => {
                catalog
                    .diagnostics
                    .push(format!("Discover profiles: {error}"));
                return catalog;
            }
        };
        for file in files {
            let result = (|| {
                let file = file.map_err(|e| format!("Discover profile: {e}"))?;
                let path = file.path();
                if !file.file_type().map_err(|e| e.to_string())?.is_file()
                    || !path
                        .extension()
                        .is_some_and(|ext| ext.eq_ignore_ascii_case("ini"))
                {
                    return Ok(None);
                }
                let name = file.file_name().to_string_lossy().into_owned();
                let source = fs::read_to_string(&path).map_err(|e| format!("Read {name}: {e}"))?;
                let profile =
                    Profile::parse(&source, hotkey).map_err(|e| format!("{name}: {e}"))?;
                Ok(Some(AvailableProfile {
                    name: profile.name,
                    path,
                }))
            })();
            match result {
                Ok(Some(profile)) => catalog.profiles.push(profile),
                Ok(None) => {}
                Err(error) => catalog.diagnostics.push(error),
            }
        }
        catalog
            .profiles
            .sort_by_key(|p| (p.name.to_lowercase(), p.path.clone()));
        for pair in catalog.profiles.windows(2) {
            if pair[0].name.to_lowercase() == pair[1].name.to_lowercase() {
                catalog.diagnostics.push(format!(
                    "Duplicate profile name: {}. Rename a profile to distinguish them.",
                    pair[0].name
                ));
            }
        }
        catalog
    }

    pub fn check_name(
        &self,
        name: &str,
        except: Option<&Path>,
        hotkey: Option<Chord>,
    ) -> Result<String, String> {
        let name = clean_name(name)?;
        let catalog = self.scan(hotkey);
        if catalog.profiles.iter().any(|p| {
            p.name.to_lowercase() == name.to_lowercase()
                && !except.is_some_and(|path| same_file(path, &p.path))
        }) {
            return Err(format!(
                "A profile named {name} already exists. Choose a different name."
            ));
        }
        Ok(name.into())
    }

    pub fn contains(&self, path: &Path) -> bool {
        path.parent()
            .is_some_and(|parent| same_file(parent, &self.directory))
    }

    pub fn destination(&self, name: &str, hotkey: Option<Chord>) -> Result<PathBuf, String> {
        self.check_name(name, None, hotkey)?;
        // A prefix avoids reserved Windows device names. Display names stay
        // independent of filenames, including after a Rename.
        let stem: String = name
            .chars()
            .take(64)
            .map(|c| {
                if c.is_control() || "<>:\"/\\|?*".contains(c) {
                    '-'
                } else {
                    c
                }
            })
            .collect();
        fs::create_dir_all(&self.directory)
            .map_err(|e| format!("Create profiles directory: {e}"))?;
        for suffix in 0..10000 {
            let name = if suffix == 0 {
                format!("profile-{stem}.typing.ini")
            } else {
                format!("profile-{stem}-{suffix}.typing.ini")
            };
            let path = self.directory.join(name);
            if !path.try_exists().map_err(|e| e.to_string())? {
                return Ok(path);
            }
        }
        Err("Cannot choose an unused profile filename".into())
    }

    pub fn save(
        &self,
        document: &mut Document,
        path: &Path,
        overwrite: bool,
        hotkey: Option<Chord>,
    ) -> Result<(), String> {
        if self.contains(path) {
            self.check_name(
                &Profile::parse(&document.text, hotkey)?.name,
                Some(path),
                hotkey,
            )?;
        }
        document.save(path, overwrite, hotkey)
    }

    pub fn remove(&self, document: &Document, active: Option<&Path>) -> Result<(), String> {
        let path = document
            .path
            .as_deref()
            .ok_or("Save the profile before deleting it")?;
        if !self.contains(path) {
            return Err("Only profiles in the default directory can be deleted".into());
        }
        if !path
            .extension()
            .is_some_and(|ext| ext.eq_ignore_ascii_case("ini"))
            || !fs::symlink_metadata(path)
                .map_err(|e| e.to_string())?
                .file_type()
                .is_file()
        {
            return Err("Only available profile files can be deleted".into());
        }
        if active.is_some_and(|active| same_file(active, path)) {
            return Err("Switch to a different profile before deleting the active profile".into());
        }
        if document.dirty() {
            return Err("Resolve unsaved changes before deleting the profile".into());
        }
        let current = fs::read(path).map_err(|e| format!("Read profile before deletion: {e}"))?;
        if Some(current.as_slice()) != document.fingerprint.as_deref() {
            return Err("External change: reload the profile before deleting it".into());
        }
        fs::remove_file(path).map_err(|e| format!("Delete profile: {e}"))
    }
}

pub fn import_document(
    source: &str,
    profile_file: bool,
    name: &str,
    hotkey: Option<Chord>,
) -> Result<Document, String> {
    let mut document = if profile_file
        || source
            .trim_start_matches('\u{feff}')
            .lines()
            .any(|l| l.trim().eq_ignore_ascii_case("[profile]"))
    {
        Profile::parse(source, hotkey)?;
        Document::untitled(source.into())
    } else {
        Document::untitled(Profile::import_legacy(source, name)?)
    };
    document.rename(name)?;
    document.baseline = document.text.clone();
    Ok(document)
}

static TEMP_ID: AtomicU64 = AtomicU64::new(0);

/// The full read baseline is the fingerprint: no hash collision can hide a change.
pub fn guarded_write(
    path: &Path,
    bytes: &[u8],
    expected: Option<&[u8]>,
    overwrite: bool,
) -> Result<(), String> {
    let read = || match fs::read(path) {
        Ok(bytes) => Ok(Some(bytes)),
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(None),
        Err(e) => Err(format!("Read {}: {e}", path.display())),
    };
    let original = read()?;
    if !overwrite && original.as_deref() != expected {
        return Err(format!(
            "External change: {}. Reload, Save As, or explicitly overwrite.",
            path.display()
        ));
    }
    if original.as_deref() == Some(bytes) {
        return Ok(());
    }
    if path.metadata().is_ok_and(|m| m.permissions().readonly()) {
        return Err(format!("Save {}: destination is read-only", path.display()));
    }
    let temp = path.with_file_name(format!(
        ".typing-{}-{}.tmp",
        std::process::id(),
        TEMP_ID.fetch_add(1, Ordering::Relaxed)
    ));
    let result = (|| {
        let mut file = fs::OpenOptions::new()
            .write(true)
            .create_new(true)
            .open(&temp)
            .map_err(|e| format!("Prepare {}: {e}", temp.display()))?;
        file.write_all(bytes)
            .and_then(|()| file.sync_all())
            .map_err(|e| format!("Prepare {}: {e}", temp.display()))?;
        drop(file);
        if read()? != original {
            return Err("External change while preparing save; reload or retry with an explicit overwrite decision".into());
        }
        if let Some(old) = original {
            let mut backup_name = path.as_os_str().to_os_string();
            backup_name.push(".bak");
            let backup = PathBuf::from(backup_name);
            let mut file = fs::File::create(&backup)
                .map_err(|e| format!("Backup {}: {e}", backup.display()))?;
            file.write_all(&old)
                .and_then(|()| file.sync_all())
                .map_err(|e| format!("Backup {}: {e}", backup.display()))?;
        }
        fs::rename(&temp, path).map_err(|e| format!("Replace {}: {e}", path.display()))
    })();
    if result.is_err() {
        let _ = fs::remove_file(&temp);
    }
    result
}

#[derive(Clone, Debug)]
pub struct Application {
    pub path: PathBuf,
    pub source: String,
    pub settings: Config,
    pub selected: Option<PathBuf>,
}

impl Application {
    pub fn read(path: &Path) -> Result<Self, String> {
        let path = std::path::absolute(path).map_err(|e| e.to_string())?;
        let source = fs::read_to_string(&path)
            .map_err(|e| format!("Read application configuration {}: {e}", path.display()))?;
        Self::parse(path, source)
    }
    pub fn parse(path: PathBuf, source: String) -> Result<Self, String> {
        let mut section = "";
        let mut filtered = String::new();
        let mut active = None;
        for line in source.trim_start_matches('\u{feff}').lines() {
            let value = line.trim();
            if value.starts_with('[') && value.ends_with(']') {
                section = &value[1..value.len() - 1];
            }
            if section == "profiles"
                && let Some((key, value)) = value.split_once('=')
                && key.trim() == "active"
            {
                active = if value.trim().is_empty() {
                    None
                } else {
                    Some(PathBuf::from(value.trim()))
                };
            }
            if section == "common" {
                filtered.push_str(line);
            }
            filtered.push('\n');
        }
        let settings = Config::parse_explicit(&filtered).map_err(diagnostics)?;
        let selected = active.map(|p| {
            if p.is_absolute() {
                p
            } else {
                path.parent().unwrap().join(p)
            }
        });
        Ok(Self {
            path,
            source,
            settings,
            selected,
        })
    }
    pub fn remember(&mut self, profile: &Path) -> Result<(), String> {
        let base = self.path.parent().unwrap();
        let stored = profile
            .strip_prefix(base)
            .unwrap_or(profile)
            .to_string_lossy();
        let mut output = String::new();
        let mut section = "";
        let mut found = false;
        let mut inserted = false;
        for line in self.source.split_inclusive('\n') {
            let value = line.trim();
            if value.starts_with('[') && value.ends_with(']') {
                if section == "profiles" && !inserted {
                    output.push_str(&format!("active={stored}\r\n"));
                    inserted = true;
                }
                section = &value[1..value.len() - 1];
                if section == "profiles" {
                    found = true;
                }
            }
            if section == "profiles"
                && value
                    .split_once('=')
                    .is_some_and(|(key, _)| key.trim() == "active")
            {
                if !inserted {
                    output.push_str(&format!("active={stored}\r\n"));
                    inserted = true;
                }
            } else {
                output.push_str(line);
            }
        }
        if !inserted {
            if !output.ends_with('\n') {
                output.push_str("\r\n");
            }
            if !found {
                output.push_str("\r\n[profiles]\r\n");
            }
            output.push_str(&format!("active={stored}\r\n"));
        }
        guarded_write(
            &self.path,
            output.as_bytes(),
            Some(self.source.as_bytes()),
            false,
        )?;
        self.source = output;
        self.selected = Some(profile.to_path_buf());
        Ok(())
    }
}

#[derive(Clone, Debug)]
pub struct Candidate {
    pub document: Document,
    pub profile: Profile,
    pub replace_editor: bool,
}

pub struct Activation {
    pub application: Application,
    pub active: Option<Candidate>,
    pub pending: Option<Candidate>,
    pub completed: Option<Candidate>,
    pub valid: bool,
    pub error: Option<String>,
}

impl Activation {
    pub fn is_current(&self, document: &Document) -> bool {
        document.path.is_some()
            && self.active.as_ref().is_some_and(|active| {
                same_file(
                    active.document.path.as_ref().unwrap(),
                    document.path.as_ref().unwrap(),
                )
            })
    }

    pub fn is_active_revision(&self, document: &Document) -> bool {
        self.is_current(document)
            && self
                .active
                .as_ref()
                .is_some_and(|active| active.document.text == document.text)
    }

    pub fn load(application: Application) -> Result<(Self, Config), String> {
        let mut this = Self {
            application,
            active: None,
            pending: None,
            completed: None,
            valid: false,
            error: None,
        };
        let config = if let Some(path) = this.application.selected.as_ref() {
            match Document::open(path, this.application.settings.global_hotkey) {
                Ok(document) => {
                    let profile =
                        Profile::parse(&document.text, this.application.settings.global_hotkey)?;
                    let config = this.application.settings.with_bindings(&profile.config);
                    this.active = Some(Candidate {
                        document,
                        profile,
                        replace_editor: true,
                    });
                    this.valid = true;
                    config
                }
                Err(error) => {
                    this.error = Some(error);
                    this.application.settings.clone()
                }
            }
        } else {
            this.valid = true;
            Config::parse(&this.application.source).map_err(diagnostics)?
        };
        Ok((this, config))
    }
    pub fn prepare(&mut self, document: Document, replace_editor: bool) -> Result<(), String> {
        if self.pending.is_some() || self.completed.is_some() {
            return Err("A profile operation is already in progress".into());
        }
        if document.path.is_none() || document.dirty() {
            return Err("Save the profile before activating it".into());
        }
        let profile = Profile::parse(&document.text, self.application.settings.global_hotkey)?;
        self.pending = Some(Candidate {
            document,
            profile,
            replace_editor,
        });
        self.error = None;
        Ok(())
    }
    pub fn cancel(&mut self, error: impl Into<String>) {
        if self.pending.take().is_some() {
            self.error = Some(error.into());
        }
    }
    pub fn commit(&mut self) -> Option<Config> {
        let candidate = self.pending.take()?;
        let config = self
            .application
            .settings
            .with_bindings(&candidate.profile.config);
        self.valid = true;
        self.error = None;
        self.active = Some(candidate.clone());
        self.completed = Some(candidate);
        Some(config)
    }
    pub fn name(&self) -> &str {
        self.active
            .as_ref()
            .map(|c| c.profile.name.as_str())
            .unwrap_or(if self.valid {
                "Legacy configuration"
            } else {
                "No valid profile"
            })
    }
}

#[derive(Default)]
pub struct Recorder {
    held: BTreeSet<typing_core::Key>,
    order: Vec<typing_core::Key>,
    pub chord: Option<String>,
    pub error: Option<String>,
}

impl Recorder {
    pub fn input(&mut self, key: typing_core::Key, down: bool) {
        if down {
            if self.held.is_empty() {
                self.order.clear();
                self.chord = None;
                self.error = None;
            }
            if self.held.insert(key) {
                self.order.push(key);
            }
        } else {
            self.held.remove(&key);
            if self.held.is_empty() && !self.order.is_empty() {
                let text = self
                    .order
                    .iter()
                    .map(ToString::to_string)
                    .collect::<Vec<_>>()
                    .join("+");
                // Only modifiers may precede the final key. The legacy parser
                // otherwise accepts multiple ordinary keys and drops the earlier one.
                let modifiers = [20, 160, 161, 162, 163, 164, 165];
                if self.order[..self.order.len() - 1]
                    .iter()
                    .any(|key| !modifiers.contains(&key.code()))
                {
                    self.error = Some("Only modifiers may precede the final key; this gesture cannot be represented".into());
                } else if text.parse::<Chord>().is_ok() {
                    self.chord = Some(text);
                }
            }
        }
    }
    pub fn released(&self) -> bool {
        self.held.is_empty()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::time::Duration;
    use typing_core::{Core, Direction, Key, KeyEvent};

    struct Directory(PathBuf);
    impl Directory {
        fn new() -> Self {
            let root = Path::new(env!("CARGO_MANIFEST_DIR")).join("../target/profile-tests");
            let root = std::path::absolute(root).unwrap();
            fs::create_dir_all(&root).unwrap();
            let path = root.join(format!(
                "typing-profile-tests-{}-{}",
                std::process::id(),
                TEMP_ID.fetch_add(1, Ordering::Relaxed)
            ));
            fs::create_dir(&path).unwrap();
            Self(path)
        }
    }
    impl Drop for Directory {
        fn drop(&mut self) {
            let _ = fs::remove_dir_all(&self.0);
        }
    }

    #[test]
    fn catalog_is_nonrecursive_and_external_save_as_never_changes_discovery() {
        let directory = Directory::new();
        let library = Library {
            directory: directory.0.join("profiles"),
        };
        fs::create_dir_all(library.directory.join("nested")).unwrap();
        fs::write(
            library.directory.join("one.typing.ini"),
            Document::new("One").text,
        )
        .unwrap();
        fs::write(
            library.directory.join("nested/two.typing.ini"),
            Document::new("Nested").text,
        )
        .unwrap();
        fs::write(
            library.directory.join("broken.ini"),
            "[profile]\nversion=99",
        )
        .unwrap();
        fs::write(
            library.directory.join("one.typing.ini.bak"),
            Document::new("Backup").text,
        )
        .unwrap();
        let mut external = Document::new("External");
        library
            .save(
                &mut external,
                &directory.0.join("external.typing.ini"),
                false,
                None,
            )
            .unwrap();
        let duplicate_path = library.directory.join("duplicate.typing.ini");
        assert!(
            library
                .save(&mut Document::new("ONE"), &duplicate_path, false, None)
                .is_err()
        );
        assert!(!duplicate_path.exists());
        let catalog = library.scan(None);
        assert_eq!(catalog.profiles.len(), 1);
        assert_eq!(catalog.profiles[0].name, "One");
        assert_eq!(catalog.diagnostics.len(), 1);
        assert!(!library.contains(external.path.as_ref().unwrap()));
        assert!(library.check_name("one", None, None).is_err());
        assert!(
            library
                .check_name("One", Some(&catalog.profiles[0].path), None)
                .is_ok()
        );
    }

    #[test]
    fn managed_save_names_do_not_overwrite_existing_files_or_escape_the_directory() {
        let directory = Directory::new();
        let library = Library {
            directory: directory.0.join("profiles"),
        };
        let first = library.destination("../CON:layout", None).unwrap();
        assert_eq!(first.parent(), Some(library.directory.as_path()));
        fs::write(&first, "existing unrecognized file").unwrap();
        let second = library.destination("../CON:layout", None).unwrap();
        assert_ne!(first, second);
        let mut document = Document::new("../CON:layout");
        document.save(&second, false, None).unwrap();
        assert_eq!(
            fs::read_to_string(first).unwrap(),
            "existing unrecognized file"
        );
        assert_eq!(library.scan(None).profiles[0].name, "../CON:layout");
    }

    #[test]
    fn profile_deletion_protects_active_external_dirty_and_changed_files() {
        let directory = Directory::new();
        let library = Library {
            directory: directory.0.join("profiles"),
        };
        let path = library.destination("Delete me", None).unwrap();
        let mut document = Document::new("Delete me");
        document.save(&path, false, None).unwrap();
        assert!(library.remove(&document, Some(&path)).is_err());
        document.rename("Dirty").unwrap();
        assert!(library.remove(&document, None).is_err());
        document.discard();
        fs::write(&path, Document::new("Changed externally").text).unwrap();
        assert!(library.remove(&document, None).is_err());
        document = Document::open(&path, None).unwrap();
        library.remove(&document, None).unwrap();
        assert!(!path.exists());
        document
            .save(&directory.0.join("outside.typing.ini"), false, None)
            .unwrap();
        assert!(library.remove(&document, None).is_err());
        assert!(document.path.unwrap().exists());
    }

    #[test]
    fn importing_profiles_preserves_source_semantics_and_rejects_invalid_versions() {
        let mut source = Document::new("Source");
        source
            .edit(None, "modekeys", "CapsLock", "mode", None)
            .unwrap();
        source
            .edit(None, "hotkeys", "LControlKey+J", "date", None)
            .unwrap();
        let imported = import_document(&source.text, true, "Imported", None).unwrap();
        assert!(imported.path.is_none());
        assert_eq!(
            Profile::parse(&imported.text, None).unwrap().name,
            "Imported"
        );
        assert!(
            Profile::parse(&source.text, None)
                .unwrap()
                .config
                .same_bindings(&Profile::parse(&imported.text, None).unwrap().config)
        );
        assert_eq!(Profile::parse(&source.text, None).unwrap().name, "Source");
        assert!(
            import_document(
                &source.text.replace("version=1", "version=99"),
                false,
                "Bad",
                None
            )
            .is_err()
        );
        assert!(import_document("[modekeys]\nJ=Left", true, "Bad", None).is_err());
        let legacy = "[modekeys]\nCapsLock=mode\nJ=Left\n[hotkeys]\nJ=Left\n";
        let imported = import_document(legacy, false, "Legacy copy", None).unwrap();
        assert!(
            Config::parse(legacy)
                .unwrap()
                .same_bindings(&Profile::parse(&imported.text, None).unwrap().config)
        );
    }

    #[test]
    fn disabled_entries_are_validated_and_do_not_affect_resolution_or_warnings() {
        let source = "[profile]\nversion=2\nname=States\n[modekeys]\n!CapsLock=mode\n!J=date\nK=Down\n[hotkeys]\nJ=Left\n!Control+U=disable\n";
        let profile = Profile::parse(source, Some("Control+U".parse().unwrap())).unwrap();
        assert_eq!(profile.entries.len(), 5);
        assert_eq!(profile.entries.iter().filter(|e| e.enabled).count(), 2);
        assert!(profile.config.mode_keys().is_empty());
        let binding = profile.config.binding("J".parse().unwrap()).unwrap();
        assert_eq!(binding.group, BindingGroup::Always);
        assert_eq!(binding.command, Command::Movement("Left".parse().unwrap()));
        assert_eq!(profile.warnings, ["Line 7: command mode has no mode key"]);
        assert!(
            profile
                .config
                .binding("Control+U".parse().unwrap())
                .is_none()
        );
        for (from, to) in [
            ("!J=date", "!J=not-a-command"),
            ("!J=date", "!J+K=date"),
            ("!J=date", "J=Left\n!74=date"),
        ] {
            assert!(Profile::parse(&source.replace(from, to), None).is_err());
        }
        assert!(Profile::parse(&source.replace("version=2", "version=1"), None).is_err());
        // Excluding a hotkey must also remove its eligibility override while
        // preserving the enabled command-mode action.
        let profile = Profile::parse(
            &source
                .replace("!J=date", "J=date")
                .replace("\nJ=Left", "\n!J=Left"),
            None,
        )
        .unwrap();
        let binding = profile.config.binding("J".parse().unwrap()).unwrap();
        assert_eq!(binding.group, BindingGroup::CommandMode);
        assert_eq!(binding.command, Command::Date);
    }

    #[test]
    fn disabled_mappings_pass_input_and_disabled_mode_keys_cannot_enter_mode() {
        let source = "[profile]\nversion=2\nname=Disabled\n[modekeys]\n!CapsLock=mode\n!J=Left\n[hotkeys]\n!K=date\n";
        let profile = Profile::parse(source, None).unwrap();
        assert_eq!(profile.config.bindings().count(), 0);
        assert!(profile.config.mode_keys().is_empty());
        let mut core = Core::new(profile.config);
        for key in [Key::CAPS_LOCK, "J".parse().unwrap(), "K".parse().unwrap()] {
            for direction in [Direction::Down, Direction::Up] {
                let decision = core.input(KeyEvent { key, direction }, Duration::ZERO);
                assert!(!decision.consume);
                assert!(decision.request.is_none());
            }
        }
        assert!(!core.command_mode());
        let mut core = Core::new(
            Profile::parse(&source.replace("!CapsLock", "CapsLock"), None)
                .unwrap()
                .config,
        );
        for direction in [Direction::Down, Direction::Up] {
            core.input(
                KeyEvent {
                    key: Key::CAPS_LOCK,
                    direction,
                },
                Duration::ZERO,
            );
        }
        assert!(core.command_mode());
        for key in ["J", "K"] {
            let decision = core.input(
                KeyEvent {
                    key: key.parse().unwrap(),
                    direction: Direction::Down,
                },
                Duration::ZERO,
            );
            assert!(!decision.consume);
            assert!(decision.request.is_none());
        }
    }

    #[test]
    fn toggles_preserve_source_and_edits_preserve_disabled_state_across_types() {
        let source = "\u{feff}[profile]\r\nversion=1\r\nname=Source\r\n[modekeys]\r\n; keep this comment\r\n  CapsLock = mode  \r\nJ = Left\r\n[hotkeys]\r\nK=Down";
        let mut document = Document::untitled(source.into());
        assert!(
            Profile::parse(source, None)
                .unwrap()
                .entries
                .iter()
                .all(|e| e.enabled)
        );
        document.set_enabled(6, false, None).unwrap();
        assert_eq!(
            document.text,
            source
                .replace("version=1", "version=2")
                .replace("  CapsLock", "  !CapsLock")
        );
        assert!(document.dirty());
        document.set_enabled(6, true, None).unwrap();
        assert_eq!(document.text, source.replace("version=1", "version=2"));
        document.set_enabled(9, false, None).unwrap();
        assert!(document.text.ends_with("!K=Down"));
        let before = document.text.clone();
        assert!(document.set_enabled(5, false, None).is_err());
        assert_eq!(document.text, before);
        let line = document
            .edit(Some(9), "modekeys", "K", "date", None)
            .unwrap();
        let profile = Profile::parse(&document.text, None).unwrap();
        let entry = profile.entries.iter().find(|e| e.line == line).unwrap();
        assert!(!entry.enabled);
        assert_eq!(entry.command, Command::Date);
        document.set_enabled(line, true, None).unwrap();
        assert!(
            Profile::parse(&document.text, None)
                .unwrap()
                .config
                .binding("K".parse().unwrap())
                .is_some()
        );
        document.discard();
        assert_eq!(document.text, source);
    }

    #[test]
    fn mapping_states_survive_import_save_as_restart_and_explicit_activation() {
        let directory = Directory::new();
        let path = directory.0.join("current.typing.ini");
        let mut document = Document::new("States");
        let line = document.edit(None, "hotkeys", "J", "Left", None).unwrap();
        document.save(&path, false, None).unwrap();
        let app = Application::parse(
            directory.0.join("configuration.ini"),
            "[common]\nhotkey=Control+U\n[profiles]\nactive=current.typing.ini\n".into(),
        )
        .unwrap();
        let (mut activation, original) = Activation::load(app.clone()).unwrap();
        assert!(original.binding("J".parse().unwrap()).is_some());
        document.set_enabled(line, false, None).unwrap();
        assert!(activation.prepare(document.clone(), false).is_err());
        document.save(&path, false, None).unwrap();
        assert!(!activation.is_active_revision(&document));
        assert!(
            activation
                .active
                .as_ref()
                .unwrap()
                .profile
                .config
                .binding("J".parse().unwrap())
                .is_some()
        );
        let reopened = Document::open(&path, None).unwrap();
        let copied = import_document(&reopened.text, true, "Copy", None).unwrap();
        let mut imported = copied.clone();
        imported
            .save(&directory.0.join("outside.typing.ini"), false, None)
            .unwrap();
        assert!(
            !Profile::parse(
                &Document::open(imported.path.as_ref().unwrap(), None)
                    .unwrap()
                    .text,
                None
            )
            .unwrap()
            .entries[0]
                .enabled
        );
        let (_, restarted) = Activation::load(app).unwrap();
        assert!(restarted.binding("J".parse().unwrap()).is_none());
        activation.prepare(reopened.clone(), false).unwrap();
        activation.cancel("cleanup failed");
        assert!(
            activation
                .active
                .as_ref()
                .unwrap()
                .profile
                .config
                .binding("J".parse().unwrap())
                .is_some()
        );
        activation.prepare(reopened, false).unwrap();
        let config = activation.commit().unwrap();
        assert!(config.binding("J".parse().unwrap()).is_none());
        assert_eq!(config.global_hotkey, original.global_hotkey);
    }

    #[test]
    fn empty_profile_is_complete_and_composition_preserves_application_settings() {
        let empty = Profile::parse(&Document::new("Empty").text, None).unwrap();
        assert!(empty.config.mode_keys().is_empty());
        assert_eq!(empty.config.bindings().count(), 0);
        let app = Config::parse_explicit("[common]\nmode_auto_exit=off\nmode_auto_exit_time=17\nhotkey=Control+U\nmode_mouse_tip=off\n").unwrap();
        let composed = app.with_bindings(&empty.config);
        assert!(!composed.auto_exit);
        assert_eq!(composed.auto_exit_seconds, 17);
        assert_eq!(composed.global_hotkey, app.global_hotkey);
        assert_eq!(composed.common, app.common);
        assert!(composed.mode_keys().is_empty());
    }
    #[test]
    fn format_rejects_unknown_data_duplicates_and_unrepresentable_chords() {
        let base = Document::new("Test").text;
        for extra in [
            "[common]\nhotkey=Control+Q",
            "[profile]\nname=Other",
            "[extra]",
            "J=Left\n74=Right",
            "J+L=Left",
            "CapsLock=mode",
        ] {
            assert!(
                Profile::parse(&format!("{base}{extra}\n"), None).is_err(),
                "{extra}"
            );
        }
        assert!(Profile::parse(&base.replace("version=1", "version=99"), None).is_err());
        assert!(Profile::parse(&base.replace("name=Test", "unknown=abc"), None).is_err());
    }
    #[test]
    fn core_resolution_drives_cross_section_diagnostics() {
        let source = "[profile]\nversion=1\nname=Collisions\n[modekeys]\nCapsLock=mode\nJ=date\n[hotkeys]\nJ=Left\n";
        let profile = Profile::parse(source, Some("J".parse().unwrap())).unwrap();
        let binding = profile.config.binding("J".parse().unwrap()).unwrap();
        assert_eq!(binding.group, BindingGroup::Always);
        assert_eq!(binding.command, Command::Date);
        assert!(
            profile
                .warnings
                .iter()
                .any(|w| w.contains("effective Always: date"))
        );
        assert!(profile.warnings.iter().any(|w| w.contains("global toggle")));
    }
    #[test]
    fn editing_keeps_comments_and_unaffected_entry_bytes_and_filters_cannot_drop_entries() {
        let source = "\u{feff}[profile]\r\nversion=1\r\nname=Old\r\n\r\n[modekeys]\r\n; movement\r\nJ = Left\r\nK=Down\r\n\r\n[hotkeys]\r\n; copy\r\nLMenu+C = Control+C\r\n";
        let mut document = Document::untitled(source.into());
        document.rename("New").unwrap();
        document
            .edit(Some(7), "modekeys", "J", "Right", None)
            .unwrap();
        assert!(
            document
                .text
                .contains("; movement\r\nJ=Right\r\nK=Down\r\n")
        );
        assert!(document.text.contains("; copy\r\nLMenu+C = Control+C\r\n"));
        assert!(document.dirty());
        document.discard();
        assert_eq!(document.text, source);
        let snapshot = document.text.clone();
        assert!(document.edit(None, "modekeys", "J", "Up", None).is_err());
        assert_eq!(document.text, snapshot);
    }
    #[test]
    fn save_backup_conflict_and_save_as_keep_editor_and_active_snapshots_separate() {
        let directory = Directory::new();
        let path = directory.0.join("layout.typing.ini");
        let mut document = Document::new("Saved");
        document.save(&path, false, None).unwrap();
        let active = document.clone();
        document.edit(None, "modekeys", "J", "Left", None).unwrap();
        document.save(&path, false, None).unwrap();
        assert!(!document.dirty());
        assert_ne!(document.text, active.text);
        assert_eq!(
            fs::read_to_string(path.with_file_name("layout.typing.ini.bak")).unwrap(),
            active.text
        );
        fs::write(&path, "external").unwrap();
        document.rename("Edited").unwrap();
        assert!(
            document
                .save(&path, false, None)
                .unwrap_err()
                .starts_with("External change")
        );
        assert!(document.dirty());
        assert_eq!(fs::read_to_string(&path).unwrap(), "external");
        let other = directory.0.join("other.typing.ini");
        document.save(&other, false, None).unwrap();
        assert_eq!(document.path.as_ref(), Some(&other));
        assert_eq!(active.path.as_ref(), Some(&path));
    }
    #[test]
    fn failed_backup_preserves_destination_and_unsaved_work() {
        let directory = Directory::new();
        let path = directory.0.join("layout.typing.ini");
        let mut document = Document::new("Saved");
        document.save(&path, false, None).unwrap();
        let original = fs::read(&path).unwrap();
        fs::create_dir(path.with_file_name("layout.typing.ini.bak")).unwrap();
        document.rename("Changed").unwrap();
        assert!(
            document
                .save(&path, false, None)
                .unwrap_err()
                .starts_with("Backup")
        );
        assert_eq!(fs::read(&path).unwrap(), original);
        assert!(document.dirty());
    }
    #[test]
    fn read_only_destination_preserves_unsaved_work() {
        let directory = Directory::new();
        let path = directory.0.join("readonly.typing.ini");
        let mut document = Document::new("Original");
        document.save(&path, false, None).unwrap();
        let original = fs::metadata(&path).unwrap().permissions();
        let mut readonly = original.clone();
        readonly.set_readonly(true);
        fs::set_permissions(&path, readonly).unwrap();
        document.rename("Changed").unwrap();
        let result = document.save(&path, false, None);
        fs::set_permissions(&path, original).unwrap();
        assert!(result.unwrap_err().contains("read-only"));
        assert!(document.dirty());
        assert!(fs::read_to_string(&path).unwrap().contains("name=Original"));
    }
    #[test]
    fn application_selection_ignores_legacy_errors_and_remember_preserves_other_text() {
        let directory = Directory::new();
        let path = directory.0.join("configuration.ini");
        let source = "; keep me\r\n[common]\r\nhotkey=Control+U\r\n[modekeys]\r\ninvalid legacy line\r\n[profiles]\r\nactive=missing.typing.ini\r\n";
        fs::write(&path, source).unwrap();
        let mut app = Application::read(&path).unwrap();
        assert_eq!(app.selected, Some(directory.0.join("missing.typing.ini")));
        let (state, config) = Activation::load(app.clone()).unwrap();
        assert!(!state.valid);
        assert!(state.error.is_some());
        assert!(config.mode_keys().is_empty());
        app.remember(&directory.0.join("new.typing.ini")).unwrap();
        let saved = fs::read_to_string(&path).unwrap();
        assert!(saved.contains("active=new.typing.ini"));
        assert!(saved.starts_with("; keep me\r\n"));
        assert!(saved.contains("invalid legacy line\r\n"));
        fs::write(&path, "external").unwrap();
        assert!(app.remember(&directory.0.join("other.typing.ini")).is_err());
        assert_eq!(fs::read_to_string(&path).unwrap(), "external");
    }
    #[test]
    fn activation_failure_retains_old_selection_and_success_composes_saved_snapshot() {
        let directory = Directory::new();
        let path = directory.0.join("a.typing.ini");
        let mut document = Document::new("Candidate");
        document.save(&path, false, None).unwrap();
        let app = Application::parse(
            directory.0.join("configuration.ini"),
            "[common]\nhotkey=Control+U\n[modekeys]\nCapsLock=mode\nJ=Left\n".into(),
        )
        .unwrap();
        let (mut activation, old) = Activation::load(app).unwrap();
        activation.prepare(document.clone(), true).unwrap();
        activation.cancel("cleanup failure");
        assert_eq!(activation.name(), "Legacy configuration");
        assert!(activation.valid);
        assert!(old.binding("J".parse().unwrap()).is_some());
        activation.prepare(document.clone(), false).unwrap();
        assert!(activation.prepare(document, false).is_err());
        let new = activation.commit().unwrap();
        assert_eq!(activation.name(), "Candidate");
        assert!(new.binding("J".parse().unwrap()).is_none());
        assert_eq!(new.global_hotkey, old.global_hotkey);
        assert!(activation.completed.is_some());
    }
    #[test]
    fn saving_keeps_the_active_snapshot_and_explicit_activation_can_retry() {
        let directory = Directory::new();
        let path = directory.0.join("current.typing.ini");
        let mut document = Document::new("Original");
        document.save(&path, false, None).unwrap();
        let original = document.text.clone();
        let app = Application::parse(
            directory.0.join("configuration.ini"),
            "[profiles]\nactive=current.typing.ini\n".into(),
        )
        .unwrap();
        let (mut activation, _) = Activation::load(app).unwrap();
        assert!(activation.is_active_revision(&document));
        document.rename("Updated").unwrap();
        document.save(&path, false, None).unwrap();
        assert!(!document.dirty());
        assert!(!activation.is_active_revision(&document));
        assert_eq!(activation.name(), "Original");
        activation.prepare(document.clone(), false).unwrap();
        activation.cancel("cleanup failed");
        document.save(&path, false, None).unwrap();
        assert_eq!(
            fs::read_to_string(path.with_file_name("current.typing.ini.bak")).unwrap(),
            original
        );
        assert!(!activation.is_active_revision(&document));
        activation.prepare(document.clone(), false).unwrap();
        activation.commit().unwrap();
        assert!(activation.is_active_revision(&document));
        let other = directory.0.join("other.typing.ini");
        document.save(&other, false, None).unwrap();
        assert!(!activation.is_current(&document));
        assert_eq!(activation.name(), "Updated");
    }
    #[test]
    fn installed_legacy_bindings_import_with_equivalent_semantics() {
        let source = include_str!("../../typing-core/tests/fixtures/aldetl-installed.ini");
        let imported = Profile::import_legacy(source, "Imported").unwrap();
        let profile = Profile::parse(&imported, None).unwrap();
        assert!(
            profile
                .config
                .same_bindings(&Config::parse(source).unwrap())
        );
        assert!(!imported.contains("[common]"));
        let error =
            Profile::import_legacy("[hotkeys]\nJ=Left\n", "Unrepresentable defaults").unwrap_err();
        assert!(error.contains("CapsLock: legacy [no mapping]; profile [command mode:"));
    }
    #[test]
    fn packaged_profile_is_complete_and_matches_installed_bindings() {
        let profile = Profile::parse(
            include_str!("../../resources/profiles/default.typing.ini"),
            None,
        )
        .unwrap();
        assert!(
            profile.config.same_bindings(
                &Config::parse(include_str!(
                    "../../typing-core/tests/fixtures/aldetl-installed.ini"
                ))
                .unwrap()
            )
        );
        let application = Application::parse(
            PathBuf::from("C:/Typing/configuration.ini"),
            include_str!("../../resources/configuration.ini").into(),
        )
        .unwrap();
        assert!(
            application
                .selected
                .unwrap()
                .ends_with("profiles/default.typing.ini")
        );
    }
    #[test]
    fn recorder_preserves_sides_waits_for_release_and_rejects_multi_key_sequences() {
        let mut recorder = Recorder::default();
        recorder.input(Key::LEFT_CONTROL, true);
        recorder.input(Key::LEFT, true);
        recorder.input(Key::LEFT, true);
        recorder.input(Key::LEFT, false);
        assert!(recorder.chord.is_none());
        recorder.input(Key::LEFT_CONTROL, false);
        assert_eq!(recorder.chord.as_deref(), Some("LControlKey+Left"));
        recorder.input("J".parse().unwrap(), true);
        recorder.input("K".parse().unwrap(), true);
        recorder.input("J".parse().unwrap(), false);
        recorder.input("K".parse().unwrap(), false);
        assert!(recorder.error.is_some());
    }
    #[test]
    fn editor_observation_cancels_mode_gestures_without_admitting_commands() {
        let config = Config::parse_explicit("[modekeys]\nCapsLock=mode\nJ=Left\n").unwrap();
        let mut core = Core::new(config);
        core.input(
            KeyEvent {
                key: Key::CAPS_LOCK,
                direction: Direction::Down,
            },
            Duration::ZERO,
        );
        core.observe_unbound(KeyEvent {
            key: Key::CAPS_LOCK,
            direction: Direction::Up,
        });
        assert!(!core.command_mode());
        core.observe_unbound(KeyEvent {
            key: "J".parse().unwrap(),
            direction: Direction::Down,
        });
        core.observe_unbound(KeyEvent {
            key: "J".parse().unwrap(),
            direction: Direction::Up,
        });
        assert!(!core.command_mode());
    }
}
