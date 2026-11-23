//! Linux's primary selection on the OS (X11 `PRIMARY`, Wayland's primary selection through
//! data control): its own worker over arboard, beside the clipboard's, so a primary read never
//! waits behind a clipboard read and a secret's hold never sees it. What goes on it and where a
//! middle click pastes it is the runtime's (`runtime/input/primary_selection.rs`).
use arboard::{ClearExtLinux, GetExtLinux, LinuxClipboardKind::Primary, SetExtLinux};

use super::*;

struct PrimaryBoard(arboard::Clipboard);

impl Backend for PrimaryBoard {
    fn write(&mut self, copy: &ClipboardWrite) -> Result<(), String> {
        self.0.set().clipboard(Primary).text(copy.content().text()).map_err(|e| e.to_string())
    }
    fn text(&mut self) -> Option<Zeroizing<String>> {
        self.0.get().clipboard(Primary).text().ok().filter(|s| !s.is_empty()).map(Zeroizing::new)
    }
    fn revision(&self) -> Option<u64> { None }
    fn clear(&mut self) -> Result<(), String> {
        self.0.clear_with().clipboard(Primary).map_err(|e| e.to_string())
    }
}

static WORKER: OnceLock<native::Worker> = OnceLock::new();

fn worker() -> &'static mpsc::Sender<Request> {
    if cfg!(test) {
        panic!("unit test reached the OS primary selection; install a test clipboard (`rt.use_clipboard` or `Client::use_memory`)");
    }
    &WORKER.get_or_init(|| native::Worker::spawn(|| arboard::Clipboard::new().ok().map(PrimaryBoard))).sender
}

pub(super) fn shutdown() {
    if let Some(worker) = WORKER.get() { worker.stop(); }
}

impl Client {
    /// Hands the worker what the runtime asked of the primary selection.
    pub(super) fn flush_primary(&self, rt: &mut crate::runtime::RuntimeCore) {
        if let Some(write) = rt.edit.primary.write.take() { worker().send(Request::Write(write, None)).ok(); }
        if let Some(target) = rt.edit.primary.paste.take() {
            let purpose = Purpose::Paste(Some(target), Forms::Text);
            worker().send(Request::Read { purpose, reply: self.sender.clone(), wake: self.wake.clone() }).ok();
        }
    }
}
