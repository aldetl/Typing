//! Portable language-profile policy. Native hosts provide paths, preferences,
//! OS language observations and UI-thread integration.
use std::{
    collections::{BTreeMap, BTreeSet},
    fs,
    path::{Path, PathBuf},
    sync::{Arc, OnceLock},
};
use typing_core::{Message, msg};

pub const ENGLISH_SOURCE: &str = include_str!("../../resources/languages/en.ini");
pub const ENGLISH_GUIDE: &str = include_str!("../../docs/USER_GUIDE.md");

#[derive(Clone, Debug)]
pub struct Profile {
    pub id: String,
    pub name: String,
    pub strings: BTreeMap<String, String>,
    pub unknown: Vec<String>,
}

fn invalid(line: usize, reason: Message) -> Message {
    msg!(
        "language.invalid",
        "Invalid language profile, line {line}: {reason}",
        line = line,
        reason = reason
    )
}
fn decode(text: &str) -> Result<String, Message> {
    let mut output = String::new();
    let mut chars = text.chars();
    while let Some(c) = chars.next() {
        if c == '\\' {
            output.push(match chars.next() {
                Some('\\') => '\\',
                Some('n') => '\n',
                Some('r') => '\r',
                Some('t') => '\t',
                _ => return Err(msg!("language.escape", "Invalid text escape")),
            });
        } else if c.is_control() {
            return Err(msg!(
                "language.control",
                "Control characters are not allowed"
            ));
        } else {
            output.push(c);
        }
    }
    Ok(output)
}
pub fn valid_id(id: &str) -> bool {
    let mut parts = id.split('-');
    let primary = parts.next().unwrap_or("");
    !id.eq_ignore_ascii_case("system")
        && (2..=8).contains(&primary.len())
        && primary.bytes().all(|c| c.is_ascii_alphabetic())
        && parts.all(|p| (1..=8).contains(&p.len()) && p.bytes().all(|c| c.is_ascii_alphanumeric()))
}
pub fn placeholders(template: &str) -> Result<BTreeSet<&str>, Message> {
    let mut names = BTreeSet::new();
    let mut rest = template;
    while !rest.is_empty() {
        if let Some(tail) = rest.strip_prefix("{{").or_else(|| rest.strip_prefix("}}")) {
            rest = tail;
        } else if let Some(tail) = rest.strip_prefix('{') {
            let end = tail
                .find('}')
                .ok_or_else(|| msg!("language.template", "Invalid message placeholder"))?;
            let name = &tail[..end];
            if name.is_empty() || !name.bytes().all(|c| c.is_ascii_alphanumeric() || c == b'_') {
                return Err(msg!("language.template", "Invalid message placeholder"));
            }
            names.insert(name);
            rest = &tail[end + 1..];
        } else if rest.starts_with('}') {
            return Err(msg!("language.template", "Invalid message placeholder"));
        } else {
            rest = &rest[rest.chars().next().unwrap().len_utf8()..];
        }
    }
    Ok(names)
}

