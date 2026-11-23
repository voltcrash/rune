//! EmptyClipboard assigns ownership to the HWND used to open it. Keeping that window alive
//! identifies this write even when another process replaces it with exactly the same text.
#![allow(unsafe_code)]

use std::{ptr, sync::mpsc};
use windows_sys::Win32::{Foundation::HWND, System::DataExchange::GetClipboardOwner, UI::WindowsAndMessaging::*};

pub(super) struct Source(usize);

impl Source {
    pub(super) fn lost(&self) -> bool {
        // SAFETY: GetClipboardOwner takes no pointers.
        unsafe { GetClipboardOwner() as usize != self.0 }
    }
}

impl Drop for Source {
    fn drop(&mut self) {
        // SAFETY: The source thread owns this HWND until it receives this message or exits.
        unsafe { PostMessageW(self.0 as HWND, WM_APP, 0, 0); }
    }
}

pub(super) fn write(text: &str) -> Result<Source, String> {
    let (ready, acquired) = mpsc::sync_channel(1);
    let text = text.to_owned();
    std::thread::Builder::new().name("rune-clipboard-source".into()).spawn(move || {
        // SAFETY: STATIC is a system window class. This thread creates, pumps and destroys
        // the message-only window, including while other clipboard clients notify its owner.
        unsafe {
            let hwnd = CreateWindowExW(0, windows_sys::core::w!("STATIC"), ptr::null(), 0, 0, 0, 0, 0, HWND_MESSAGE, ptr::null_mut(), ptr::null_mut(), ptr::null());
            if hwnd.is_null() {
                let _ = ready.send(Err(std::io::Error::last_os_error().to_string()));
                return;
            }
            let written = clipboard_win::Clipboard::new_attempts_for(hwnd.cast(), 10).and_then(|_open| clipboard_win::raw::set_string(&text));
            drop(text);
            match written {
                Ok(()) if ready.send(Ok(Source(hwnd as usize))).is_ok() => {
                    let mut message = std::mem::zeroed();
                    while GetMessageW(&mut message, ptr::null_mut(), 0, 0) > 0 {
                        if message.hwnd == hwnd && message.message == WM_APP { break; }
                        DispatchMessageW(&message);
                    }
                }
                Err(e) => { let _ = ready.send(Err(e.to_string())); }
                Ok(()) => {}
            }
            DestroyWindow(hwnd);
        }
    }).map_err(|e| e.to_string())?;
    acquired.recv().map_err(|e| e.to_string())?
}
