//! Resolve deployment policy once; the portable path and argument rules stay intact.
use super::*;
use std::{ffi::OsString, path::Path, sync::atomic::AtomicUsize};
use windows_sys::Win32::System::{LibraryLoader::GetProcAddress, SystemInformation::*};

static CONTEXT: OnceLock<Context> = OnceLock::new();

#[derive(Clone, Debug)]
pub struct Context {
    pub resources: PathBuf,
    pub config: PathBuf,
    pub inputsim: PathBuf,
    pub profiles: PathBuf,
    pub preference: PathBuf,
    pub log: PathBuf,
    pub language_preference: Option<PathBuf>,
    pub family: Option<String>,
    pub show: bool,
}

pub fn context() -> Option<&'static Context> {
    CONTEXT.get()
}
pub fn packaged() -> bool {
    context().is_some_and(|context| context.family.is_some())
}
pub fn initialize(context: Context) -> Result<(), Message> {
    CONTEXT.set(context).map_err(|_| {
        msg!(
            "store.context_initialized",
            "Windows distribution context is already initialized."
        )
    })
}

fn operation(operation: Message, error: impl std::fmt::Display) -> Message {
    msg!(
        "store.operation",
        "{operation}: {error}",
        operation = operation,
        error = error.to_string()
    )
}

pub fn package_family() -> Result<Option<String>, Message> {
    unsafe {
        let address = GetProcAddress(
            GetModuleHandleW(wide("kernel32.dll").as_ptr()),
            c"GetCurrentPackageFamilyName".as_ptr().cast(),
        );
        let Some(address) = address else {
            let mut version: OSVERSIONINFOW = std::mem::zeroed();
            version.dwOSVersionInfoSize = std::mem::size_of_val(&version) as u32;
            if GetVersionExW(&mut version) != 0
                && (version.dwMajorVersion, version.dwMinorVersion) < (6, 2)
            {
                return Ok(None);
            }
            return Err(msg!(
                "store.identity_unavailable",
                "Windows package identity is unavailable on this system."
            ));
        };
        let get: unsafe extern "system" fn(*mut u32, *mut u16) -> i32 =
            std::mem::transmute(address);
        let mut length = 0;
        let result = get(&mut length, std::ptr::null_mut());
        if result == 15700 {
            return Ok(None);
        } // APPMODEL_ERROR_NO_PACKAGE
        let error = || msg!("store.read_identity", "Read Windows package identity");
        if result != ERROR_INSUFFICIENT_BUFFER as i32 || length == 0 {
            return Err(operation(
                error(),
                std::io::Error::from_raw_os_error(result),
            ));
        }
        let mut buffer = vec![0; length as usize];
        let result = get(&mut length, buffer.as_mut_ptr());
        if result != 0 {
            return Err(operation(
                error(),
                std::io::Error::from_raw_os_error(result),
            ));
        }
        let length = buffer.iter().position(|c| *c == 0).unwrap_or(buffer.len());
        String::from_utf16(&buffer[..length])
            .map(Some)
            .map_err(|e| operation(error(), e))
    }
}

impl Context {
    pub fn resolve(directory: PathBuf, family: Option<String>) -> Result<Self, Message> {
        let cache = if family.is_some() {
            Some(super::store::local_cache()?)
        } else {
            None
        };
        Self::paths(directory, family, cache, vec![])
    }

    fn paths(
        directory: PathBuf,
        family: Option<String>,
        cache: Option<PathBuf>,
        arguments: Vec<OsString>,
    ) -> Result<Self, Message> {
        let portable = family.is_none();
        let root = if portable {
            directory.clone()
        } else {
            cache
                .ok_or_else(|| {
                    msg!(
                        "store.cache_unavailable",
                        "Cannot locate the package's local cache folder."
                    )
                })?
                .join("typing")
        };
        let preference = if portable {
            ui::preference_path()
        } else {
            root.join("window-size.txt")
        };
        let context = Self {
            inputsim: directory.join("inputsim.dll"),
            resources: directory,
            config: root.join("configuration.ini"),
            profiles: root.join("profiles"),
            log: preference.with_file_name("typing.log"),
            preference,
            language_preference: if portable {
                std::env::var_os("LOCALAPPDATA")
                    .filter(|p| !p.is_empty())
                    .map(|p| PathBuf::from(p).join("Typing/language.txt"))
            } else {
                Some(root.join("language.txt"))
            },
            family,
            show: false,
        };
        context.arguments(arguments)
    }

