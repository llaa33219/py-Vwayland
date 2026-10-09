//! Compositor-side Wayland clipboard (`wl_data_device` selection) control.
//!
//! - `clipboard_set` publishes a UTF-8 text selection owned by the compositor.
//! - `clipboard_get` returns the compositor-owned text directly, or pulls the
//!   text out of a selection owned by the running app.
//! - `clipboard_clear` removes the selection.
//!
//! Only the clipboard (primary) selection is handled; the primary selection is
//! out of scope.

use std::io::{Read, Write};
use std::os::unix::io::OwnedFd;
use std::os::unix::net::UnixStream;
use std::sync::{Arc, Mutex, OnceLock};
use std::time::{Duration, Instant};

use serde_json::{json, Value};
use smithay::reexports::calloop::timer::TimeoutAction;
use smithay::reexports::wayland_server::DisplayHandle;
use smithay::wayland::selection::data_device::{
    clear_data_device_selection, current_data_device_selection_userdata,
    request_data_device_client_selection, set_data_device_selection, SelectionRequestError,
};

use tracing::warn;

use crate::state::Vwayland;
use crate::CalloopData;

/// Mime types advertised for compositor-owned selections, best first.
const MIME_TYPES: [&str; 2] = ["text/plain;charset=utf-8", "text/plain"];

/// Upper bound for text pulled out of a client-owned selection.
const MAX_BYTES: usize = 8 * 1024 * 1024;

/// Total time a `clipboard_get` may wait for the app to fill the pipe.
const GET_TIMEOUT: Duration = Duration::from_secs(5);

/// Poll interval while a `clipboard_get` is in flight.
const POLL_INTERVAL: Duration = Duration::from_millis(10);

/// Interval of the timer when no `clipboard_get` is in flight.
const IDLE_INTERVAL: Duration = Duration::from_millis(500);

/// Take ownership of the clipboard selection with the given UTF-8 text.
pub fn set(dh: &DisplayHandle, state: &Vwayland, text: &str) {
    set_data_device_selection::<Vwayland>(
        dh,
        &state.seat,
        MIME_TYPES.iter().map(|m| (*m).to_string()).collect(),
        Arc::from(text.as_bytes()),
    );
}

/// Drop the current selection.
pub fn clear(dh: &DisplayHandle, state: &Vwayland) {
    clear_data_device_selection::<Vwayland>(dh, &state.seat);
}

/// Answer `clipboard_get`.
///
/// Returns `Some(response)` when the answer is known immediately, or `None`
/// when the app has to fill a pipe first — in that case `stream` is parked in
/// [`pending`] and the poll timer (`poll`) writes the response later.
pub fn get(state: &Vwayland, stream: UnixStream) -> Option<Value> {
    if lock_pending().is_some() {
        return Some(error("another clipboard_get is still in flight"));
    }
    // Our own selection: the bytes are already in memory, no round-trip needed.
    if let Some(user_data) = current_data_device_selection_userdata::<Vwayland>(&state.seat) {
        return Some(text_response(Some(to_text(user_data[..].to_vec()))));
    }

    let mut last_error = String::new();
    for mime_type in MIME_TYPES {
        let (write_end, read_end) = match UnixStream::pair() {
            Ok(pair) => pair,
            Err(e) => return Some(error(format!("clipboard pipe: {e}"))),
        };
        match request_data_device_client_selection::<Vwayland>(
            &state.seat,
            mime_type.to_string(),
            OwnedFd::from(write_end),
        ) {
            Ok(()) => {
                if let Err(e) = read_end.set_nonblocking(true) {
                    return Some(error(format!("clipboard pipe: {e}")));
                }
                *lock_pending() = Some(PendingGet {
                    stream,
                    read_end,
                    buf: Vec::new(),
                    deadline: Instant::now() + GET_TIMEOUT,
                });
                return None;
            }
            Err(SelectionRequestError::NoSelection) => return Some(text_response(None)),
            // The app offered the other text mime type: retry with a fresh pipe.
            Err(SelectionRequestError::InvalidMimetype) => {
                last_error = format!("the app offers no {mime_type}");
            }
            Err(e) => return Some(error(format!("clipboard request: {e}"))),
        }
    }
    Some(error(last_error))
}

/// Event-loop tick: advance an in-flight `clipboard_get`, if any.
///
/// The wait must stay event-loop driven: `wl_data_offer.receive` is only
/// dispatched while the wayland loop runs, so blocking here would guarantee a
/// timeout for every client-owned selection. Instead the pipe is polled and the
/// wayland clients are flushed between reads.
pub fn poll(data: &mut CalloopData) -> TimeoutAction {
    let mut slot = lock_pending();
    let Some(mut request) = slot.take() else {
        return TimeoutAction::ToDuration(IDLE_INTERVAL);
    };
    let _ = data.display_handle.flush_clients();
    match request.step() {
        Step::Pending => {
            *slot = Some(request);
            TimeoutAction::ToDuration(POLL_INTERVAL)
        }
        Step::Done(response) => {
            write_response(&mut request.stream, response);
            TimeoutAction::ToDuration(IDLE_INTERVAL)
        }
    }
}

/// A `clipboard_get` waiting for the app to write the selection into the pipe.
struct PendingGet {
    stream: UnixStream,
    read_end: UnixStream,
    buf: Vec<u8>,
    deadline: Instant,
}

enum Step {
    Pending,
    Done(Value),
}

impl PendingGet {
    fn step(&mut self) -> Step {
        let mut chunk = [0u8; 8192];
        loop {
            match self.read_end.read(&mut chunk) {
                // EOF: the app is done writing.
                Ok(0) => {
                    let text = to_text(std::mem::take(&mut self.buf));
                    return Step::Done(text_response(Some(text)));
                }
                Ok(n) => {
                    self.buf.extend_from_slice(&chunk[..n]);
                    if self.buf.len() > MAX_BYTES {
                        return Step::Done(error("clipboard data exceeds 8 MiB"));
                    }
                }
                Err(e) if e.kind() == std::io::ErrorKind::WouldBlock => break,
                Err(e) => return Step::Done(error(format!("clipboard read: {e}"))),
            }
        }
        if Instant::now() >= self.deadline {
            Step::Done(error("clipboard_get timed out after 5s"))
        } else {
            Step::Pending
        }
    }
}

fn pending() -> &'static Mutex<Option<PendingGet>> {
    static PENDING: OnceLock<Mutex<Option<PendingGet>>> = OnceLock::new();
    PENDING.get_or_init(|| Mutex::new(None))
}

fn lock_pending() -> std::sync::MutexGuard<'static, Option<PendingGet>> {
    pending()
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner())
}

fn write_response(stream: &mut UnixStream, response: Value) {
    let line = match serde_json::to_string(&response) {
        Ok(mut line) => {
            line.push('\n');
            line
        }
        Err(e) => format!("{{\"ok\": false, \"error\": \"response encode: {e}\"}}\n"),
    };
    if let Err(e) = stream.write_all(line.as_bytes()).and_then(|()| stream.flush()) {
        warn!(error = %e, "clipboard response write failed");
    }
}

fn to_text(bytes: Vec<u8>) -> String {
    String::from_utf8(bytes).unwrap_or_else(|e| String::from_utf8_lossy(e.as_bytes()).into_owned())
}

fn text_response(text: Option<String>) -> Value {
    json!({"ok": true, "text": text})
}

fn error(message: impl Into<String>) -> Value {
    json!({"ok": false, "error": message.into()})
}