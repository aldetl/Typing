//! Checked native capture replacement. A rejected removal retains its handle.
use super::*;

pub fn replace_hook(handle: &mut HHOOK, enable: bool) -> Result<(), Message> {
    replace_hook_with(
        handle,
        enable,
        |hook| {
            if unsafe { UnhookWindowsHookEx(hook) } != 0 {
                Ok(())
            } else {
                Err(unsafe { GetLastError() })
            }
        },
        || {
            let hook = unsafe {
                SetWindowsHookExW(
                    WH_KEYBOARD_LL,
                    Some(keyboard_hook),
                    GetModuleHandleW(std::ptr::null()),
                    0,
                )
            };
            if hook != 0 {
                Ok(hook)
            } else {
                Err(unsafe { GetLastError() })
            }
        },
    )
}

fn replace_hook_with(
    handle: &mut HHOOK,
    enable: bool,
    mut remove: impl FnMut(HHOOK) -> Result<(), u32>,
    mut install: impl FnMut() -> Result<HHOOK, u32>,
) -> Result<(), Message> {
    if *handle != 0 {
        match remove(*handle) {
            Ok(()) | Err(ERROR_INVALID_HOOK_HANDLE) => *handle = 0,
            Err(code) => {
                return Err(msg!(
                    "desktop.remove_hook",
                    "remove hook: {arg0}",
                    arg0 = std::io::Error::from_raw_os_error(code as i32)
                ));
            }
        }
    }
    if enable {
        *handle = install().map_err(|code| {
            msg!(
                "desktop.install_hook",
                "install hook: {arg0}",
                arg0 = std::io::Error::from_raw_os_error(code as i32)
            )
        })?;
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn rejected_removal_retains_ownership_and_does_not_install() {
        let mut handle = 42;
        let result = replace_hook_with(
            &mut handle,
            true,
            |_| Err(ERROR_ACCESS_DENIED),
            || panic!("must not install"),
        );
        assert!(result.unwrap_err().starts_with("remove hook:"));
        assert_eq!(handle, 42);
        replace_hook_with(&mut handle, false, |_| Ok(()), || panic!("disabled")).unwrap();
        assert_eq!(handle, 0);
    }
    #[test]
    fn stale_handle_is_replaced_and_failed_install_can_be_explicitly_retried() {
        let mut handle = 42;
        assert!(
            replace_hook_with(
                &mut handle,
                true,
                |_| Err(ERROR_INVALID_HOOK_HANDLE),
                || Err(ERROR_ACCESS_DENIED)
            )
            .is_err()
        );
        assert_eq!(handle, 0);
        replace_hook_with(&mut handle, true, |_| panic!("absent"), || Ok(43)).unwrap();
        assert_eq!(handle, 43);
    }
    #[test]
    fn enabled_restart_replaces_even_a_cached_active_hook() {
        let mut handle = 42;
        let removed = std::cell::Cell::new(false);
        replace_hook_with(
            &mut handle,
            true,
            |old| {
                assert_eq!(old, 42);
                removed.set(true);
                Ok(())
            },
            || {
                assert!(removed.get());
                Ok(43)
            },
        )
        .unwrap();
        assert_eq!(handle, 43);
    }
}