    pub fn arguments(mut self, arguments: Vec<OsString>) -> Result<Self, Message> {
        let portable = self.family.is_none();
        if !portable {
            self.show = match arguments.as_slice() {
                [] => true,
                [argument] if argument == "--startup" => false,
                _ => {
                    return Err(msg!(
                        "store.arguments",
                        "The installed MSIX accepts only an ordinary launch or --startup. Path overrides are available in the portable edition."
                    ));
                }
            };
        } else {
            let mut arguments = arguments.into_iter();
            while let Some(option) = arguments.next() {
                let value = arguments.next().ok_or_else(|| {
                    msg!(
                        "main.expected_a_path_after_each_option",
                        "expected a path after each option"
                    )
                })?;
                match option.to_str() {
                    Some("--config") => self.config = value.into(),
                    Some("--inputsim") => self.inputsim = value.into(),
                    Some("--state") => self.preference = value.into(),
                    Some("--log") => self.log = value.into(),
                    _ => {
                        return Err(msg!(
                            "main.options_are_config_inputsim_log_and",
                            "options are --config, --inputsim, --log, and --state"
                        ));
                    }
                }
            }
        }
        Ok(self)
    }

    pub fn prepare_data(&self) -> Result<(), Message> {
        if self.family.is_none() {
            return Ok(());
        }
        let root = self.config.parent().unwrap();
        if root.try_exists().map_err(|e| {
            operation(
                msg!("store.inspect_data", "Inspect installed TypingVE data"),
                e,
            )
        })? {
            return Ok(()); // Existing damaged data is intentionally not repaired.
        }
        seed(&self.resources.join("Defaults"), root).map_err(|e| {
            operation(
                msg!(
                    "store.initialize_data",
                    "Initialize installed TypingVE data"
                ),
                e,
            )
        })
    }
}

