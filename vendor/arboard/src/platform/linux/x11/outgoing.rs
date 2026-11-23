// SPDX-License-Identifier: Apache-2.0 OR MIT
use super::*;
use x11rb::protocol::xproto::{ChangeWindowAttributesAux, Window};

struct Transfer {
	request: SelectionRequestEvent,
	data: Arc<ClipboardData>,
	offset: usize,
}

#[derive(Default)]
pub(super) struct Transfers(HashMap<(Window, Atom), Transfer>);

impl Transfers {
	fn chunk_size(ctx: &Inner) -> usize {
		// Use the core request limit, leaving space for the ChangeProperty header.
		usize::from(ctx.server.conn.setup().maximum_request_length) * 4 - 24
	}

	pub(super) fn pending(&self, request: &SelectionRequestEvent) -> bool {
		self.0.contains_key(&(request.requestor, request.property))
	}

	pub(super) fn send(
		&mut self,
		ctx: &Inner,
		request: SelectionRequestEvent,
		data: &Arc<ClipboardData>,
	) -> Result<()> {
		let conn = &ctx.server.conn;
		let window = request.requestor;
		if data.bytes.len() <= Self::chunk_size(ctx) {
			conn.change_property8(
				PropMode::REPLACE,
				window,
				request.property,
				request.target,
				&data.bytes,
			)
			.map_err(into_unknown)?
			.check()
			.map_err(into_unknown)?;
			return Ok(());
		}

		let mask = EventMask::PROPERTY_CHANGE | EventMask::STRUCTURE_NOTIFY;
		conn.change_window_attributes(window, &ChangeWindowAttributesAux::new().event_mask(mask))
			.map_err(into_unknown)?
			.check()
			.map_err(into_unknown)?;
		let len = u32::try_from(data.bytes.len()).unwrap_or(u32::MAX);
		conn.change_property32(PropMode::REPLACE, window, request.property, ctx.atoms.INCR, &[len])
			.map_err(into_unknown)?
			.check()
			.map_err(into_unknown)?;
		self.0.insert(
			(window, request.property),
			Transfer { request, data: data.clone(), offset: 0 },
		);
		Ok(())
	}

	pub(super) fn advance(
		&mut self,
		ctx: &Inner,
		event: PropertyNotifyEvent,
	) -> Result<Option<SelectionRequestEvent>> {
		let Entry::Occupied(mut entry) = self.0.entry((event.window, event.atom)) else {
			return Ok(None);
		};
		let transfer = entry.get_mut();
		let end = transfer.data.bytes.len().min(transfer.offset + Self::chunk_size(ctx));
		let bytes = &transfer.data.bytes[transfer.offset..end];
		let conn = &ctx.server.conn;
		conn.change_property8(
			PropMode::APPEND,
			event.window,
			event.atom,
			transfer.request.target,
			bytes,
		)
		.map_err(into_unknown)?;
		transfer.offset = end;
		let completed = if bytes.is_empty() {
			let request = entry.remove().request;
			if event.window != ctx.server.win_id
				&& !self.0.keys().any(|(window, _)| *window == event.window)
			{
				conn.change_window_attributes(
					event.window,
					&ChangeWindowAttributesAux::new().event_mask(EventMask::NO_EVENT),
				)
				.map_err(into_unknown)?;
			}
			Some(request)
		} else {
			None
		};
		conn.flush().map_err(into_unknown)?;
		Ok(completed)
	}

	pub(super) fn forget_window(&mut self, window: Window) {
		self.0.retain(|(requestor, _), _| *requestor != window);
	}
}
