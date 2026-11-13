//! Embedded Help artwork, decoded once with Windows and scaled by native GDI.
use super::*;
use std::ffi::c_void;
use windows_sys::Win32::{
    Graphics::GdiPlus::*, System::SystemServices::SS_OWNERDRAW, UI::Shell::SHCreateMemStream,
};

const PNG: &[u8] = include_bytes!("../../docs/assets/command-mode-movement.png");
const WIDTH: i32 = 1140;
const HEIGHT: i32 = 930;

pub fn height(width: i32) -> i32 {
    ((i64::from(width) * i64::from(HEIGHT) + i64::from(WIDTH) / 2) / i64::from(WIDTH)).max(1) as i32
}

struct Decoder {
    token: usize,
    stream: *mut c_void,
    image: *mut GpImage,
}

impl Drop for Decoder {
    fn drop(&mut self) {
        unsafe {
            if !self.image.is_null() {
                GdipDisposeImage(self.image);
            }
            if !self.stream.is_null() {
                // IStream inherits IUnknown's first three vtable entries.
                #[repr(C)]
                struct UnknownVtable {
                    query_interface: usize,
                    add_ref: usize,
                    release: unsafe extern "system" fn(*mut c_void) -> u32,
                }
                let table = *(self.stream as *const *const UnknownVtable);
                ((*table).release)(self.stream);
            }
            if self.token != 0 {
                GdiplusShutdown(self.token);
            }
        }
    }
}

struct Picture(HBITMAP);

impl Drop for Picture {
    fn drop(&mut self) {
        unsafe { DeleteObject(self.0) };
    }
}

fn decode() -> Result<Picture, Message> {
    let failure = || {
        msg!(
            "windows.movement_picture_failed",
            "Unable to load the default cursor movement picture."
        )
    };
    let mut decoder = Decoder {
        token: 0,
        stream: std::ptr::null_mut(),
        image: std::ptr::null_mut(),
    };
    let input = GdiplusStartupInput {
        GdiplusVersion: 1,
        DebugEventCallback: 0,
        SuppressBackgroundThread: 0,
        SuppressExternalCodecs: 1,
    };
    unsafe {
        if GdiplusStartup(&mut decoder.token, &input, std::ptr::null_mut()) != Ok {
            return Err(failure());
        }
        decoder.stream = SHCreateMemStream(PNG.as_ptr(), PNG.len() as u32);
        if decoder.stream.is_null()
            || GdipLoadImageFromStream(decoder.stream, &mut decoder.image) != Ok
        {
            return Err(failure());
        }
        let (mut width, mut height) = (0, 0);
        if GdipGetImageWidth(decoder.image, &mut width) != Ok
            || GdipGetImageHeight(decoder.image, &mut height) != Ok
            || width != WIDTH as u32
            || height != HEIGHT as u32
        {
            return Err(failure());
        }
        let mut bitmap = 0;
        if GdipCreateHBITMAPFromBitmap(decoder.image, &mut bitmap, 0xffffffff) != Ok || bitmap == 0
        {
            return Err(failure());
        }
        Result::Ok(Picture(bitmap))
    }
}

pub fn create(parent: HWND, id: usize) -> Result<HWND, Message> {
    let picture = Box::new(decode()?);
    let window = child(
        parent,
        "STATIC",
        msg!(
            "ui.default_cursor_movement_description",
            "Default profile: U Home, I Up, O End, P one word right; H one word left, J Left, K Down, L Right; N Page Up, M Page Down."
        ),
        SS_OWNERDRAW,
        id,
    )?;
    let picture = Box::into_raw(picture);
    if unsafe { SetWindowSubclass(window, Some(picture_proc), 1, picture as usize) } == 0 {
        unsafe {
            drop(Box::from_raw(picture));
            DestroyWindow(window);
        }
        return Err(last_error(&msg!(
            "windows.operation.create_control",
            "create control"
        )));
    }
    Result::Ok(window)
}

unsafe fn paint(window: HWND, dc: HDC, picture: &Picture) {
    unsafe {
        let mut bounds: RECT = std::mem::zeroed();
        GetClientRect(window, &mut bounds);
        let source = CreateCompatibleDC(dc);
        if source != 0 {
            let old = SelectObject(source, picture.0);
            let mode = SetStretchBltMode(dc, HALFTONE);
            let mut origin: POINT = std::mem::zeroed();
            SetBrushOrgEx(dc, 0, 0, &mut origin);
            StretchBlt(
                dc,
                0,
                0,
                bounds.right,
                bounds.bottom,
                source,
                0,
                0,
                WIDTH,
                HEIGHT,
                SRCCOPY,
            );
            SetBrushOrgEx(dc, origin.x, origin.y, std::ptr::null_mut());
            SetStretchBltMode(dc, mode);
            SelectObject(source, old);
            DeleteDC(source);
        }
    }
}

unsafe extern "system" fn picture_proc(
    window: HWND,
    message: u32,
    w: WPARAM,
    l: LPARAM,
    id: usize,
    data: usize,
) -> LRESULT {
    unsafe {
        let picture = &*(data as *const Picture);
        match message {
            WM_PAINT => {
                let mut state = std::mem::zeroed();
                let dc = BeginPaint(window, &mut state);
                paint(window, dc, picture);
                EndPaint(window, &state);
                0
            }
            WM_PRINT | WM_PRINTCLIENT => {
                paint(window, w as HDC, picture);
                0
            }
            WM_ERASEBKGND => 1,
            WM_SIZE => {
                InvalidateRect(window, std::ptr::null(), 0);
                DefSubclassProc(window, message, w, l)
            }
            WM_MOUSEWHEEL => SendMessageW(GetParent(window), message, w, l),
            WM_NCDESTROY => {
                RemoveWindowSubclass(window, Some(picture_proc), id);
                drop(Box::from_raw(data as *mut Picture));
                DefSubclassProc(window, message, w, l)
            }
            _ => DefSubclassProc(window, message, w, l),
        }
    }
}
