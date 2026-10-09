//! vwayland-test-client: minimal Wayland client for py-Vwayland integration tests.
//!
//! - Draws a solid-color (RRGGBB, default ff0000) fullscreen shm buffer.
//! - Prints "VWTEST ..." lines to stdout for received pointer/keyboard events.
//! - Prints "VWTEST ready <w>x<h>" after the first draw.
//! - Prints "VWTEST clipboard <text>" / "VWTEST clipboard-cleared" for selections.
//! - Prints "VWTEST text-input <text>" for text committed by an input method,
//!   and "VWTEST typed <char>" for characters resolved from the xkb keymap.
//!
//! Usage: vwayland-test-client [RRGGBB] [--no-text-input]

mod clipboard;
mod draw;
mod keyboard_text;
mod text_input;

/// Argument that suppresses the text input, so `type_text` takes the key
/// event path instead.
const NO_TEXT_INPUT: &str = "--no-text-input";

use wayland_client::globals::{registry_queue_init, GlobalListContents};
use wayland_client::protocol::{
    wl_buffer, wl_compositor, wl_data_device_manager, wl_pointer, wl_registry, wl_seat, wl_shm,
    wl_shm_pool, wl_surface,
};
use wayland_client::{delegate_noop, Connection, Dispatch, QueueHandle};
use wayland_protocols::wp::text_input::zv3::client::zwp_text_input_manager_v3;
use wayland_protocols::xdg::shell::client::{xdg_surface, xdg_toplevel, xdg_wm_base};

struct AppState {
    shm: wl_shm::WlShm,
    surface: wl_surface::WlSurface,
    width: u32,
    height: u32,
    /// 0xRRGGBB
    color: u32,
    drawn: bool,
    clipboard: clipboard::Clipboard,
    keyboard_text: keyboard_text::KeyboardText,
    text_input: text_input::TextInput,
}

fn log_line(line: String) {
    println!("VWTEST {line}");
    use std::io::Write;
    let _ = std::io::stdout().flush();
}

delegate_noop!(AppState: ignore wl_compositor::WlCompositor);
delegate_noop!(AppState: ignore wl_shm::WlShm);
delegate_noop!(AppState: ignore wl_shm_pool::WlShmPool);
delegate_noop!(AppState: ignore wl_buffer::WlBuffer);
delegate_noop!(AppState: ignore wl_surface::WlSurface);
delegate_noop!(AppState: ignore wl_data_device_manager::WlDataDeviceManager);

impl Dispatch<xdg_wm_base::XdgWmBase, ()> for AppState {
    fn event(
        _state: &mut Self,
        proxy: &xdg_wm_base::XdgWmBase,
        event: xdg_wm_base::Event,
        _data: &(),
        _conn: &Connection,
        _qh: &QueueHandle<Self>,
    ) {
        if let xdg_wm_base::Event::Ping { serial } = event {
            proxy.pong(serial);
        }
    }
}

impl Dispatch<xdg_surface::XdgSurface, ()> for AppState {
    fn event(
        state: &mut Self,
        proxy: &xdg_surface::XdgSurface,
        event: xdg_surface::Event,
        _data: &(),
        _conn: &Connection,
        qh: &QueueHandle<Self>,
    ) {
        if let xdg_surface::Event::Configure { serial } = event {
            proxy.ack_configure(serial);
            state.draw(qh);
            state.drawn = true;
        }
    }
}

impl Dispatch<xdg_toplevel::XdgToplevel, ()> for AppState {
    fn event(
        state: &mut Self,
        _proxy: &xdg_toplevel::XdgToplevel,
        event: xdg_toplevel::Event,
        _data: &(),
        _conn: &Connection,
        _qh: &QueueHandle<Self>,
    ) {
        match event {
            xdg_toplevel::Event::Configure { width, height, .. } => {
                if width > 0 && height > 0 {
                    state.width = width as u32;
                    state.height = height as u32;
                }
            }
            xdg_toplevel::Event::Close => {
                log_line("toplevel_close".to_string());
                std::process::exit(0);
            }
            _ => {}
        }
    }
}

impl Dispatch<wl_seat::WlSeat, ()> for AppState {
    fn event(
        state: &mut Self,
        proxy: &wl_seat::WlSeat,
        event: wl_seat::Event,
        _data: &(),
        _conn: &Connection,
        qh: &QueueHandle<Self>,
    ) {
        if let wl_seat::Event::Capabilities { capabilities } = event {
            let _ = state;
            let caps: wayland_client::WEnum<wl_seat::Capability> = capabilities;
            if let wayland_client::WEnum::Value(caps) = caps {
                if caps.contains(wl_seat::Capability::Pointer) {
                    proxy.get_pointer(qh, ());
                }
                if caps.contains(wl_seat::Capability::Keyboard) {
                    proxy.get_keyboard(qh, ());
                }
            }
        }
    }
}

