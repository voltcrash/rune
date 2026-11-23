// SPDX-License-Identifier: Apache-2.0 OR MIT
use super::*;
use x11rb::protocol::xproto::{ChangeWindowAttributesAux, ClientMessageEvent, Window};

struct Transfer {
	request: SelectionRequestEvent,
	data: Arc<ClipboardData>,
	offset: usize,
	deadline: Instant,
}

#[derive(Default)]
pub(super) struct Transfers {
	entries: HashMap<(Window, Atom), Transfer>,
	alarm: Option<Arc<AtomicBool>>,
}

const TRANSFER_IDLE: Duration = Duration::from_secs(5);

impl Transfers {
	fn chunk_size(ctx: &Inner) -> usize {
		// Use the core request limit, leaving space for the ChangeProperty header.
		usize::from(ctx.server.conn.setup().maximum_request_length) * 4 - 24
	}

	pub(super) fn pending(&self, request: &SelectionRequestEvent) -> bool {
		self.entries.contains_key(&(request.requestor, request.property))
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
		let header = conn
			.change_property32(PropMode::REPLACE, window, request.property, ctx.atoms.INCR, &[len])
			.map_err(into_unknown)?
			.check()
			.map_err(into_unknown);
		if let Err(error) = header {
			self.unsubscribe_unused(ctx, window)?;
			return Err(error);
		}
		self.entries.insert(
			(window, request.property),
			Transfer {
				request,
				data: data.clone(),
				offset: 0,
				deadline: Instant::now() + TRANSFER_IDLE,
			},
		);
		if let Err(error) = self.arm(ctx) {
			self.cancel(ctx, &request)?;
			return Err(error);
		}
		Ok(())
	}

	pub(super) fn cancel(&mut self, ctx: &Inner, request: &SelectionRequestEvent) -> Result<()> {
		if self.entries.remove(&(request.requestor, request.property)).is_some() {
			self.unsubscribe_unused(ctx, request.requestor)?;
		}
		Ok(())
	}

	fn arm(&mut self, ctx: &Inner) -> Result<()> {
		if self.alarm.is_some() {
			return Ok(());
		}
		let Some(deadline) = self.entries.values().map(|entry| entry.deadline).min() else {
			return Ok(());
		};
		let conn = Arc::downgrade(&ctx.server.conn);
		let fired = Arc::new(AtomicBool::new(false));
		let wake_ready = fired.clone();
		let window = ctx.server.win_id;
		let atom = ctx.atoms.ARBOARD_INCR_TIMEOUT;
		// Wake wait_for_event even if a still-mounted requestor stops reading.
		std::thread::Builder::new()
			.name("arboard-incr-timeout".into())
			.spawn(move || {
				std::thread::sleep(deadline.saturating_duration_since(Instant::now()));
				if let Some(conn) = conn.upgrade() {
					wake_ready.store(true, Ordering::Release);
					let wake = ClientMessageEvent::new(32, window, atom, [0; 5]);
					let _ = conn.send_event(false, window, EventMask::NO_EVENT, wake);
					let _ = conn.flush();
				}
			})
			.map_err(into_unknown)?;
		self.alarm = Some(fired);
		Ok(())
	}

	fn take_fired_alarm(&mut self) -> bool {
		if self.alarm.as_ref().is_some_and(|fired| fired.load(Ordering::Acquire)) {
			self.alarm = None;
			true
		} else {
			false
		}
	}

	pub(super) fn expire(&mut self, ctx: &Inner) -> Result<()> {
		if !self.take_fired_alarm() {
			return Ok(());
		}
		let now = Instant::now();
		let mut expired = Vec::new();
		self.entries.retain(|&(window, _), entry| {
			if entry.deadline > now {
				return true;
			}
			expired.push(window);
			false
		});
		expired.sort_unstable();
		expired.dedup();
		for window in expired {
			self.unsubscribe_unused(ctx, window)?;
		}
		if let Err(error) = self.arm(ctx) {
			warn!("Could not rearm incremental clipboard transfers: {error}");
			while let Some(request) = self.entries.values().next().map(|entry| entry.request) {
				self.cancel(ctx, &request)?;
			}
		}
		ctx.server.conn.flush().map_err(into_unknown)
	}

	fn unsubscribe_unused(&self, ctx: &Inner, window: Window) -> Result<()> {
		if window != ctx.server.win_id && !self.entries.keys().any(|(w, _)| *w == window) {
			ctx.server
				.conn
				.change_window_attributes(
					window,
					&ChangeWindowAttributesAux::new().event_mask(EventMask::NO_EVENT),
				)
				.map_err(into_unknown)?;
		}
		Ok(())
	}

	pub(super) fn advance(
		&mut self,
		ctx: &Inner,
		event: PropertyNotifyEvent,
	) -> Result<Option<SelectionRequestEvent>> {
		let Entry::Occupied(mut entry) = self.entries.entry((event.window, event.atom)) else {
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
		transfer.deadline = Instant::now() + TRANSFER_IDLE;
		let completed = if bytes.is_empty() {
			let request = entry.remove().request;
			self.unsubscribe_unused(ctx, event.window)?;
			Some(request)
		} else {
			None
		};
		conn.flush().map_err(into_unknown)?;
		Ok(completed)
	}

	pub(super) fn forget_window(&mut self, window: Window) {
		self.entries.retain(|(requestor, _), _| *requestor != window);
	}
}

#[cfg(test)]
mod tests {
	use super::*;

	#[test]
	fn only_the_current_fired_alarm_can_rearm_the_transfer_timer() {
		let first = Arc::new(AtomicBool::new(false));
		let mut transfers = Transfers { alarm: Some(first.clone()), ..Transfers::default() };
		assert!(!transfers.take_fired_alarm());
		first.store(true, Ordering::Release);
		assert!(transfers.take_fired_alarm());
		assert!(!transfers.take_fired_alarm());
		let next = Arc::new(AtomicBool::new(false));
		transfers.alarm = Some(next.clone());
		assert!(!transfers.take_fired_alarm());
		next.store(true, Ordering::Release);
		assert!(transfers.take_fired_alarm());
	}
}
