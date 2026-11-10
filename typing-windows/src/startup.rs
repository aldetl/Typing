//! Current-user Windows sign-in registration. No INI enablement flag is stored.
use std::{os::windows::ffi::OsStrExt, path::Path};
use windows_sys::Win32::{Foundation::*, System::Registry::*};

const RUN_KEY: &str = r"Software\Microsoft\Windows\CurrentVersion\Run";
const VALUE_NAME: &str = "Typing";
const MAX_COMMAND: usize = 260;

fn wide(value: &str) -> Vec<u16> {
    value.encode_utf16().chain(Some(0)).collect()
}

fn error(operation: &str, code: u32) -> String {
    format!(
        "{operation}: {}",
        std::io::Error::from_raw_os_error(code as i32)
    )
}

fn command(executable: &Path) -> Result<Vec<u16>, String> {
    let path: Vec<_> = executable.as_os_str().encode_wide().collect();
    if !executable.is_absolute() || path.iter().any(|&c| c == 0 || c == b'"' as u16) {
        return Err(
            "Startup requires an absolute executable path without quotes or NUL characters.".into(),
        );
    }
    if path.len() + 2 > MAX_COMMAND {
        return Err("The executable path is too long for Windows startup. Move Typing to a shorter folder path and try again.".into());
    }
    Ok(std::iter::once(b'"' as u16)
        .chain(path)
        .chain([b'"' as u16, 0])
        .collect())
}

struct Key(HKEY);

impl Drop for Key {
    fn drop(&mut self) {
        unsafe { RegCloseKey(self.0) };
    }
}

// Tests use the same adapter with a disposable subkey, never the actual Run key.
struct Registration<'a> {
    subkey: &'a str,
}

impl Registration<'_> {
    fn open(&self, access: u32, create: bool) -> Result<Option<Key>, String> {
        let mut key = 0;
        let name = wide(self.subkey);
        let code = unsafe {
            if create {
                RegCreateKeyExW(
                    HKEY_CURRENT_USER,
                    name.as_ptr(),
                    0,
                    std::ptr::null(),
                    REG_OPTION_NON_VOLATILE,
                    access | KEY_WOW64_64KEY,
                    std::ptr::null(),
                    &mut key,
                    std::ptr::null_mut(),
                )
            } else {
                RegOpenKeyExW(
                    HKEY_CURRENT_USER,
                    name.as_ptr(),
                    0,
                    access | KEY_WOW64_64KEY,
                    &mut key,
                )
            }
        };
        match code {
            ERROR_SUCCESS => Ok(Some(Key(key))),
            ERROR_FILE_NOT_FOUND | ERROR_PATH_NOT_FOUND if !create => Ok(None),
            _ => Err(error("Open Windows startup registration", code)),
        }
    }

    fn matches(&self, executable: &Path) -> Result<bool, String> {
        // An overlong current location cannot match a valid Run command.
        let expected = command(executable);
        let Some(key) = self.open(KEY_QUERY_VALUE, false)? else {
            return Ok(false);
        };
        let mut data = [0u16; MAX_COMMAND + 1];
        let mut size = std::mem::size_of_val(&data) as u32;
        let mut kind = 0;
        let code = unsafe {
            RegQueryValueExW(
                key.0,
                wide(VALUE_NAME).as_ptr(),
                std::ptr::null(),
                &mut kind,
                data.as_mut_ptr().cast(),
                &mut size,
            )
        };
        match code {
            ERROR_FILE_NOT_FOUND | ERROR_MORE_DATA => Ok(false),
            ERROR_SUCCESS => Ok(expected.is_ok_and(|expected| {
                kind == REG_SZ
                    && size as usize == expected.len() * 2
                    && data[..expected.len()] == expected
            })),
            _ => Err(error("Read Windows startup registration", code)),
        }
    }

    fn enable(&self, executable: &Path) -> Result<(), String> {
        let command = command(executable)?;
        let key = self.open(KEY_SET_VALUE, true)?.unwrap();
        let code = unsafe {
            RegSetValueExW(
                key.0,
                wide(VALUE_NAME).as_ptr(),
                0,
                REG_SZ,
                command.as_ptr().cast(),
                (command.len() * 2) as u32,
            )
        };
        if code == ERROR_SUCCESS {
            Ok(())
        } else {
            Err(error("Enable Start with Windows", code))
        }
    }

    fn disable(&self) -> Result<(), String> {
        let Some(key) = self.open(KEY_SET_VALUE, false)? else {
            return Ok(());
        };
        let code = unsafe { RegDeleteValueW(key.0, wide(VALUE_NAME).as_ptr()) };
        if matches!(code, ERROR_SUCCESS | ERROR_FILE_NOT_FOUND) {
            Ok(())
        } else {
            Err(error("Cancel Start with Windows", code))
        }
    }
}

pub fn is_registered() -> Result<bool, String> {
    let executable = std::env::current_exe().map_err(|e| format!("Locate Typing: {e}"))?;
    Registration { subkey: RUN_KEY }.matches(&executable)
}