impl Profile {
    pub fn parse(source: &str, baseline: Option<&Profile>) -> Result<Self, Message> {
        let mut section = "";
        let mut sections = BTreeSet::new();
        let mut metadata = BTreeMap::new();
        let mut strings = BTreeMap::new();
        let mut unknown = Vec::new();
        for (index, source_line) in source.trim_start_matches('\u{feff}').lines().enumerate() {
            let line = index + 1;
            let source_line = source_line.trim();
            if source_line.is_empty() || source_line.starts_with(';') {
                continue;
            }
            if source_line.starts_with('[') && source_line.ends_with(']') {
                section = &source_line[1..source_line.len() - 1];
                if !["language", "strings"].contains(&section)
                    || !sections.insert(section)
                    || (section == "strings" && !sections.contains("language"))
                {
                    return Err(invalid(
                        line,
                        msg!(
                            "language.section",
                            "Unknown, repeated, or misplaced section"
                        ),
                    ));
                }
                continue;
            }
            let (key, encoded) = source_line
                .split_once('=')
                .ok_or_else(|| invalid(line, msg!("language.assignment", "Expected key=value")))?;
            let key = key.trim();
            let value = decode(encoded.trim()).map_err(|e| invalid(line, e))?;
            let target = match section {
                "language" => {
                    if !["version", "id", "name"].contains(&key)
                        || value.chars().any(char::is_control)
                    {
                        return Err(invalid(
                            line,
                            msg!("language.metadata", "Invalid language metadata"),
                        ));
                    }
                    &mut metadata
                }
                "strings" => {
                    if key.is_empty()
                        || !key
                            .bytes()
                            .all(|c| c.is_ascii_alphanumeric() || c == b'.' || c == b'_')
                    {
                        return Err(invalid(
                            line,
                            msg!("language.message_id", "Invalid message ID"),
                        ));
                    }
                    if baseline.is_some_and(|baseline| !baseline.strings.contains_key(key)) {
                        unknown.push(key.to_owned());
                    }
                    if !value.is_empty() {
                        let names = placeholders(&value).map_err(|e| invalid(line, e))?;
                        if let Some(baseline) = baseline
                            && let Some(english) = baseline.strings.get(key)
                            && names != placeholders(english)?
                        {
                            return Err(invalid(
                                line,
                                msg!(
                                    "language.arguments",
                                    "Message arguments do not match English: {id}",
                                    id = key
                                ),
                            ));
                        }
                    }
                    &mut strings
                }
                _ => {
                    return Err(invalid(
                        line,
                        msg!(
                            "language.section",
                            "Unknown, repeated, or misplaced section"
                        ),
                    ));
                }
            };
            if target.insert(key.to_owned(), value).is_some() {
                return Err(invalid(
                    line,
                    msg!("language.duplicate", "Repeated key: {key}", key = key),
                ));
            }
        }
        if sections.len() != 2 || metadata.get("version").map(String::as_str) != Some("1") {
            return Err(invalid(
                0,
                msg!(
                    "language.version",
                    "Expected language format version=1 and both required sections"
                ),
            ));
        }
        let id = metadata.remove("id").unwrap_or_default();
        let name = metadata.remove("name").unwrap_or_default();
        if !valid_id(&id) || name.is_empty() {
            return Err(invalid(
                0,
                msg!(
                    "language.identity",
                    "Invalid language ID or empty native name"
                ),
            ));
        }
        if let Some(baseline) = baseline {
            strings.retain(|key, value| baseline.strings.contains_key(key) && !value.is_empty());
        } else if strings.values().any(String::is_empty) {
            return Err(invalid(
                0,
                msg!(
                    "language.empty_english",
                    "English baseline messages must not be empty"
                ),
            ));
        }
        Ok(Self {
            id,
            name,
            strings,
            unknown,
        })
    }
}
pub fn english() -> &'static Profile {
    static BASELINE: OnceLock<Profile> = OnceLock::new();
    BASELINE.get_or_init(|| {
        Profile::parse(ENGLISH_SOURCE, None).expect("valid embedded English profile")
    })
}