fn seed(defaults: &Path, root: &Path) -> Result<(), Message> {
    static NEXT: AtomicUsize = AtomicUsize::new(0);
    let parent = root.parent().unwrap();
    std::fs::create_dir_all(parent).map_err(|e| Message::from(e.to_string()))?;
    let stage = parent.join(format!(
        ".typing-init-{}-{}",
        std::process::id(),
        NEXT.fetch_add(1, Ordering::Relaxed)
    ));
    std::fs::create_dir(&stage).map_err(|e| Message::from(e.to_string()))?;
    struct Staging(PathBuf);
    impl Drop for Staging {
        fn drop(&mut self) {
            let _ = std::fs::remove_dir_all(&self.0);
        }
    }
    let stage = Staging(stage);
    let copy = || -> std::io::Result<()> {
        std::fs::create_dir(stage.0.join("profiles"))?;
        std::fs::copy(
            defaults.join("configuration.ini"),
            stage.0.join("configuration.ini"),
        )?;
        std::fs::copy(
            defaults.join("profiles/default.typing.ini"),
            stage.0.join("profiles/default.typing.ini"),
        )?;
        Ok(())
    };
    copy().map_err(|e| Message::from(e.to_string()))?;
    let application = typing_host::profiles::Application::read(&stage.0.join("configuration.ini"))?;
    if application.selected.as_deref()
        != Some(stage.0.join("profiles/default.typing.ini").as_path())
    {
        return Err(msg!(
            "store.invalid_seed",
            "The packaged configuration must select profiles/default.typing.ini."
        ));
    }
    let (activation, config) = typing_host::profiles::Activation::load(application)?;
    if !activation.valid {
        return Err(activation.error.unwrap_or_else(|| {
            msg!(
                "store.invalid_default",
                "The packaged default profile is invalid."
            )
        }));
    }
    typing_host::Feedback::from_config(&config)?;
    match std::fs::rename(&stage.0, root) {
        Ok(()) => Ok(()),
        Err(_) if root.is_dir() => Ok(()), // A complete directory was published first.
        Err(error) => Err(error.to_string().into()),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn portable_overrides_do_not_move_discovery_or_implicitly_move_logs() {
        let directory = PathBuf::from(r"C:\Typing");
        let defaults = Context::paths(directory.clone(), None, None, vec![]).unwrap();
        let context = Context::paths(
            directory.clone(),
            None,
            None,
            [
                "--config",
                r"D:\custom.ini",
                "--state",
                r"D:\state.txt",
                "--inputsim",
                r"D:\input.dll",
            ]
            .map(OsString::from)
            .into(),
        )
        .unwrap();
        assert_eq!(context.profiles, directory.join("profiles"));
        assert_eq!(context.config, PathBuf::from(r"D:\custom.ini"));
        assert_eq!(context.log, defaults.log);
        assert!(!context.show);
        assert!(Context::paths(directory, None, None, vec!["--startup".into()]).is_err());
    }
    #[test]
    fn package_paths_and_launch_intent_cannot_escape_to_portable_overrides() {
        let context = |args| {
            Context::paths(
                r"C:\Package".into(),
                Some("Test_family".into()),
                Some(r"C:\User\LocalCache".into()),
                args,
            )
        };
        let foreground = context(vec![]).unwrap();
        assert_eq!(
            foreground.config,
            PathBuf::from(r"C:\User\LocalCache\typing\configuration.ini")
        );
        assert_eq!(
            foreground.profiles,
            foreground.config.parent().unwrap().join("profiles")
        );
        assert!(foreground.show);
        assert!(!context(vec!["--startup".into()]).unwrap().show);
        assert!(context(vec!["--config".into(), "other.ini".into()]).is_err());
        assert!(context(vec!["--startup".into(), "--startup".into()]).is_err());
    }
    #[test]
    fn first_use_is_complete_preserves_existing_files_and_never_publishes_bad_seeds() {
        static NEXT: AtomicUsize = AtomicUsize::new(0);
        let base = std::env::temp_dir().join(format!(
            "typing-store-seed-{}-{}",
            std::process::id(),
            NEXT.fetch_add(1, Ordering::Relaxed)
        ));
        std::fs::create_dir(&base).unwrap();
        let defaults = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
            .parent()
            .unwrap()
            .join("resources");
        let root = base.join("typing");
        seed(&defaults, &root).unwrap();
        let config = root.join("configuration.ini");
        std::fs::write(&config, "user contents").unwrap();
        seed(&defaults, &root).unwrap();
        assert_eq!(std::fs::read_to_string(&config).unwrap(), "user contents");
        let other = base.join("invalid");
        assert!(seed(&base.join("missing"), &other).is_err());
        assert!(!other.exists());
        assert_eq!(std::fs::read_dir(&base).unwrap().count(), 1);
        let broken = base.join("broken-defaults");
        std::fs::create_dir_all(broken.join("profiles")).unwrap();
        std::fs::copy(
            defaults.join("configuration.ini"),
            broken.join("configuration.ini"),
        )
        .unwrap();
        std::fs::write(broken.join("profiles/default.typing.ini"), "not a profile").unwrap();
        assert!(seed(&broken, &other).is_err());
        assert!(!other.exists());
        let context = Context::paths(
            base.clone(),
            Some("Test_family".into()),
            Some(base.clone()),
            vec![],
        )
        .unwrap();
        context.prepare_data().unwrap();
        assert_eq!(std::fs::read_to_string(&config).unwrap(), "user contents");
        std::fs::remove_dir_all(base).unwrap();
    }
}