pub fn set_enabled(enabled: bool) -> Result<(), String> {
    let registration = Registration { subkey: RUN_KEY };
    if enabled {
        let executable = std::env::current_exe().map_err(|e| format!("Locate Typing: {e}"))?;
        registration.enable(&executable)
    } else {
        registration.disable()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::atomic::{AtomicUsize, Ordering};

    struct Fixture(String);

    impl Fixture {
        fn new() -> Self {
            static NEXT: AtomicUsize = AtomicUsize::new(0);
            Self(format!(
                r"Software\Typing\StartupTests-{}-{}-{}",
                std::process::id(),
                std::time::SystemTime::now()
                    .duration_since(std::time::UNIX_EPOCH)
                    .unwrap()
                    .as_nanos(),
                NEXT.fetch_add(1, Ordering::Relaxed)
            ))
        }

        fn registration(&self) -> Registration<'_> {
            Registration { subkey: &self.0 }
        }
    }

    impl Drop for Fixture {
        fn drop(&mut self) {
            unsafe {
                RegDeleteKeyExW(
                    HKEY_CURRENT_USER,
                    wide(&self.0).as_ptr(),
                    KEY_WOW64_64KEY,
                    0,
                );
            }
        }
    }

    #[test]
    fn enable_roundtrips_unicode_and_spaces_and_disable_removes_only_typing() {
        let fixture = Fixture::new();
        let registration = fixture.registration();
        let executable = Path::new(r"C:\应用程序\Typing folder\Typing.exe");
        assert!(!registration.matches(executable).unwrap());
        registration.disable().unwrap();
        registration.enable(executable).unwrap();
        assert!(registration.matches(executable).unwrap());
        let key = registration
            .open(KEY_SET_VALUE | KEY_QUERY_VALUE, false)
            .unwrap()
            .unwrap();
        let mut saved = [0u16; MAX_COMMAND + 1];
        let mut size = std::mem::size_of_val(&saved) as u32;
        let mut kind = 0;
        assert_eq!(
            unsafe {
                RegQueryValueExW(
                    key.0,
                    wide(VALUE_NAME).as_ptr(),
                    std::ptr::null(),
                    &mut kind,
                    saved.as_mut_ptr().cast(),
                    &mut size,
                )
            },
            ERROR_SUCCESS
        );
        assert_eq!(kind, REG_SZ);
        assert_eq!(
            &saved[..size as usize / 2],
            wide("\"C:\\应用程序\\Typing folder\\Typing.exe\"")
        );
        let neighbor = wide("OtherApp");
        let bytes = wide("keep");
        assert_eq!(
            unsafe {
                RegSetValueExW(
                    key.0,
                    neighbor.as_ptr(),
                    0,
                    REG_SZ,
                    bytes.as_ptr().cast(),
                    (bytes.len() * 2) as u32,
                )
            },
            ERROR_SUCCESS
        );
        registration.disable().unwrap();
        registration.disable().unwrap();
        assert!(!registration.matches(executable).unwrap());
        let mut size = 0;
        assert_eq!(
            unsafe {
                RegQueryValueExW(
                    key.0,
                    wide(VALUE_NAME).as_ptr(),
                    std::ptr::null(),
                    std::ptr::null_mut(),
                    std::ptr::null_mut(),
                    &mut size,
                )
            },
            ERROR_FILE_NOT_FOUND
        );
        assert_eq!(
            unsafe {
                RegQueryValueExW(
                    key.0,
                    neighbor.as_ptr(),
                    std::ptr::null(),
                    std::ptr::null_mut(),
                    std::ptr::null_mut(),
                    &mut size,
                )
            },
            ERROR_SUCCESS
        );
    }

    #[test]
    fn moved_copy_replaces_stale_registration_and_can_cancel_it() {
        let fixture = Fixture::new();
        let registration = fixture.registration();
        let old = Path::new(r"C:\old\Typing.exe");
        let new = Path::new(r"D:\new\Typing.exe");
        registration.enable(old).unwrap();
        assert!(!registration.matches(new).unwrap());
        registration.enable(new).unwrap();
        assert!(registration.matches(new).unwrap());
        assert!(!registration.matches(old).unwrap());
        registration.disable().unwrap();
        assert!(!registration.matches(new).unwrap());
        registration.enable(old).unwrap();
        registration.disable().unwrap();
        assert!(!registration.matches(old).unwrap());
    }

    #[test]
    fn invalid_commands_do_not_overwrite_existing_registration() {
        let fixture = Fixture::new();
        let registration = fixture.registration();
        let valid = Path::new(r"C:\Typing\Typing.exe");
        registration.enable(valid).unwrap();
        for invalid in [
            "relative.exe".to_string(),
            "C:\\bad\"path.exe".to_string(),
            "C:\\bad\0path.exe".to_string(),
            format!("C:\\{}\\Typing.exe", "a".repeat(260)),
        ] {
            assert!(registration.enable(Path::new(&invalid)).is_err());
            assert!(registration.matches(valid).unwrap());
        }
        // Exercise Win32 error propagation with an invalid registry subkey.
        let invalid = Registration {
            subkey: "\\invalid",
        };
        assert!(invalid.enable(valid).is_err());
        assert!(invalid.matches(valid).is_err());
        assert!(invalid.disable().is_err());
    }

    #[test]
    fn command_length_limit_counts_quotes_and_preserves_exact_boundary() {
        let at_limit = format!("C:\\{}", "a".repeat(MAX_COMMAND - 5));
        assert_eq!(
            command(Path::new(&at_limit)).unwrap().len(),
            MAX_COMMAND + 1
        );
        assert!(command(Path::new(&(at_limit + "a"))).is_err());
    }
}
