//! IPC server: JSON-line protocol over the Unix socket (ipc.sock) in the
//! instance directory.
//!
//! Request: one JSON line ({"cmd": ...})
//! Response: one JSON line ({"ok": true, ...} or {"ok": false, "error": ...})
//! Only screenshot is followed by N bytes of PNG after its response header line.

use std::collections::HashMap;
use std::io::{BufRead, BufReader, Write};
use std::os::unix::net::{UnixListener, UnixStream};
use std::path::Path;
use std::time::Duration;

use serde::Deserialize;
use serde_json::{json, Value};
use smithay::reexports::calloop::{
    generic::Generic,
    timer::Timer,
    EventLoop, Interest, Mode, PostAction,
};
use tracing::{info, warn};

use crate::{clipboard, typing, Backend, CalloopData, VERSION};

pub fn init(event_loop: &mut EventLoop<CalloopData>, sock_path: &Path) -> std::io::Result<()> {
    if sock_path.exists() {
        std::fs::remove_file(sock_path)?;
    }
    let listener = UnixListener::bind(sock_path)?;
    listener.set_nonblocking(true)?;
    event_loop.handle().insert_source(
        Generic::new(listener, Interest::READ, Mode::Level),
        move |_, listener, data| {
            loop {
                match listener.accept() {
                    Ok((stream, _)) => {
                        if let Err(e) = handle_conn(stream, data) {
                            warn!(error = %e, "ipc connection error");
                        }
                    }
                    Err(e) if e.kind() == std::io::ErrorKind::WouldBlock => break,
                    Err(e) => {
                        warn!(error = %e, "ipc accept failed");
                        break;
                    }
                }
            }
            Ok(PostAction::Continue)
        },
    )
    .map_err(|e| std::io::Error::new(std::io::ErrorKind::Other, format!("insert ipc source: {e:?}")))?;

    // Drives in-flight clipboard_get requests: it must not block the loop, or
    // the app could never answer the wl_data_offer.receive we just sent.
    event_loop
        .handle()
        .insert_source(Timer::from_duration(Duration::from_millis(500)), |_, _, data| {
            clipboard::poll(data)
        })
        .map_err(|e| {
            std::io::Error::new(std::io::ErrorKind::Other, format!("insert clipboard timer: {e:?}"))
        })?;

    info!(path = %sock_path.display(), "ipc listening");
    Ok(())
}

#[derive(Debug, Deserialize)]
#[serde(tag = "cmd", rename_all = "snake_case")]
enum Request {
    Ping,
    Launch {
        argv: Vec<String>,
        #[serde(default)]
        env: HashMap<String, String>,
        cwd: Option<String>,
    },
    CloseApp,
    Resize {
        width: i32,
        height: i32,
    },
    Screenshot,
    PointerMove {
        x: f64,
        y: f64,
    },
    PointerButton {
        button: u32,
        pressed: bool,
    },
    PointerAxis {
        dx: f64,
        dy: f64,
    },
    Key {
        code: u32,
        pressed: bool,
    },
    ClipboardSet {
        text: String,
    },
    ClipboardGet,
    ClipboardClear,
    TypeText {
        text: String,
        #[serde(default)]
        interval_ms: u64,
    },
    Shutdown,
}

fn handle_conn(stream: UnixStream, data: &mut CalloopData) -> Result<(), String> {
    stream.set_nonblocking(false).map_err(|e| e.to_string())?;
    stream
        .set_read_timeout(Some(Duration::from_secs(5)))
        .map_err(|e| e.to_string())?;
    stream
        .set_write_timeout(Some(Duration::from_secs(60)))
        .map_err(|e| e.to_string())?;

    let mut reader = BufReader::new(stream.try_clone().map_err(|e| e.to_string())?);
    let mut line = String::new();
    reader.read_line(&mut line).map_err(|e| format!("read: {e}"))?;

    // A deferred answer (clipboard_get on an app-owned selection) is written to
    // this duplicate of the connection later, from the event loop.
    let reply_stream = stream.try_clone().map_err(|e| e.to_string())?;

    let (resp, payload, deferred): (Value, Option<Vec<u8>>, bool) =
        match serde_json::from_str::<Request>(line.trim()) {
            Ok(req) => dispatch(req, data, reply_stream),
            Err(e) => (
                json!({"ok": false, "error": format!("invalid request: {e}")}),
                None,
                false,
            ),
        };

    let mut out = stream;
    if !deferred {
        let mut header = serde_json::to_string(&resp).map_err(|e| e.to_string())?;
        header.push('\n');
        out.write_all(header.as_bytes()).map_err(|e| format!("write: {e}"))?;
        if let Some(bytes) = payload {
            out.write_all(&bytes).map_err(|e| format!("write payload: {e}"))?;
        }
        out.flush().map_err(|e| e.to_string())?;
    }

    let _ = data.display_handle.flush_clients();
    Ok(())
}

