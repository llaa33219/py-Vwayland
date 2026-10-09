//! Clipboard (wl_data_device) support for the test client.
//!
//! A selection transfer is asynchronous: the compositor only starts writing the
//! selected data into our pipe after it has received the `receive` request,
//! which is sent from the main loop because an event handler cannot flush the
//! queue it is currently dispatching. The payload then arrives while the event
//! loop must keep running, so the read side is drained on a helper thread with
//! a bounded deadline instead of blocking the Wayland connection.

use std::io;
use std::os::fd::{AsRawFd, BorrowedFd, OwnedFd};
use std::os::unix::net::UnixStream;
use std::time::{Duration, Instant};

use wayland_client::protocol::{wl_data_device, wl_data_offer};
use wayland_client::{event_created_child, Connection, Dispatch, EventQueue, QueueHandle};

/// Mime types tried in order; the first one advertised by the offer is used.
const PREFERRED_MIMES: [&str; 2] = ["text/plain;charset=utf-8", "text/plain"];

/// Total time budget for a single transfer before it is reported as timed out.
const TRANSFER_TIMEOUT: Duration = Duration::from_secs(5);

const READ_CHUNK: usize = 4096;

pub struct Clipboard {
    /// Mime types advertised by the current data offer.
    mimes: Vec<String>,
    /// Proxy for the current data offer, kept to request the transfer.
    offer: Option<wl_data_offer::WlDataOffer>,
    /// Transfer announced by an event handler, sent once the queue is flushed.
    pending: Option<PendingTransfer>,
}

impl Clipboard {
    pub fn new() -> Self {
        Self {
            mimes: Vec::new(),
            offer: None,
            pending: None,
        }
    }

    /// Handles `wl_data_device.data_offer`: the mime list of the new offer
    /// arrives through the following `wl_data_offer.offer` events.
    pub fn on_data_offer(&mut self, id: wl_data_offer::WlDataOffer) {
        // Only one selection is live at a time: drop the previous offer.
        if let Some(previous) = self.offer.replace(id) {
            previous.destroy();
        }
        self.mimes.clear();
    }

    pub fn add_mime(&mut self, mime: String) {
        self.mimes.push(mime);
    }

    /// Handles `wl_data_device.selection`; `None` means the selection was cleared.
    pub fn on_selection(&mut self, id: Option<wl_data_offer::WlDataOffer>) {
        if id.is_none() {
            crate::log_line("clipboard-cleared".to_string());
            return;
        }
        let Some(offer) = self.offer.clone() else {
            return;
        };
        let Some(mime) = pick_mime(&self.mimes) else {
            crate::log_line("clipboard-unsupported".to_string());
            return;
        };
        if let Ok(pipe) = Pipe::new() {
            self.pending = Some(PendingTransfer { offer, mime, pipe });
        }
    }

    /// Sends a transfer announced by an event handler and drains its pipe.
    pub fn flush_pending<State: 'static>(&mut self, queue: &EventQueue<State>) {
        let Some(PendingTransfer { offer, mime, pipe }) = self.pending.take() else {
            return;
        };
        let Pipe { reader, writer } = pipe;
        offer.receive(mime, unsafe { BorrowedFd::borrow_raw(writer.as_raw_fd()) });
        queue.flush().expect("flush failed");
        // The compositor owns a dup of the write end since the flush, so ours
        // must go: while it is open the read end can never see EOF.
        drop(writer);
        spawn_reader(reader);
    }
}

struct PendingTransfer {
    offer: wl_data_offer::WlDataOffer,
    mime: String,
    pipe: Pipe,
}

/// A pipe pair; the write end is sent to the compositor with `receive`.
struct Pipe {
    reader: OwnedFd,
    writer: OwnedFd,
}

impl Pipe {
    fn new() -> io::Result<Self> {
        let (reader, writer) = UnixStream::pair()?;
        Ok(Self {
            reader: OwnedFd::from(reader),
            writer: OwnedFd::from(writer),
        })
    }
}

/// Reads a transferred selection to EOF on a helper thread and logs the
/// payload (or a timeout) on stdout, leaving the event loop free to run.
fn spawn_reader(reader: OwnedFd) {
    std::thread::spawn(move || match read_to_eof(&reader, TRANSFER_TIMEOUT) {
        Transfer::Complete(bytes) => {
            let text = String::from_utf8(bytes)
                .unwrap_or_else(|e| String::from_utf8_lossy(e.as_bytes()).into_owned());
            crate::log_line(format!("clipboard {text}"));
        }
        Transfer::TimedOut => crate::log_line("clipboard-timeout".to_string()),
        Transfer::Failed => crate::log_line("clipboard-failed".to_string()),
    });
}

/// Picks the best supported mime type of a `wl_data_offer` mime list.
fn pick_mime(mimes: &[String]) -> Option<String> {
    PREFERRED_MIMES
        .iter()
        .find(|preferred| mimes.iter().any(|mime| mime == *preferred))
        .map(|preferred| (*preferred).to_string())
}

enum Transfer {
    Complete(Vec<u8>),
    TimedOut,
    Failed,
}

/// Reads until EOF, giving up once `timeout` has elapsed.
fn read_to_eof(fd: &OwnedFd, timeout: Duration) -> Transfer {
    let deadline = Instant::now() + timeout;
    let mut out: Vec<u8> = Vec::new();
    let mut buf = [0u8; READ_CHUNK];
    loop {
        let remaining = deadline.saturating_duration_since(Instant::now());
        if remaining.is_zero() {
            return Transfer::TimedOut;
        }
        let mut poll_fd = libc::pollfd {
            fd: fd.as_raw_fd(),
            events: libc::POLLIN,
            revents: 0,
        };
        // Wait at most one second per poll so the deadline is re-checked.
        let wait_ms = remaining.as_millis().clamp(1, 1000) as i32;
        let ready = unsafe { libc::poll(&mut poll_fd, 1, wait_ms) };
        if ready < 0 {
            return Transfer::Failed;
        }
        if ready == 0 {
            continue;
        }
        let n = unsafe {
            libc::read(
                fd.as_raw_fd(),
                buf.as_mut_ptr().cast::<libc::c_void>(),
                buf.len(),
            )
        };
        match n {
            0 => return Transfer::Complete(out),
            n if n > 0 => out.extend_from_slice(&buf[..n as usize]),
            err if err == libc::EINTR as isize => {}
            _ => return Transfer::Failed,
        }
    }
}

impl Dispatch<wl_data_device::WlDataDevice, ()> for crate::AppState {
    fn event(
        state: &mut Self,
        _proxy: &wl_data_device::WlDataDevice,
        event: wl_data_device::Event,
        _data: &(),
        _conn: &Connection,
        _qh: &QueueHandle<Self>,
    ) {
        match event {
            wl_data_device::Event::DataOffer { id } => state.clipboard.on_data_offer(id),
            wl_data_device::Event::Selection { id } => state.clipboard.on_selection(id),
            _ => {}
        }
    }

    event_created_child!(crate::AppState, wl_data_device::WlDataDevice, [
        wl_data_device::EVT_DATA_OFFER_OPCODE => (wl_data_offer::WlDataOffer, ()),
    ]);
}

impl Dispatch<wl_data_offer::WlDataOffer, ()> for crate::AppState {
    fn event(
        state: &mut Self,
        _proxy: &wl_data_offer::WlDataOffer,
        event: wl_data_offer::Event,
        _data: &(),
        _conn: &Connection,
        _qh: &QueueHandle<Self>,
    ) {
        if let wl_data_offer::Event::Offer { mime_type } = event {
            state.clipboard.add_mime(mime_type);
        }
    }
}
