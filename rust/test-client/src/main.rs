//! vwayland-test-client: minimal Wayland client for py-Vwayland integration tests.
//!
//! - Draws a solid-color (RRGGBB, default ff0000) fullscreen shm buffer.
//! - Prints "VWTEST ..." lines to stdout for received pointer/keyboard events.
//! - Prints "VWTEST ready <w>x<h>" after the first draw.
//!
//! Usage: vwayland-test-client [RRGGBB]

use std::ffi::CString;
use std::os::unix::io::BorrowedFd;

use wayland_client::globals::{registry_queue_init, GlobalListContents};
use wayland_client::protocol::{
    wl_buffer, wl_compositor, wl_keyboard, wl_pointer, wl_registry, wl_seat, wl_shm, wl_shm_pool,
    wl_surface,
};
use wayland_client::{delegate_noop, Connection, Dispatch, QueueHandle};
use wayland_protocols::xdg::shell::client::{xdg_surface, xdg_toplevel, xdg_wm_base};

struct AppState {
    shm: wl_shm::WlShm,
    surface: wl_surface::WlSurface,
    width: u32,
    height: u32,
    /// 0xRRGGBB
    color: u32,
    drawn: bool,
}

impl AppState {
    fn draw(&self, qh: &QueueHandle<Self>) {
        if self.width == 0 || self.height == 0 {
            return;
        }
        let size = (self.width * self.height * 4) as usize;

        let name = CString::new("vwayland-test-client").unwrap();
        let fd = unsafe { libc::memfd_create(name.as_ptr(), 0) };
        assert!(fd >= 0, "memfd_create failed");
        assert_eq!(unsafe { libc::ftruncate(fd, size as libc::off_t) }, 0);
        let ptr = unsafe {
            libc::mmap(
                std::ptr::null_mut(),
                size,
                libc::PROT_READ | libc::PROT_WRITE,
                libc::MAP_SHARED,
                fd,
                0,
            )
        };
        assert_ne!(ptr, libc::MAP_FAILED, "mmap failed");

        // XRGB8888 = 0x00RRGGBB
        let pixel: u32 = self.color & 0x00ff_ffff;
        let pixels = unsafe { std::slice::from_raw_parts_mut(ptr as *mut u32, size / 4) };
        pixels.fill(pixel);

        let pool = self.shm.create_pool(unsafe { BorrowedFd::borrow_raw(fd) }, size as i32, qh, ());
        let buffer = pool.create_buffer(
            0,
            self.width as i32,
            self.height as i32,
            (self.width * 4) as i32,
            wl_shm::Format::Xrgb8888,
            qh,
            (),
        );
        self.surface.attach(Some(&buffer), 0, 0);
        self.surface.damage(0, 0, self.width as i32, self.height as i32);
        self.surface.commit();

        buffer.destroy();
        pool.destroy();
        unsafe {
            libc::munmap(ptr, size);
            libc::close(fd);
        }

        if !self.drawn {
            println!("VWTEST ready {}x{}", self.width, self.height);
            use std::io::Write;
            let _ = std::io::stdout().flush();
        }
    }
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
            wl_pointer::Event::Enter { surface_x, surface_y, .. } => {
                log_line(format!("pointer_enter {surface_x:.1} {surface_y:.1}"));
            }
            wl_pointer::Event::Motion { surface_x, surface_y, .. } => {
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

impl Dispatch<wl_keyboard::WlKeyboard, ()> for AppState {
    fn event(
        _state: &mut Self,
        _proxy: &wl_keyboard::WlKeyboard,
        event: wl_keyboard::Event,
        _data: &(),
        _conn: &Connection,
        _qh: &QueueHandle<Self>,
    ) {
        match event {
            wl_keyboard::Event::Enter { .. } => log_line("keyboard_enter".to_string()),
            wl_keyboard::Event::Key { key, state, .. } => {
                log_line(format!("key {key} {state:?}"));
            }
            wl_keyboard::Event::Modifiers { mods_depressed, .. } => {
                log_line(format!("modifiers {mods_depressed}"));
            }
            wl_keyboard::Event::Leave { .. } => log_line("keyboard_leave".to_string()),
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
    let color = std::env::args()
        .nth(1)
        .and_then(|s| u32::from_str_radix(s.trim_start_matches('#'), 16).ok())
        .unwrap_or(0xff0000);

    let conn = Connection::connect_to_env().expect("cannot connect to wayland display");
    let (globals, mut queue) = registry_queue_init::<AppState>(&conn).expect("registry failed");
    let qh = queue.handle();

    let compositor: wl_compositor::WlCompositor = globals.bind(&qh, 1..=4, ()).expect("no compositor");
    let shm: wl_shm::WlShm = globals.bind(&qh, 1..=1, ()).expect("no shm");
    let xdg: xdg_wm_base::XdgWmBase = globals.bind(&qh, 1..=6, ()).expect("no xdg_wm_base");
    let seat: wl_seat::WlSeat = globals.bind(&qh, 1..=7, ()).expect("no seat");
    let _ = &seat; // capability events are handled in Dispatch

    let surface = compositor.create_surface(&qh, ());
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
    };

    // Roundtrip once to receive the seat capabilities event
    queue.roundtrip(&mut state).expect("roundtrip failed");

    loop {
        queue.blocking_dispatch(&mut state).expect("dispatch failed");
    }
}
