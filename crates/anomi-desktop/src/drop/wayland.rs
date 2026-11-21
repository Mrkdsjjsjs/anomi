//! A `wl_data_device` for the seat, on winit's `wl_display` but on a
//! queue of its own, which a thread dispatches: winit reads the socket
//! and libwayland queues these events for the thread. It takes
//! `text/uri-list` on a drop. There's a device for each seat, not for a
//! window, so a window made again, as from the tray, needs nothing new:
//! closing a window leaves winit's connection open.
//!
//! The display is winit's, which it closes as its event loop drops, and
//! with it every object on it. So the thread runs only while the [`Watch`]
//! it came with is held: dropping that wakes the thread, which lets go of
//! its objects while the display is still open, and waits for it. A
//! connection that breaks ends the thread on its own, which then touches
//! nothing of it again: its objects are forgotten rather than destroyed,
//! as destroying one reaches into the display, which may be gone. So are
//! they if the display closed before the watch stopped, as when a panic
//! unwinds past winit's event loop ([`Socket`]).

mod list;
#[cfg(test)]
mod tests;
mod thread;

use std::io::{ErrorKind, PipeReader};
use std::os::fd::{AsFd, AsRawFd, BorrowedFd, RawFd};
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};

use wayland_client::backend::{Backend, WaylandError};
use wayland_client::protocol::wl_data_device::{self, WlDataDevice};
use wayland_client::protocol::wl_data_device_manager::{DndAction, WlDataDeviceManager};
use wayland_client::protocol::wl_data_offer::{self, WlDataOffer};
use wayland_client::protocol::wl_registry::{self, WlRegistry};
use wayland_client::protocol::wl_seat::WlSeat;
use wayland_client::{Connection, Dispatch, EventQueue, Proxy, QueueHandle, event_created_child};

pub use thread::Watch;
use thread::{Woke, wait};

use super::Drag;

const URI_LIST: &str = "text/uri-list";

/// Follows files dragged over Yapfast's windows on the connection whose
/// `wl_display` this is, reporting each step to `report` from a thread,
/// until the watch drops; none if the thread can't start.
///
/// # Safety
///
/// `display` is a live `wl_display`, which stays open until the watch has
/// dropped. winit closes its own as its event loop drops, after the app's
/// state, so the app keeps the watch there.
pub unsafe fn watch(
    display: std::ptr::NonNull<std::ffi::c_void>,
    report: impl Fn(Drag) + Send + Sync + 'static,
) -> Option<Watch> {
    let display = display.as_ptr() as usize;
    let started = Watch::spawn(move |stop| {
        // SAFETY: the caller keeps the display open while the watch is
        // held, and dropping the watch wakes this thread and waits for it
        // to let go of the display.
        let backend = unsafe { Backend::from_foreign_display(display as *mut _) };
        follow(Connection::from_backend(backend), Arc::new(report), stop);
    });
    match started {
        Ok(watch) => Some(watch),
        Err(error) => {
            tracing::warn!(target: crate::LOG, %error, "could not watch for files dropped on the window");
            None
        }
    }
}

/// The connection broke: why, from libwayland or the socket.
struct Broken(String);

impl<E: std::fmt::Display> From<E> for Broken {
    fn from(error: E) -> Self {
        Self(error.to_string())
    }
}

/// The display's socket, told apart from a file that took its number once
/// winit closed it.
struct Socket {
    fd: RawFd,
    /// Its device and inode, as it opened: none if they couldn't be read.
    file: Option<(u64, u64)>,
}

impl Socket {
    fn of(connection: &Connection) -> Self {
        let fd = connection.backend().poll_fd().as_raw_fd();
        Self { fd, file: file(fd) }
    }

    /// Whether the display is still open, as its socket is: the app should
    /// drop the watch first, but a panic unwinding past winit's event loop
    /// closes the display before it.
    fn open(&self) -> bool {
        self.file.is_none() || file(self.fd) == self.file
    }
}

/// The device and inode of the file numbered `fd`, if one is open.
fn file(fd: RawFd) -> Option<(u64, u64)> {
    use std::os::unix::fs::MetadataExt;
    // SAFETY: the number is only looked up, through a copy of the file
    // under it, if there's one, which is closed again here.
    let copy = unsafe { BorrowedFd::borrow_raw(fd) }
        .try_clone_to_owned()
        .ok()?;
    let metadata = std::fs::File::from(copy).metadata().ok()?;
    Some((metadata.dev(), metadata.ino()))
}