#[derive(Clone, Debug)]
pub struct Snapshot {
    pub id: String,
    pub name: String,
    pub revision: u64,
    pub guide: String,
    strings: BTreeMap<String, String>,
}
impl Snapshot {
    pub fn english(revision: u64) -> Self {
        Self {
            id: "en".into(),
            name: "English".into(),
            revision,
            guide: ENGLISH_GUIDE.into(),
            strings: english().strings.clone(),
        }
    }
    pub fn render(&self, message: &Message) -> String {
        message.render_with(&|id| self.strings.get(id).map(String::as_str))
    }
    pub fn text<'a>(&'a self, id: &'a str) -> &'a str {
        self.strings.get(id).map(String::as_str).unwrap_or(id)
    }
}
#[derive(Clone, Debug)]
pub struct Choice {
    pub id: String,
    pub name: String,
    pub path: Option<PathBuf>,
}
pub struct Catalog {
    pub choices: Vec<Choice>,
    pub diagnostics: Vec<Message>,
}
fn file_error(path: &Path, error: Message) -> Message {
    msg!(
        "language.file",
        "Language file {path}: {error}",
        path = path.display(),
        error = error
    )
}
fn read_profile(path: &Path) -> Result<Profile, Message> {
    let source = fs::read_to_string(path).map_err(|e| file_error(path, e.to_string().into()))?;
    let profile = Profile::parse(&source, Some(english())).map_err(|e| file_error(path, e))?;
    if !path
        .file_stem()
        .is_some_and(|s| s.to_string_lossy().eq_ignore_ascii_case(&profile.id))
    {
        return Err(file_error(
            path,
            msg!("language.filename", "Filename must match the language ID"),
        ));
    }
    Ok(profile)
}
pub fn discover(directory: &Path) -> Catalog {
    let mut catalog = Catalog {
        choices: Vec::new(),
        diagnostics: Vec::new(),
    };
    let mut by_id: BTreeMap<String, Vec<Choice>> = BTreeMap::new();
    match fs::read_dir(directory) {
        Ok(entries) => {
            for entry in entries {
                let entry = match entry {
                    Ok(entry) => entry,
                    Err(e) => {
                        catalog
                            .diagnostics
                            .push(file_error(directory, e.to_string().into()));
                        continue;
                    }
                };
                let path = entry.path();
                if !path
                    .extension()
                    .is_some_and(|e| e.eq_ignore_ascii_case("ini"))
                    || !path.is_file()
                {
                    continue;
                }
                match read_profile(&path) {
                    Ok(profile) => {
                        for id in &profile.unknown {
                            catalog.diagnostics.push(msg!(
                                "language.unknown",
                                "Unknown language message ignored: {id}",
                                id = id
                            ));
                        }
                        by_id
                            .entry(profile.id.to_ascii_lowercase())
                            .or_default()
                            .push(Choice {
                                id: profile.id,
                                name: profile.name,
                                path: Some(path),
                            });
                    }
                    Err(error) => catalog.diagnostics.push(error),
                }
            }
        }
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => {}
        Err(e) => catalog
            .diagnostics
            .push(file_error(directory, e.to_string().into())),
    }
    for (id, mut choices) in by_id {
        if choices.len() == 1 {
            catalog.choices.push(choices.remove(0));
        } else {
            catalog.diagnostics.push(msg!(
                "language.ambiguous",
                "Multiple language profiles declare {id}",
                id = id
            ));
        }
    }
    if !catalog
        .choices
        .iter()
        .any(|c| c.id.eq_ignore_ascii_case("en"))
    {
        catalog.choices.push(Choice {
            id: "en".into(),
            name: "English".into(),
            path: None,
        });
    }
    catalog.choices.sort_by_key(|c| {
        (
            u8::from(!c.id.eq_ignore_ascii_case("en")),
            c.id.to_ascii_lowercase(),
        )
    });
    catalog
}

