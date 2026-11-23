//! Publish on all seats and service transfer pipes independently of source cancellation.
use calloop::{EventLoop, Interest, LoopHandle, Mode, PostAction, generic::Generic};
use calloop_wayland_source::WaylandSource;
use rustix::fs::{OFlags, fcntl_getfl, fcntl_setfl};
use std::{
    error::Error,
    fs::File,
    io::{self, Write},
    os::fd::OwnedFd,
    rc::Rc,
    sync::{Arc, mpsc::SyncSender},
    time::{Duration, Instant},
};
use wayland_client::{
    Connection, Dispatch, Proxy, QueueHandle, delegate_noop, event_created_child,
    globals::{GlobalListContents, registry_queue_init},
    protocol::{wl_registry::WlRegistry, wl_seat::WlSeat},
};
use wayland_protocols::ext::data_control::v1::client::{
    ext_data_control_device_v1 as ext_device, ext_data_control_manager_v1::ExtDataControlManagerV1,
    ext_data_control_offer_v1::ExtDataControlOfferV1, ext_data_control_source_v1 as ext_source,
};
use wayland_protocols_wlr::data_control::v1::client::{
    zwlr_data_control_device_v1 as wlr_device,
    zwlr_data_control_manager_v1::ZwlrDataControlManagerV1,
    zwlr_data_control_offer_v1::ZwlrDataControlOfferV1, zwlr_data_control_source_v1 as wlr_source,
};

const TEXT_TYPES: &[&str] = &[
    "text/plain;charset=utf-8",
    "text/plain",
    "STRING",
    "UTF8_STRING",
    "TEXT",
];

#[derive(Default)]
struct Device {
    finished: bool,
    primary_supported: bool,
    publishing: bool,
}

struct State {
    handle: LoopHandle<'static, Self>,
    text: Rc<[u8]>,
    devices: Vec<Device>,
    transfers: usize,
    transferred: usize,
    owner: Option<Arc<()>>,
    closed: bool,
}

impl State {
    fn stop_publishing(&mut self, seat: usize) {
        if std::mem::take(&mut self.devices[seat].publishing)
            && !self.devices.iter().any(|device| device.publishing)
        {
            self.owner = None;
        }
    }

    fn send(&mut self, fd: OwnedFd) {
        let Ok(flags) = fcntl_getfl(&fd) else { return };
        if fcntl_setfl(&fd, flags | OFlags::NONBLOCK).is_err() {
            return;
        }
        let data = self.text.clone();
        let mut offset = 0;
        let sink = Generic::new(File::from(fd), Interest::WRITE, Mode::Level);
        if self
            .handle
            .insert_source(sink, move |_, file, state| {
                match file.as_ref().write(&data[offset..]) {
                    Ok(written) if written > 0 => {
                        state.transferred = state.transferred.wrapping_add(written);
                        offset += written;
                        if offset < data.len() {
                            return Ok(PostAction::Continue);
                        }
                    }
                    Err(e)
                        if matches!(
                            e.kind(),
                            io::ErrorKind::Interrupted | io::ErrorKind::WouldBlock
                        ) =>
                    {
                        return Ok(PostAction::Continue);
                    }
                    _ => {}
                }
                state.transfers -= 1;
                Ok(PostAction::Remove)
            })
            .is_ok()
        {
            self.transfers += 1;
        }
    }
}

delegate_noop!(State: ignore WlSeat);
delegate_noop!(State: ignore ExtDataControlManagerV1);
delegate_noop!(State: ignore ZwlrDataControlManagerV1);
delegate_noop!(State: ignore ExtDataControlOfferV1);
delegate_noop!(State: ignore ZwlrDataControlOfferV1);

impl Dispatch<WlRegistry, GlobalListContents> for State {
    fn event(
        _: &mut Self,
        _: &WlRegistry,
        _: <WlRegistry as Proxy>::Event,
        _: &GlobalListContents,
        _: &Connection,
        _: &QueueHandle<Self>,
    ) {
    }
}

