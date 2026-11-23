//! Linux's primary selection, as Chromium keeps it on X11 and Wayland: the text a pointer
//! selects (a drag, a double or triple click, Shift+click), in selectable text or a text field,
//! goes on the primary selection when the button comes up; a middle click in a text field puts
//! the caret where it landed and pastes the primary selection there. A secure field's text
//! never goes on it. The clipboard's worker beside it writes and reads it
//! (`clipboard/primary.rs`), so Cmd/Ctrl+C and Cmd/Ctrl+V never see it.
use std::hash::BuildHasher;

use super::*;

/// What the runtime asks of the primary selection, for the clipboard's flush to take.
#[derive(Default)]
pub(crate) struct PrimarySelection {
    /// A primary button came up: after this frame's handlers, what's selected goes on it.
    pending: bool,
    /// A hash of the text last put on it, so a click that leaves the selection as it was
    /// writes nothing.
    written: Option<u64>,
    /// Text to put on it.
    pub(crate) write: Option<String>,
    /// The text field a middle click pastes it into.
    pub(crate) paste: Option<String>,
}

pub(in crate::runtime) enum PrimarySource {
    Field(String),
    Text(Option<String>),
}

impl RuntimeCore {
    /// The primary button came up.
    pub(in crate::runtime) fn primary_released(&mut self) {
        self.edit.primary.pending = true;
    }

    /// Unmounted rows must be read while their builder and the application model agree.
    pub(in crate::runtime) fn primary_text_before_handlers(&self, app: &dyn ViewDriver) -> Option<String> {
        if !self.edit.primary.pending || self.commit.dirty { return None; }
        self.selected_text(app)
    }

    /// Holds the selection's source before a release handler's requested focus takes effect.
    pub(in crate::runtime) fn take_primary_source(&mut self, text: Option<String>) -> Option<PrimarySource> {
        if !std::mem::take(&mut self.edit.primary.pending) { return None; }
        if let Some(path) = self.kbd.focus.path() && self.input_at(path).is_some() {
            return Some(PrimarySource::Field(path.to_owned()));
        }
        Some(PrimarySource::Text(text))
    }

    pub(in crate::runtime) fn note_primary_selection(&mut self, app: &dyn ViewDriver, source: PrimarySource) {
        let text = match source {
            PrimarySource::Field(path) => {
                let Some(input) = self.input_at(&path) else { return };
                let edit = &input.edit;
                let (start, end) = edit.range();
                if edit.is_secure() || start == end { return; }
                input.copy_text.as_ref().and_then(|f| f(edit, crate::CopyDestination::PrimarySelection))
                    .or_else(|| Some(edit.selected().to_owned())).filter(|t| !t.is_empty())
            }
            PrimarySource::Text(text) => text.or_else(|| self.selected_text(app)),
        };
        let Some(text) = text else { return };
        let hash = std::hash::BuildHasherDefault::<std::hash::DefaultHasher>::default().hash_one(&text);
        if self.edit.primary.written == Some(hash) { return; }
        self.edit.primary.written = Some(hash);
        self.edit.primary.write = Some(text);
    }

    /// A middle press at (x, y): on a text field that takes text, it focuses the field with its
    /// caret there and asks for the primary selection to paste. True when it did.
    pub(in crate::runtime) fn primary_paste_down(&mut self, x: f32, y: f32) -> bool {
        let Some(i) = self.primary_target(x, y).filter(|&i| self.allowed(i) && !self.subtree_exiting(i)) else { return false };
        let takes = self.tree.nodes[i].input.as_ref().is_some_and(|input| input.access.is_some() && !input.edit.is_secure());
        if !takes { return false; }
        self.place_caret(i, x, y);
        self.edit.primary.paste = Some(self.tree.nodes[i].path.clone());
        true
    }
}
