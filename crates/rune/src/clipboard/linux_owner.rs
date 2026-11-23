//! Ownership of resource-bearing text on Linux. Each selection has its own source.
use arboard::LinuxClipboardKind;
use std::sync::{Arc, Weak, mpsc};

#[path = "wayland_owner.rs"]
mod wayland;

pub(super) struct Selection {
    token: Weak<()>,
    _wayland: Option<calloop::channel::Sender<()>>,
}

impl Selection {
    pub(super) fn lost(&self) -> bool {
        self.token.strong_count() == 0
    }
}

pub(super) fn write(
    board: &mut arboard::Clipboard,
    text: &str,
    primary: bool,
) -> Result<Selection, String> {
    let token = Arc::new(());
    let owner = Arc::downgrade(&token);
    let kind = if primary {
        LinuxClipboardKind::Primary
    } else {
        LinuxClipboardKind::Clipboard
    };
    if board
        .set_x11_text_with_owner(kind, text, token.clone())
        .map_err(|e| e.to_string())?
    {
        return Ok(Selection {
            token: owner,
            _wayland: None,
        });
    }
    let (ready, acquired) = mpsc::sync_channel(1);
    let (shutdown, close) = calloop::channel::channel::<()>();
    let text = text.to_owned();
    std::thread::Builder::new()
        .name("rune-clipboard-source".into())
        .spawn(move || {
            if let Err(e) = wayland::serve(text, primary, token, close, &ready) {
                let _ = ready.try_send(Err(e.to_string()));
            }
        })
        .map_err(|e| e.to_string())?;
    acquired.recv().map_err(|e| e.to_string())??;
    Ok(Selection {
        token: owner,
        _wayland: Some(shutdown),
    })
}