/// The thread: binds a data device for each seat and follows drags until
/// the watch drops or the connection breaks.
fn follow(connection: Connection, report: Arc<dyn Fn(Drag) + Send + Sync>, stop: PipeReader) {
    tracing::debug!(target: crate::LOG, "watching for files dropped on the window");
    let mut queue = connection.new_event_queue();
    connection.display().get_registry(&queue.handle(), ());
    let mut state = State {
        report,
        socket: Socket::of(&connection),
        connection,
        stop,
        stopping: false,
        manager: None,
        seats: Vec::new(),
        offer: None,
        left: None,
    };
    match dispatch(&mut queue, &mut state) {
        // The display is still open: the device, the seats and the offers
        // go with `state` and `queue` as they drop here.
        Ok(()) => {
            tracing::debug!(target: crate::LOG, "stopped watching for files dropped on the window")
        }
        Err(Broken(error)) => {
            tracing::warn!(target: crate::LOG, %error, "files can no longer be dropped on the window");
            // Every object of a broken connection is already dead, and
            // destroying one reaches into the display, which may be gone
            // with it: a handle to the connection is left behind, once, so
            // libwayland's side of it, each object and the queue, is never
            // destroyed as `state` and `queue` drop here.
            std::mem::forget(state.connection.clone());
        }
    }
}

/// Dispatches the queue's events as they come, until the watch drops.
fn dispatch(queue: &mut EventQueue<State>, state: &mut State) -> Result<(), Broken> {
    loop {
        queue.dispatch_pending(state)?;
        // The watch dropped while a drop's list was read.
        if state.stopping {
            return state.still_open();
        }
        match queue.flush() {
            // A full socket takes the rest at the next flush, winit's or
            // this thread's.
            Err(WaylandError::Io(error)) if error.kind() == ErrorKind::WouldBlock => {}
            flushed => flushed?,
        }
        // Events another thread read meanwhile wait in the queue.
        let Some(guard) = queue.prepare_read() else {
            continue;
        };
        let woke = wait(guard.connection_fd(), state.stop.as_fd());
        match woke {
            Ok(Woke::Ready) => match guard.read() {
                Ok(_) => {}
                // Another thread read the socket first.
                Err(WaylandError::Io(error)) if error.kind() == ErrorKind::WouldBlock => {}
                Err(error) => return Err(error.into()),
            },
            // Dropping the guard gives the read back, on the open display.
            Ok(Woke::Stopped) if state.socket.open() => return Ok(()),
            Ok(Woke::Stopped | Woke::Closed) => {
                // Giving the read back would reach into the display.
                std::mem::forget(guard);
                return Err(Broken("the display's socket was closed".into()));
            }
            Err(error) => return Err(error.into()),
        }
    }
}

impl State {
    /// Whether the display is still open as the watch stops.
    fn still_open(&self) -> Result<(), Broken> {
        if self.socket.open() {
            Ok(())
        } else {
            Err(Broken("the display closed before the watch stopped".into()))
        }
    }
}

struct State {
    report: Arc<dyn Fn(Drag) + Send + Sync>,
    socket: Socket,
    connection: Connection,
    /// Reads as closed once the watch drops.
    stop: PipeReader,
    /// The watch dropped while a drop's list was read.
    stopping: bool,
    manager: Option<WlDataDeviceManager>,
    /// Seats announced before the manager, waiting for it.
    seats: Vec<WlSeat>,
    /// The offer over a window, and whether it holds files.
    offer: Option<(WlDataOffer, bool)>,
    /// The offer a drag left with, kept until the next drag comes: Weston
    /// cancels a drag whose last offer is destroyed before another
    /// client's takes its place.
    left: Option<WlDataOffer>,
}

/// Whether an offer holds files.
#[derive(Default)]
struct Offered(AtomicBool);