macro_rules! dispatch_data_control {
    ($device:ty, $device_mod:ident, $offer:ty, $source:ty, $source_mod:ident) => {
        impl Dispatch<$device, usize> for State {
            fn event(state: &mut Self, device: &$device, event: <$device as Proxy>::Event, seat: &usize, _: &Connection, _: &QueueHandle<Self>) {
                match event {
                    $device_mod::Event::DataOffer { id } => id.destroy(),
                    $device_mod::Event::PrimarySelection { .. } => state.devices[*seat].primary_supported = true,
                    $device_mod::Event::Finished => {
                        state.devices[*seat].finished = true;
                        state.stop_publishing(*seat);
                        device.destroy();
                    }
                    _ => {}
                }
            }
            event_created_child!(State, $device, [$device_mod::EVT_DATA_OFFER_OPCODE => ($offer, ())]);
        }
        impl Dispatch<$source, usize> for State {
            fn event(state: &mut Self, source: &$source, event: <$source as Proxy>::Event, seat: &usize, _: &Connection, _: &QueueHandle<Self>) {
                match event {
                    $source_mod::Event::Send { mime_type, fd } if TEXT_TYPES.contains(&mime_type.as_str()) => state.send(fd),
                    $source_mod::Event::Cancelled => { source.destroy(); state.stop_publishing(*seat); }
                    _ => {}
                }
            }
        }
    }
}
dispatch_data_control!(
    ext_device::ExtDataControlDeviceV1,
    ext_device,
    ExtDataControlOfferV1,
    ext_source::ExtDataControlSourceV1,
    ext_source
);
dispatch_data_control!(
    wlr_device::ZwlrDataControlDeviceV1,
    wlr_device,
    ZwlrDataControlOfferV1,
    wlr_source::ZwlrDataControlSourceV1,
    wlr_source
);

pub(super) fn serve(
    text: String,
    primary: bool,
    owner: Arc<()>,
    close: calloop::channel::Channel<()>,
    ready: &SyncSender<Result<(), String>>,
) -> Result<(), Box<dyn Error>> {
    let conn = Connection::connect_to_env()?;
    let (globals, mut queue) = registry_queue_init::<State>(&conn)?;
    let qh = queue.handle();
    let seats = globals.contents().with_list(|list| {
        list.iter()
            .filter(|global| global.interface == "wl_seat")
            .map(|global| {
                globals
                    .registry()
                    .bind::<WlSeat, _, _>(global.name, 1, &qh, ())
            })
            .collect::<Vec<_>>()
    });
    if seats.is_empty() {
        return Err("no Wayland seat".into());
    }
    let mut event_loop = EventLoop::try_new()?;
    let mut state = State {
        handle: event_loop.handle(),
        text: Rc::from(text.into_bytes()),
        devices: seats.iter().map(|_| Device::default()).collect(),
        transfers: 0,
        transferred: 0,
        owner: Some(owner),
        closed: false,
    };
    state
        .handle
        .insert_source(close, |_, _, state| state.closed = true)?;
    macro_rules! publish {
        ($manager:expr) => {{
            let manager = $manager;
            let devices = seats
                .iter()
                .enumerate()
                .map(|(index, seat)| manager.get_data_device(seat, &qh, index))
                .collect::<Vec<_>>();
            queue.roundtrip(&mut state)?;
            for (index, device) in devices.into_iter().enumerate() {
                let seat = &mut state.devices[index];
                if seat.finished || (primary && !seat.primary_supported) {
                    continue;
                }
                let source = manager.create_data_source(&qh, index);
                for mime in TEXT_TYPES {
                    source.offer((*mime).into());
                }
                if primary {
                    device.set_primary_selection(Some(&source));
                } else {
                    device.set_selection(Some(&source));
                }
                seat.publishing = true;
            }
        }};
    }
    if let Ok(manager) = globals.bind::<ExtDataControlManagerV1, _, _>(&qh, 1..=1, ()) {
        publish!(manager);
    } else {
        publish!(globals.bind::<ZwlrDataControlManagerV1, _, _>(
            &qh,
            (if primary { 2 } else { 1 })..=2,
            ()
        )?);
    }
    queue.roundtrip(&mut state)?;
    let result = if state.devices.iter().any(|device| device.publishing) {
        Ok(())
    } else {
        state.owner = None;
        Err("no active Wayland device accepted the selection".into())
    };
    WaylandSource::new(conn, queue).insert(event_loop.handle())?;
    if ready.send(result).is_err() {
        state.closed = true;
    }
    while state.owner.is_some() && !state.closed {
        event_loop.dispatch(None, &mut state)?;
    }
    state.owner = None;
    // Finish pastes already in progress after replacement, allowing a stalled reader to expire.
    const DRAIN_IDLE: Duration = Duration::from_secs(5);
    let mut deadline = Instant::now() + DRAIN_IDLE;
    while state.transfers > 0 {
        let Some(left) = deadline.checked_duration_since(Instant::now()) else {
            break;
        };
        let before = state.transferred;
        event_loop.dispatch(left, &mut state)?;
        if state.transferred != before {
            deadline = Instant::now() + DRAIN_IDLE;
        }
    }
    Ok(())
}