fn ok(extra: Value) -> (Value, Option<Vec<u8>>, bool) {
    let mut obj = extra;
    if let Value::Object(ref mut map) = obj {
        map.insert("ok".to_string(), Value::Bool(true));
    }
    (obj, None, false)
}

fn err(msg: impl Into<String>) -> (Value, Option<Vec<u8>>, bool) {
    (json!({"ok": false, "error": msg.into()}), None, false)
}

/// Handle one request.
///
/// The third tuple item is `true` when the response is deferred: the command
/// parked `reply_stream` and will answer it from the event loop later.
fn dispatch(req: Request, data: &mut CalloopData, reply_stream: UnixStream) -> (Value, Option<Vec<u8>>, bool) {
    let state = &mut data.state;
    match req {
        Request::Ping => {
            let size = state.output_size();
            ok(json!({
                "version": VERSION,
                "id": state.id,
                "display": state.socket_name.to_string_lossy(),
                "width": size.w,
                "height": size.h,
                "headless": state.headless,
                "app_pid": state.app_pid(),
            }))
        }
        Request::Launch { argv, env, cwd } => match state.launch_app(&argv, &env, cwd.as_deref()) {
            Ok(pid) => ok(json!({"pid": pid})),
            Err(e) => err(e),
        },
        Request::CloseApp => {
            let closed = state.close_app();
            ok(json!({"closed": closed}))
        }
        Request::Resize { width, height } => {
            if !(16..=16384).contains(&width) || !(16..=16384).contains(&height) {
                return err("width/height must be in 16..=16384");
            }
            state.set_output_size(width, height);
            match &mut data.backend {
                Backend::Headless(h) => {
                    if let Err(e) = h.resize(state) {
                        return err(e);
                    }
                }
                Backend::Windowed(w) => {
                    let _ = w.backend.window().request_inner_size(
                        smithay::reexports::winit::dpi::PhysicalSize::new(width as u32, height as u32),
                    );
                }
            }
            let size = state.output_size();
            ok(json!({"width": size.w, "height": size.h}))
        }
        Request::Screenshot => {
            let result = match &mut data.backend {
                Backend::Headless(h) => h.screenshot(state),
                Backend::Windowed(w) => w.screenshot(state),
            };
            match result.and_then(|(w, h, rgba)| encode_png(w, h, &rgba).map(|png| (w, h, png))) {
                Ok((w, h, png)) => (
                    json!({"ok": true, "width": w, "height": h, "format": "png", "bytes": png.len()}),
                    Some(png),
                    false,
                ),
                Err(e) => err(e),
            }
        }
        Request::PointerMove { x, y } => {
            state.inject_pointer_move(x, y);
            ok(json!({}))
        }
        Request::PointerButton { button, pressed } => {
            state.inject_pointer_button(button, pressed);
            ok(json!({}))
        }
        Request::PointerAxis { dx, dy } => {
            state.inject_pointer_axis(dx, dy);
            ok(json!({}))
        }
        Request::Key { code, pressed } => {
            state.inject_key(code, pressed);
            ok(json!({}))
        }
        Request::ClipboardSet { text } => {
            clipboard::set(&data.display_handle, state, &text);
            ok(json!({}))
        }
        Request::ClipboardGet => match clipboard::get(state, reply_stream) {
            Some(resp) => (resp, None, false),
            None => (Value::Null, None, true),
        },
        Request::ClipboardClear => {
            clipboard::clear(&data.display_handle, state);
            ok(json!({}))
        }
        Request::TypeText { text, interval_ms } => match typing::type_text(state, &text, interval_ms) {
            Ok(method) => ok(json!({"method": method})),
            Err(e) => err(e),
        },
        Request::Shutdown => {
            info!("shutdown requested via ipc");
            state.loop_signal.stop();
            ok(json!({}))
        }
    }
}

fn encode_png(w: u32, h: u32, rgba: &[u8]) -> Result<Vec<u8>, String> {
    let mut out = Vec::new();
    {
        let mut encoder = png::Encoder::new(&mut out, w, h);
        encoder.set_color(png::ColorType::Rgba);
        encoder.set_depth(png::BitDepth::Eight);
        let mut writer = encoder.write_header().map_err(|e| format!("png: {e}"))?;
        writer
            .write_image_data(rgba)
            .map_err(|e| format!("png: {e}"))?;
    }
    Ok(out)
}