impl Dispatch<WlRegistry, ()> for State {
    fn event(
        state: &mut Self,
        registry: &WlRegistry,
        event: wl_registry::Event,
        _: &(),
        _: &Connection,
        queue: &QueueHandle<Self>,
    ) {
        let wl_registry::Event::Global {
            name,
            interface,
            version,
        } = event
        else {
            return;
        };
        match interface.as_str() {
            // Version 3 for the actions a drop needs agreed.
            "wl_data_device_manager" if state.manager.is_none() => {
                state.manager = Some(registry.bind(name, version.min(3), queue, ()));
            }
            "wl_seat" => state.seats.push(registry.bind(name, 1, queue, ())),
            _ => return,
        }
        if let Some(manager) = &state.manager {
            for seat in state.seats.drain(..) {
                manager.get_data_device(&seat, queue, ());
            }
        }
    }
}

impl Dispatch<WlDataDevice, ()> for State {
    fn event(
        state: &mut Self,
        _: &WlDataDevice,
        event: wl_data_device::Event,
        _: &(),
        _: &Connection,
        _: &QueueHandle<Self>,
    ) {
        match event {
            wl_data_device::Event::Enter {
                serial,
                x,
                y,
                id: Some(offer),
                ..
            } => {
                let files = offer
                    .data::<Offered>()
                    .is_some_and(|offered| offered.0.load(Ordering::Relaxed));
                if files {
                    offer.accept(serial, Some(URI_LIST.into()));
                    if offer.version() >= 3 {
                        offer.set_actions(DndAction::Copy, DndAction::Copy);
                    }
                    // Where first, so the window shows the right place.
                    (state.report)(Drag::Moved { x, y });
                    (state.report)(Drag::Entered);
                } else {
                    offer.accept(serial, None);
                }
                let old = state.offer.replace((offer, files));
                for old in [old.map(|(old, _)| old), state.left.take()]
                    .into_iter()
                    .flatten()
                {
                    old.destroy();
                }
            }
            wl_data_device::Event::Motion { x, y, .. } => {
                if state.offer.as_ref().is_some_and(|(_, files)| *files) {
                    (state.report)(Drag::Moved { x, y });
                }
            }
            wl_data_device::Event::Leave => {
                if let Some((offer, files)) = state.offer.take() {
                    state.left = Some(offer);
                    if files {
                        (state.report)(Drag::Left);
                    }
                }
            }
            wl_data_device::Event::Drop => {
                if let Some((offer, files)) = state.offer.take() {
                    let list = if files { state.receive(&offer) } else { None };
                    // Stopped while the list came: the offer goes with the
                    // rest, as the display may be gone (`dispatch`).
                    if state.stopping {
                        return;
                    }
                    if list.is_some() && offer.version() >= 3 {
                        offer.finish();
                    }
                    offer.destroy();
                    if let Some(list) = list {
                        state.dropped(list);
                    }
                }
            }
            // The clipboard's offers: Yapfast reads the clipboard elsewhere.
            wl_data_device::Event::Selection { id: Some(offer) } => offer.destroy(),
            _ => {}
        }
    }

    event_created_child!(State, WlDataDevice, [
        wl_data_device::EVT_DATA_OFFER_OPCODE => (WlDataOffer, Offered::default()),
    ]);
}

impl Dispatch<WlDataOffer, Offered> for State {
    fn event(
        _: &mut Self,
        _: &WlDataOffer,
        event: wl_data_offer::Event,
        offered: &Offered,
        _: &Connection,
        _: &QueueHandle<Self>,
    ) {
        if let wl_data_offer::Event::Offer { mime_type } = event
            && mime_type == URI_LIST
        {
            offered.0.store(true, Ordering::Relaxed);
        }
    }
}

impl Dispatch<WlDataDeviceManager, ()> for State {
    fn event(
        _: &mut Self,
        _: &WlDataDeviceManager,
        _: <WlDataDeviceManager as Proxy>::Event,
        _: &(),
        _: &Connection,
        _: &QueueHandle<Self>,
    ) {
    }
}

impl Dispatch<WlSeat, ()> for State {
    fn event(
        _: &mut Self,
        _: &WlSeat,
        _: <WlSeat as Proxy>::Event,
        _: &(),
        _: &Connection,
        _: &QueueHandle<Self>,
    ) {
    }
}

/// The `wl_display` a window is on, for [`watch`]: none for a window that
/// isn't on Wayland.
pub fn display<W: raw_window_handle::HasDisplayHandle + ?Sized>(
    window: &W,
) -> Option<std::ptr::NonNull<std::ffi::c_void>> {
    match window.display_handle().ok()?.as_raw() {
        raw_window_handle::RawDisplayHandle::Wayland(handle) => Some(handle.display),
        _ => None,
    }
}
