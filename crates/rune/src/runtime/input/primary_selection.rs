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
    text: Option<String>,
    /// A hash of the text last put on it, so a click that leaves the selection as it was
    /// writes nothing.
    written: Option<u64>,
    /// Text to put on it.
    pub(crate) write: Option<crate::ClipboardWrite>,
    /// The text field a middle click pastes it into.
    pub(crate) paste: Option<String>,
}

pub(in crate::runtime) enum PrimarySource {
    Field(String),
    Text(Option<String>),
}

impl RuntimeCore {
    /// Text selected by a pointer is final on release, before callbacks can change virtual rows.
    pub(in crate::runtime) fn primary_released(&mut self, app: &dyn ViewDriver) {
        self.edit.primary.pending = true;
        self.edit.primary.text = if self.commit.dirty && self.selection_spans_unmounted() {
            self.edit.cached_selection_snapshot(self.commit.commits).flatten().map(|s| s.text().to_owned())
        } else {
            self.selected_text(app)
        };
    }

    /// Holds the selection's source before a release handler's requested focus takes effect.
    pub(in crate::runtime) fn take_primary_source(&mut self) -> Option<PrimarySource> {
        if !std::mem::take(&mut self.edit.primary.pending) { return None; }
        let text = self.edit.primary.text.take();
        if let Some(path) = self.kbd.focus.path() && self.input_at(path).is_some() {
            return Some(PrimarySource::Field(path.to_owned()));
        }
        Some(PrimarySource::Text(text))
    }

    pub(in crate::runtime) fn note_primary_selection(&mut self, app: &dyn ViewDriver, source: PrimarySource) {
        let write = match source {
            PrimarySource::Field(path) => {
                let Some(input) = self.input_at(&path) else { return };
                let edit = &input.edit;
                let (start, end) = edit.range();
                if edit.is_secure() || start == end { return; }
                input.copy_text.as_ref().and_then(|f| f(edit, crate::CopyDestination::PrimarySelection))
                    .or_else(|| Some(crate::ClipboardWrite::text(edit.selected()))).filter(|w| !w.is_empty())
            }
            PrimarySource::Text(text) => text.or_else(|| self.selected_text(app)).map(crate::ClipboardWrite::text),
        };
        let Some(write) = write else { return };
        let hash = std::hash::BuildHasherDefault::<std::hash::DefaultHasher>::default().hash_one(write.content().text());
        if !write.has_resource() && self.edit.primary.written == Some(hash) { return; }
        self.edit.primary.written = Some(hash);
        self.edit.primary.write = Some(write);
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
