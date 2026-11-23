//! A resource-bearing text offer has an identifiable native source for its entire lifetime.
use std::sync::mpsc;
use wl_clipboard_rs::copy::{ClipboardType, MimeType, Options, Source};
use x11rb::protocol::xproto::ConnectionExt;

pub(super) enum Selection {
    Wayland(mpsc::Receiver<()>),
    X11 { clipboard: x11_clipboard::Clipboard, atom: u32 },
}

impl Selection {
    pub(super) fn lost(&self) -> bool {
        match self {
            Self::Wayland(done) => !matches!(done.try_recv(), Err(mpsc::TryRecvError::Empty)),
            Self::X11 { clipboard, atom } => {
                let setter = &clipboard.setter;
                setter.connection.get_selection_owner(*atom).ok()
                    .and_then(|cookie| cookie.reply().ok())
                    .is_some_and(|reply| reply.owner != setter.window)
            }
        }
    }
}

pub(super) fn write(text: &str, primary: bool) -> Result<Selection, String> {
    if std::env::var_os("WAYLAND_DISPLAY").is_some() && wl_clipboard_rs::utils::is_primary_selection_supported().is_ok() {
        return wayland(text.to_owned(), primary);
    }
    let clipboard = x11_clipboard::Clipboard::new().map_err(|e| e.to_string())?;
    let atoms = &clipboard.setter.atoms;
    let atom = if primary { atoms.primary } else { atoms.clipboard };
    clipboard.store(atom, atoms.utf8_string, text.as_bytes()).map_err(|e| e.to_string())?;
    Ok(Selection::X11 { clipboard, atom })
}

fn wayland(text: String, primary: bool) -> Result<Selection, String> {
    let (ready, acquired) = mpsc::sync_channel(1);
    let (done, cancelled) = mpsc::channel();
    std::thread::Builder::new().name("rune-clipboard-source".into()).spawn(move || {
        let mut options = Options::new();
        options.foreground(true).clipboard(if primary { ClipboardType::Primary } else { ClipboardType::Regular });
        let source = Source::Bytes(text.into_bytes().into_boxed_slice());
        match options.prepare_copy(source, MimeType::Text) {
            Ok(offer) => {
                if ready.send(Ok(())).is_ok() { let _ = offer.serve(); }
            }
            Err(e) => { let _ = ready.send(Err(e.to_string())); }
        }
        drop(done);
    }).map_err(|e| e.to_string())?;
    acquired.recv().map_err(|e| e.to_string())??;
    Ok(Selection::Wayland(cancelled))
}
