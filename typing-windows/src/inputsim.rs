use std::{ffi::CString, path::Path};
use typing_core::{Message, msg};
use windows_sys::Win32::{
    Foundation::FreeLibrary,
    System::LibraryLoader::{
        GetModuleHandleW, GetProcAddress, LOAD_LIBRARY_SEARCH_DEFAULT_DIRS,
        LOAD_LIBRARY_SEARCH_DLL_LOAD_DIR, LOAD_WITH_ALTERED_SEARCH_PATH, LoadLibraryExW,
    },
};

pub const INSERTED: u32 = 0;
pub const RESOLVED: u32 = 4;
pub const DOWN: u32 = 1;
pub const UP: u32 = 2;

#[repr(C)]
#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub struct Event {
    pub kind: u32,
    pub code: u32,
    pub flags: u32,
    pub reserved: u32,
}

#[repr(C)]
#[derive(Clone, Copy, Debug, Default)]
pub struct Character {
    pub key: Event,
    pub modifiers: [Event; 3],
    pub modifier_count: u32,
}

pub struct InputSim {
    module: isize,
    key: unsafe extern "C" fn(u32, *mut Event) -> u32,
    character: unsafe extern "C" fn(u32, *mut Character) -> u32,
    submit: unsafe extern "C" fn(*const Event, u32, usize) -> u32,
}

impl InputSim {
    pub fn load(path: &Path) -> Result<Self, Message> {
        let absolute = path.canonicalize().map_err(|error| msg!("inputsim.cannot_locate_inputsim_dll_keep_the", "Cannot locate InputSim DLL {arg0}: {error}. Keep the matching inputsim.dll beside Typing.exe or specify --inputsim PATH.", arg0 = path.display(), error = error))?;
        let name: Vec<u16> = absolute
            .to_string_lossy()
            .encode_utf16()
            .chain(Some(0))
            .collect();
        let modern_search = unsafe {
            GetProcAddress(
                GetModuleHandleW("kernel32.dll\0".encode_utf16().collect::<Vec<_>>().as_ptr()),
                c"AddDllDirectory".as_ptr().cast(),
            )
        }
        .is_some();
        let module = unsafe {
            LoadLibraryExW(
                name.as_ptr(),
                0,
                if modern_search {
                    LOAD_LIBRARY_SEARCH_DLL_LOAD_DIR | LOAD_LIBRARY_SEARCH_DEFAULT_DIRS
                } else {
                    LOAD_WITH_ALTERED_SEARCH_PATH
                },
            )
        };
        if module == 0 {
            return Err(msg!(
                "inputsim.cannot_load_inputsim_dll_use_a",
                "Cannot load InputSim DLL {arg0}: {arg1}. Use a DLL matching the executable architecture.",
                arg0 = absolute.display(),
                arg1 = std::io::Error::last_os_error()
            ));
        }
        let symbol = |name: &str| {
            let c_name = CString::new(name).unwrap();
            unsafe { GetProcAddress(module, c_name.as_ptr().cast()) }
                .ok_or_else(|| msg!("inputsim.inputsim_dll_is_missing_required_v", "InputSim DLL {arg0} is missing required v1 export {name}. Use the matching packaged DLL.", arg0 = absolute.display(), name = name))
        };
        let loaded = (|| {
            Ok(Self {
                module,
                key: unsafe {
                    std::mem::transmute::<
                        unsafe extern "system" fn() -> isize,
                        unsafe extern "C" fn(u32, *mut Event) -> u32,
                    >(symbol("resolve_key_v1")?)
                },
                character: unsafe {
                    std::mem::transmute::<
                        unsafe extern "system" fn() -> isize,
                        unsafe extern "C" fn(u32, *mut Character) -> u32,
                    >(symbol("resolve_char_v1")?)
                },
                submit: unsafe {
                    std::mem::transmute::<
                        unsafe extern "system" fn() -> isize,
                        unsafe extern "C" fn(*const Event, u32, usize) -> u32,
                    >(symbol("submit_v1")?)
                },
            })
        })();
        if loaded.is_err() {
            unsafe {
                FreeLibrary(module);
            }
        }
        loaded
    }

    pub fn key(&self, key: u16) -> Result<Event, Message> {
        let mut value = Event::default();
        let result = unsafe { (self.key)(key as u32, &mut value) };
        if result == RESOLVED {
            Ok(value)
        } else {
            Err(msg!(
                "inputsim.resolve_key",
                "resolve key {key}: {result}",
                key = key,
                result = result
            ))
        }
    }

    pub fn character(&self, character: char) -> Result<Character, Message> {
        let mut value = Character::default();
        let result = unsafe { (self.character)(character as u32, &mut value) };
        if result == RESOLVED && value.modifier_count <= 3 {
            Ok(value)
        } else {
            Err(msg!(
                "inputsim.resolve_character",
                "resolve character: {result}",
                result = result
            ))
        }
    }

    pub fn submit(&self, event: Event, direction: u32, tag: usize) -> Result<(), Message> {
        let result = unsafe { (self.submit)(&event, direction, tag) };
        if result == INSERTED {
            Ok(())
        } else {
            Err(msg!(
                "inputsim.submit",
                "submit {direction}: {result}",
                direction = direction,
                result = result
            ))
        }
    }
}

impl Drop for InputSim {
    fn drop(&mut self) {
        unsafe {
            FreeLibrary(self.module);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn public_structures_match_inputsim_c_abi() {
        assert_eq!(std::mem::size_of::<Event>(), 16);
        assert_eq!(std::mem::align_of::<Event>(), 4);
        assert_eq!(std::mem::size_of::<Character>(), 68);
        assert_eq!(std::mem::align_of::<Character>(), 4);
        assert_eq!(
            std::mem::size_of::<usize>(),
            if cfg!(target_pointer_width = "64") {
                8
            } else {
                4
            }
        );
    }
}