impl Dispatch<wl_pointer::WlPointer, ()> for AppState {
    fn event(
        _state: &mut Self,
        _proxy: &wl_pointer::WlPointer,
        event: wl_pointer::Event,
        _data: &(),
        _conn: &Connection,
        _qh: &QueueHandle<Self>,
    ) {
        match event {
            wl_pointer::Event::Enter {
                surface_x,
                surface_y,
                ..
            } => {
                log_line(format!("pointer_enter {surface_x:.1} {surface_y:.1}"));
            }
            wl_pointer::Event::Motion {
                surface_x,
                surface_y,
                ..
            } => {
                log_line(format!("pointer_motion {surface_x:.1} {surface_y:.1}"));
            }
            wl_pointer::Event::Button { button, state, .. } => {
                log_line(format!("pointer_button {button} {state:?}"));
            }
            wl_pointer::Event::Axis { axis, value, .. } => {
                log_line(format!("pointer_axis {axis:?} {value:.2}"));
            }
            wl_pointer::Event::Leave { .. } => log_line("pointer_leave".to_string()),
            _ => {}
        }
    }
}

impl Dispatch<wl_registry::WlRegistry, GlobalListContents> for AppState {
    fn event(
        _state: &mut Self,
        _proxy: &wl_registry::WlRegistry,
        _event: wl_registry::Event,
        _data: &GlobalListContents,
        _conn: &Connection,
        _qh: &QueueHandle<Self>,
    ) {
    }
}

fn main() {
    let args: Vec<String> = std::env::args().skip(1).collect();
    let color = args
        .first()
        .and_then(|s| u32::from_str_radix(s.trim_start_matches('#'), 16).ok())
        .unwrap_or(0xff0000);
    // Without a text input the compositor falls back to typing real key events,
    // which is the only way to observe that path.
    let wants_text_input = !args.iter().any(|arg| arg == NO_TEXT_INPUT);

    let conn = Connection::connect_to_env().expect("cannot connect to wayland display");
    let (globals, mut queue) = registry_queue_init::<AppState>(&conn).expect("registry failed");
    let qh = queue.handle();

    let compositor: wl_compositor::WlCompositor =
        globals.bind(&qh, 1..=4, ()).expect("no compositor");
    let shm: wl_shm::WlShm = globals.bind(&qh, 1..=1, ()).expect("no shm");
    let xdg: xdg_wm_base::XdgWmBase = globals.bind(&qh, 1..=6, ()).expect("no xdg_wm_base");
    let seat: wl_seat::WlSeat = globals.bind(&qh, 1..=7, ()).expect("no seat");

    let data_device_manager: wl_data_device_manager::WlDataDeviceManager = globals
        .bind(&qh, 1..=3, ())
        .expect("no data device manager");
    // The proxy stays alive in the connection; events keep arriving on it.
    let _data_device = data_device_manager.get_data_device(&seat, &qh, ());

    let surface = compositor.create_surface(&qh, ());
    if wants_text_input {
        let text_input_manager: zwp_text_input_manager_v3::ZwpTextInputManagerV3 = globals
            .bind(&qh, 1..=1, ())
            .expect("no text input manager");
        // Created before the surface is mapped so the compositor can send
        // `enter` as soon as the keyboard focus lands on it.
        text_input_manager.get_text_input(&seat, &qh, ());
    }
    let xdg_surface = xdg.get_xdg_surface(&surface, &qh, ());
    let toplevel = xdg_surface.get_toplevel(&qh, ());
    toplevel.set_title("vwayland-test-client".to_string());
    surface.commit();

    let mut state = AppState {
        shm,
        surface,
        width: 0,
        height: 0,
        color,
        drawn: false,
        clipboard: clipboard::Clipboard::new(),
        keyboard_text: keyboard_text::KeyboardText::new(),
        text_input: text_input::TextInput::new(),
    };

    // Roundtrip once to receive the seat capabilities event
    queue.roundtrip(&mut state).expect("roundtrip failed");

    loop {
        queue
            .blocking_dispatch(&mut state)
            .expect("dispatch failed");
        // The compositor only writes into the pipe after receiving these
        // requests, so they can be queued from the handler but never flushed there.
        state.clipboard.flush_pending(&queue);
    }
}