pub trait Platform {
    fn language_directory(&self) -> &Path;
    fn preferred_languages(&self) -> Vec<String>;
    fn read_selection(&self) -> Result<Option<String>, Message>;
    /// Replace the selection atomically; failure must preserve its prior value.
    fn write_selection(&self, selection: &str) -> Result<(), Message>;
}
pub struct Prepared {
    pub requested: String,
    pub snapshot: Arc<Snapshot>,
}
pub struct Service {
    pub requested: String,
    pub snapshot: Arc<Snapshot>,
    pub catalog: Catalog,
    pub notice: Option<Message>,
}
impl Service {
    pub fn initialize(platform: &impl Platform) -> Self {
        let mut service = Self {
            requested: "system".into(),
            snapshot: Arc::new(Snapshot::english(0)),
            catalog: discover(platform.language_directory()),
            notice: None,
        };
        match platform.read_selection() {
            Ok(Some(value)) => {
                let selection = value
                    .strip_suffix("\r\n")
                    .or_else(|| value.strip_suffix('\n'))
                    .unwrap_or(&value);
                if selection.eq_ignore_ascii_case("system") || valid_id(selection) {
                    service.requested = selection.to_owned();
                } else {
                    service.notice = Some(msg!(
                        "language.preference_invalid",
                        "Invalid saved language preference; using System default."
                    ));
                }
            }
            Ok(None) => {}
            Err(error) => service.notice = Some(error),
        }
        match service.prepare(&service.requested, platform) {
            Ok(prepared) => service.snapshot = prepared.snapshot,
            Err(error) => {
                service.notice = Some(msg!(
                    "language.unavailable",
                    "Saved language {id} is unavailable. Using English. {error}",
                    id = service.requested,
                    error = error
                ))
            }
        }
        service
    }
    pub fn rescan(&mut self, platform: &impl Platform) {
        self.catalog = discover(platform.language_directory());
    }
    fn load(&self, selection: &str) -> Result<Snapshot, Message> {
        let choice = self
            .catalog
            .choices
            .iter()
            .find(|c| c.id.eq_ignore_ascii_case(selection))
            .ok_or_else(|| {
                msg!(
                    "language.missing",
                    "Language profile not found: {id}",
                    id = selection
                )
            })?;
        let mut snapshot = Snapshot::english(self.snapshot.revision + 1);
        if let Some(path) = &choice.path {
            let loaded = (|| {
                let profile = read_profile(path)?;
                let guide = path.parent().unwrap().join(format!("{}.md", profile.id));
                let guide = match fs::read_to_string(&guide) {
                    Ok(text) if !text.contains('\0') => text,
                    Ok(_) => {
                        return Err(file_error(
                            &guide,
                            msg!("language.control", "Control characters are not allowed"),
                        ));
                    }
                    Err(e) if e.kind() == std::io::ErrorKind::NotFound => ENGLISH_GUIDE.into(),
                    Err(e) => return Err(file_error(&guide, e.to_string().into())),
                };
                Ok((profile, guide))
            })();
            match loaded {
                Ok((profile, guide)) => {
                    snapshot.id = profile.id;
                    snapshot.name = profile.name;
                    snapshot.strings.extend(profile.strings);
                    snapshot.guide = guide;
                }
                Err(_) if selection.eq_ignore_ascii_case("en") => {}
                Err(error) => return Err(error),
            }
        }
        Ok(snapshot)
    }
    pub fn prepare(&self, requested: &str, platform: &impl Platform) -> Result<Prepared, Message> {
        let snapshot = if requested.eq_ignore_ascii_case("system") {
            let mut seen = BTreeSet::new();
            let mut selected = None;
            for preference in platform.preferred_languages() {
                let mut candidates = vec![preference.to_ascii_lowercase()];
                if candidates[0] == "zh-hans" || candidates[0].starts_with("zh-hans-") {
                    candidates.push("zh-cn".into());
                }
                let mut parent = preference.as_str();
                while let Some((prefix, _)) = parent.rsplit_once('-') {
                    candidates.push(prefix.to_ascii_lowercase());
                    parent = prefix;
                }
                for candidate in candidates {
                    if seen.insert(candidate.clone())
                        && let Ok(snapshot) = self.load(&candidate)
                    {
                        selected = Some(snapshot);
                        break;
                    }
                }
                if selected.is_some() {
                    break;
                }
            }
            selected.unwrap_or_else(|| Snapshot::english(self.snapshot.revision + 1))
        } else {
            self.load(requested)?
        };
        Ok(Prepared {
            requested: if requested.eq_ignore_ascii_case("system") {
                "system".into()
            } else {
                snapshot.id.clone()
            },
            snapshot: Arc::new(snapshot),
        })
    }
    pub fn commit(&mut self, prepared: Prepared, platform: &impl Platform) -> Result<(), Message> {
        platform.write_selection(&prepared.requested)?;
        self.requested = prepared.requested;
        self.snapshot = prepared.snapshot;
        self.notice = None;
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::{
        cell::{Cell, RefCell},
        sync::atomic::{AtomicU64, Ordering},
    };

    struct Fake {
        root: PathBuf,
        parent: PathBuf,
        saved: RefCell<Option<String>>,
        preferred: Vec<String>,
        fail_read: bool,
        fail_write: Cell<bool>,
    }
    impl Fake {
        fn new(preferred: &[&str]) -> Self {
            static NEXT: AtomicU64 = AtomicU64::new(0);
            let parent = std::path::absolute(
                Path::new(env!("CARGO_MANIFEST_DIR")).join("../target/language-tests"),
            )
            .unwrap();
            fs::create_dir_all(&parent).unwrap();
            let parent = parent.canonicalize().unwrap();
            let root = parent.join(format!(
                "{}-{}",
                std::process::id(),
                NEXT.fetch_add(1, Ordering::Relaxed)
            ));
            fs::create_dir(&root).unwrap();
            Self {
                root,
                parent,
                saved: RefCell::new(None),
                preferred: preferred.iter().map(|v| (*v).into()).collect(),
                fail_read: false,
                fail_write: Cell::new(false),
            }
        }
        fn chinese(&self) {
            fs::write(
                self.root.join("zh-CN.ini"),
                include_str!("../../resources/languages/zh-CN.ini"),
            )
            .unwrap();
        }
    }
    impl Drop for Fake {
        fn drop(&mut self) {
            let target = self.root.canonicalize().unwrap();
            assert!(target.starts_with(&self.parent) && target != self.parent);
            fs::remove_dir_all(target).unwrap();
        }
    }
    impl Platform for Fake {
        fn language_directory(&self) -> &Path {
            &self.root
        }
        fn preferred_languages(&self) -> Vec<String> {
            self.preferred.clone()
        }
        fn read_selection(&self) -> Result<Option<String>, Message> {
            if self.fail_read {
                Err("injected read failure".into())
            } else {
                Ok(self.saved.borrow().clone())
            }
        }
        fn write_selection(&self, value: &str) -> Result<(), Message> {
            if self.fail_write.get() {
                Err("injected write failure".into())
            } else {
                *self.saved.borrow_mut() = Some(value.into());
                Ok(())
            }
        }
    }
    fn profile(strings: &str) -> String {
        format!("[language]\nversion=1\nid=zh-CN\nname=简体中文\n[strings]\n{strings}\n")
    }

    #[test]
    fn utf8_bom_escapes_and_template_contract() {
        let text = format!(
            "\u{feff}{}",
            profile("language.missing=找不到 {id}\\n重试\\t\\\\ ;#=\"文字\"")
        );
        let parsed = Profile::parse(&text.replace('\n', "\r\n"), Some(english())).unwrap();
        assert_eq!(
            parsed.strings["language.missing"],
            "找不到 {id}\n重试\t\\ ;#=\"文字\""
        );
        let mut snapshot = Snapshot::english(0);
        snapshot.strings.extend(parsed.strings);
        assert_eq!(
            snapshot.render(&msg!(
                "language.missing",
                "Language profile not found: {id}",
                id = "{nested}\\"
            )),
            "找不到 {nested}\\\n重试\t\\ ;#=\"文字\""
        );
        assert_eq!(
            placeholders("{{literal}} {name} {name} {line}").unwrap(),
            ["line", "name"].into()
        );
        for bad in [
            "language.missing=坏 {wrong}",
            "language.missing=坏 {id",
            "language.missing=坏 }",
            "language.missing=\\q",
            "language.missing=bad\0text",
        ] {
            assert!(
                Profile::parse(&profile(bad), Some(english())).is_err(),
                "{bad:?}"
            );
        }
    }
    #[test]
    fn metadata_versions_duplicates_unknown_and_missing_messages() {
        for text in [
            profile("").replace("version=1", "version=2"),
            profile("").replace("id=zh-CN", "id=system"),
            profile("").replace("id=zh-CN", "id=../zh"),
            profile("").replace("name=简体中文", "name=简体\\n中文"),
            profile("language.missing=\nlanguage.missing="),
            profile("") + "[strings]\n",
            profile("") + "[unknown]\n",
            profile("").replace("id=zh-CN", "id=zh-CN\nextra=value"),
        ] {
            assert!(Profile::parse(&text, Some(english())).is_err(), "{text}");
        }
        let parsed = Profile::parse(
            &profile("future.message=Future {x}\nlanguage.missing="),
            Some(english()),
        )
        .unwrap();
        assert_eq!(parsed.unknown, ["future.message"]);
        assert!(parsed.strings.is_empty());
        let message = msg!(
            "language.missing",
            "Language profile not found: {id}",
            id = "ja"
        );
        assert_eq!(
            Snapshot::english(0).render(&message),
            "Language profile not found: ja"
        );
        assert!(
            Profile::parse(
                &profile("").replace("[language]", "[strings]"),
                Some(english())
            )
            .is_err()
        );
    }
    #[test]
    fn system_resolution_preserves_chinese_script_distinctions_and_order() {
        for (preferred, expected) in [
            (vec!["zh-Hans-SG"], "zh-CN"),
            (vec!["ZH-cn"], "zh-CN"),
            (vec!["zh-Hant", "zh-CN"], "zh-CN"),
            (vec!["en-US", "zh-CN"], "en"),
            (vec!["zh-Hant"], "en"),
            (vec!["zh-TW"], "en"),
            (vec!["zh"], "en"),
            (vec!["xx", "zh-CN"], "zh-CN"),
        ] {
            let adapter = Fake::new(&preferred);
            adapter.chinese();
            let service = Service::initialize(&adapter);
            assert_eq!(service.snapshot.id, expected, "{preferred:?}");
            assert_eq!(service.requested, "system");
            assert!(adapter.saved.borrow().is_none());
        }
    }
    #[test]
    fn explicit_switch_and_failed_persistence_preserve_previous_snapshot() {
        let adapter = Fake::new(&[]);
        adapter.chinese();
        let mut service = Service::initialize(&adapter);
        let english = service.snapshot.clone();
        let candidate = service.prepare("zh-CN", &adapter).unwrap();
        adapter.fail_write.set(true);
        assert!(service.commit(candidate, &adapter).is_err());
        assert!(Arc::ptr_eq(&english, &service.snapshot));
        assert_eq!(service.requested, "system");
        assert!(adapter.saved.borrow().is_none());
        adapter.fail_write.set(false);
        service
            .commit(service.prepare("zh-CN", &adapter).unwrap(), &adapter)
            .unwrap();
        assert_eq!(adapter.saved.borrow().as_deref(), Some("zh-CN"));
        let restarted = Service::initialize(&adapter);
        assert_eq!(restarted.snapshot.id, "zh-CN");
        assert_eq!(restarted.snapshot.text("ui.settings"), "设置");
        assert!(service.prepare("missing", &adapter).is_err());
    }
    #[test]
    fn startup_resource_and_preference_failures_are_nonfatal_without_rewriting() {
        let mut adapter = Fake::new(&[]);
        *adapter.saved.borrow_mut() = Some("zh-CN\n".into());
        let service = Service::initialize(&adapter);
        assert_eq!(service.snapshot.id, "en");
        assert_eq!(service.requested, "zh-CN");
        assert!(service.notice.is_some());
        assert_eq!(adapter.saved.borrow().as_deref(), Some("zh-CN\n"));
        for bad in ["en\n\n", " en", "zh/../../en", "system\r\nen"] {
            *adapter.saved.borrow_mut() = Some(bad.into());
            let service = Service::initialize(&adapter);
            assert_eq!(service.requested, "system");
            assert!(service.notice.is_some());
            assert_eq!(adapter.saved.borrow().as_deref(), Some(bad));
        }
        adapter.fail_read = true;
        assert!(Service::initialize(&adapter).notice.is_some());
    }
    #[test]
    fn changed_removed_and_invalid_files_do_not_mutate_active_data() {
        let adapter = Fake::new(&[]);
        adapter.chinese();
        let mut service = Service::initialize(&adapter);
        service
            .commit(service.prepare("zh-CN", &adapter).unwrap(), &adapter)
            .unwrap();
        let original = service.snapshot.clone();
        fs::write(
            adapter.root.join("zh-CN.ini"),
            profile("ui.settings=新的设置"),
        )
        .unwrap();
        service.rescan(&adapter);
        assert!(Arc::ptr_eq(&original, &service.snapshot));
        let candidate = service.prepare("zh-CN", &adapter).unwrap();
        assert_eq!(candidate.snapshot.text("ui.settings"), "新的设置");
        fs::write(adapter.root.join("zh-CN.ini"), [0xff, 0xfe]).unwrap();
        assert!(service.prepare("zh-CN", &adapter).is_err());
        assert!(Arc::ptr_eq(&original, &service.snapshot));
        fs::remove_file(adapter.root.join("zh-CN.ini")).unwrap();
        service.rescan(&adapter);
        assert!(service.prepare("zh-CN", &adapter).is_err());
        assert_eq!(service.snapshot.text("ui.settings"), "设置");
    }
    #[test]
    fn guide_loading_fallback_and_invalid_utf8_are_part_of_preparation() {
        let adapter = Fake::new(&[]);
        adapter.chinese();
        let service = Service::initialize(&adapter);
        assert_eq!(
            service.prepare("zh-CN", &adapter).unwrap().snapshot.guide,
            ENGLISH_GUIDE
        );
        fs::write(adapter.root.join("zh-CN.md"), "# 中文指南").unwrap();
        assert_eq!(
            service.prepare("zh-CN", &adapter).unwrap().snapshot.guide,
            "# 中文指南"
        );
        fs::write(adapter.root.join("zh-CN.md"), [0xff]).unwrap();
        assert!(service.prepare("zh-CN", &adapter).is_err());
        assert_eq!(service.snapshot.id, "en");
    }
    #[test]
    fn discovery_is_nonrecursive_and_rejects_ambiguous_id_and_filename() {
        let adapter = Fake::new(&[]);
        adapter.chinese();
        fs::create_dir(adapter.root.join("nested")).unwrap();
        fs::write(
            adapter.root.join("nested/ja.ini"),
            profile("").replace("id=zh-CN", "id=ja"),
        )
        .unwrap();
        fs::write(adapter.root.join("wrong.ini"), profile("")).unwrap();
        let catalog = discover(&adapter.root);
        assert_eq!(
            catalog
                .choices
                .iter()
                .map(|c| c.id.as_str())
                .collect::<Vec<_>>(),
            ["en", "zh-CN"]
        );
        assert!(!catalog.diagnostics.is_empty());
        assert_eq!(discover(&adapter.root.join("absent")).choices[0].id, "en");
        // Windows aliases case variants to one file; distinct-file ambiguity
        // can be exercised on case-sensitive filesystems with this same test.
        #[cfg(not(target_os = "windows"))]
        {
            fs::write(adapter.root.join("ZH-cn.ini"), profile("")).unwrap();
            assert_eq!(discover(&adapter.root).choices.len(), 1);
        }
    }
    fn rust_string(source: &str, offset: &mut usize) -> String {
        while source.as_bytes()[*offset].is_ascii_whitespace() || source.as_bytes()[*offset] == b','
        {
            *offset += 1;
        }
        assert_eq!(source.as_bytes()[*offset], b'"');
        *offset += 1;
        let mut result = String::new();
        while *offset < source.len() {
            let c = source[*offset..].chars().next().unwrap();
            *offset += c.len_utf8();
            if c == '"' {
                return result;
            }
            if c == '\\' {
                let next = source[*offset..].chars().next().unwrap();
                *offset += next.len_utf8();
                result.push(match next {
                    'n' => '\n',
                    'r' => '\r',
                    't' => '\t',
                    '"' => '"',
                    '\\' => '\\',
                    _ => panic!("unsupported Rust string escape: {next}"),
                });
            } else {
                result.push(c);
            }
        }
        panic!("unterminated Rust string")
    }
    #[test]
    fn shipped_profiles_cover_actual_message_ids_and_keep_english_sources_aligned() {
        let chinese = Profile::parse(
            include_str!("../../resources/languages/zh-CN.ini"),
            Some(english()),
        )
        .unwrap();
        assert!(chinese.unknown.is_empty());
        assert_eq!(chinese.strings.len(), english().strings.len());
        let root = Path::new(env!("CARGO_MANIFEST_DIR")).parent().unwrap();
        let mut referenced = BTreeSet::new();
        for directory in ["typing-core/src", "typing-host/src", "typing-windows/src"] {
            for entry in fs::read_dir(root.join(directory)).unwrap() {
                let path = entry.unwrap().path();
                if path.extension().is_none_or(|e| e != "rs") {
                    continue;
                }
                let source = fs::read_to_string(&path).unwrap();
                let source = source.split("#[cfg(test)]\nmod tests").next().unwrap();
                let source = source.split("#[cfg(test)]\r\nmod tests").next().unwrap();
                let mut cursor = 0;
                while let Some(start) = source[cursor..].find("msg!(") {
                    let mut offset = cursor + start + 5;
                    let id = rust_string(source, &mut offset);
                    let fallback = rust_string(source, &mut offset);
                    assert_eq!(
                        english().strings.get(&id),
                        Some(&fallback),
                        "{}: {id}",
                        path.display()
                    );
                    assert!(chinese.strings.contains_key(&id), "missing Chinese: {id}");
                    referenced.insert(id);
                    cursor = offset;
                }
            }
        }
        assert_eq!(
            referenced.len(),
            english().strings.len(),
            "stale/unreferenced English messages"
        );
        assert_eq!(
            crate::help::introduction_message().to_string(),
            crate::help::introduction_text()
        );
        let guide = include_str!("../../resources/languages/zh-CN.md");
        assert_eq!(
            guide.matches("\n## ").count(),
            ENGLISH_GUIDE.matches("\n## ").count()
        );
        for token in [
            "LMenu+C=Control+C",
            "!J=Left",
            "stepN",
            "language.txt",
            "mode_cursor_tip",
        ] {
            assert!(guide.contains(token), "{token}");
        }
    }
}
