//! Smithay protocol handler implementations (compositor / xdg_shell / seat / shm / output / data_device).

use smithay::backend::renderer::utils::on_commit_buffer_handler;
use smithay::desktop::{
    find_popup_root_surface, get_popup_toplevel_coords, PopupKind, PopupManager, Space, Window,
};
use smithay::input::pointer::CursorImageStatus;
use smithay::input::{Seat, SeatHandler, SeatState};
use smithay::reexports::wayland_protocols::xdg::shell::server::xdg_toplevel;
use smithay::reexports::wayland_server::protocol::{wl_buffer, wl_seat, wl_surface::WlSurface};
use smithay::reexports::wayland_server::{Client, Resource};
use smithay::utils::Serial;
use smithay::wayland::buffer::BufferHandler;
use smithay::wayland::compositor::{
    get_parent, is_sync_subsurface, with_states, CompositorClientState, CompositorHandler,
    CompositorState,
};
use smithay::wayland::output::OutputHandler;
use smithay::wayland::selection::data_device::{
    set_data_device_focus, ClientDndGrabHandler, DataDeviceHandler, DataDeviceState,
    ServerDndGrabHandler,
};
use smithay::wayland::selection::{SelectionHandler, SelectionSource, SelectionTarget};
use smithay::wayland::shell::xdg::{
    PopupSurface, PositionerState, ToplevelSurface, XdgShellHandler, XdgShellState,
    XdgToplevelSurfaceData,
};
use smithay::wayland::shm::{ShmHandler, ShmState};
use smithay::{
    delegate_compositor, delegate_data_device, delegate_output, delegate_seat, delegate_shm,
    delegate_xdg_shell,
};
use std::os::unix::io::OwnedFd;
use std::sync::Arc;
use tracing::warn;

use crate::state::{ClientState, Vwayland};

//
// Compositor
//

impl CompositorHandler for Vwayland {
    fn compositor_state(&mut self) -> &mut CompositorState {
        &mut self.compositor_state
    }

    fn client_compositor_state<'a>(&self, client: &'a Client) -> &'a CompositorClientState {
        &client.get_data::<ClientState>().unwrap().compositor_state
    }

    fn commit(&mut self, surface: &WlSurface) {
        on_commit_buffer_handler::<Self>(surface);
        if !is_sync_subsurface(surface) {
            let mut root = surface.clone();
            while let Some(parent) = get_parent(&root) {
                root = parent;
            }
            if let Some(window) = self
                .space
                .elements()
                .find(|w| w.toplevel().unwrap().wl_surface() == &root)
            {
                window.on_commit();
            }
        };
        handle_commit(&mut self.popups, &self.space, surface);
    }
}

impl BufferHandler for Vwayland {
    fn buffer_destroyed(&mut self, _buffer: &wl_buffer::WlBuffer) {}
}

impl ShmHandler for Vwayland {
    fn shm_state(&self) -> &ShmState {
        &self.shm_state
    }
}

delegate_compositor!(Vwayland);
delegate_shm!(Vwayland);

//
// XDG Shell
//

impl XdgShellHandler for Vwayland {
    fn xdg_shell_state(&mut self) -> &mut XdgShellState {
        &mut self.xdg_shell_state
    }

    fn new_toplevel(&mut self, surface: ToplevelSurface) {
        let window = Window::new_wayland_window(surface.clone());
        self.space.map_element(window, (0, 0), true);
        self.map_toplevel_fullscreen(&surface);
    }

    fn new_popup(&mut self, surface: PopupSurface, _positioner: PositionerState) {
        self.unconstrain_popup(&surface);
        let _ = self.popups.track_popup(PopupKind::Xdg(surface));
    }

    fn reposition_request(&mut self, surface: PopupSurface, positioner: PositionerState, token: u32) {
        surface.with_pending_state(|state| {
            let geometry = positioner.get_geometry();
            state.geometry = geometry;
            state.positioner = positioner;
        });
        self.unconstrain_popup(&surface);
        surface.send_repositioned(token);
    }

    fn move_request(&mut self, _surface: ToplevelSurface, _seat: wl_seat::WlSeat, _serial: Serial) {
        // Fullscreen single-app policy: moving is not allowed
    }

    fn resize_request(
        &mut self,
        _surface: ToplevelSurface,
        _seat: wl_seat::WlSeat,
        _serial: Serial,
        _edges: xdg_toplevel::ResizeEdge,
    ) {
        // Fullscreen single-app policy: resizing is only done via the IPC resize command
    }

    fn grab(&mut self, _surface: PopupSurface, _seat: wl_seat::WlSeat, _serial: Serial) {
        // TODO: popup grabs (e.g. close menus on outside click). Not supported in v1.
    }
}

delegate_xdg_shell!(Vwayland);

/// Called on WlSurface commit (guarantees the initial configure, handles popup commits)
pub fn handle_commit(popups: &mut PopupManager, space: &Space<Window>, surface: &WlSurface) {
    if let Some(window) = space
        .elements()
        .find(|w| w.toplevel().unwrap().wl_surface() == surface)
        .cloned()
    {
        let initial_configure_sent = with_states(surface, |states| {
            states
                .data_map
                .get::<XdgToplevelSurfaceData>()
                .unwrap()
                .lock()
                .unwrap()
                .initial_configure_sent
        });

        if !initial_configure_sent {
            window.toplevel().unwrap().send_configure();
        }
    }

    popups.commit(surface);
    if let Some(popup) = popups.find_popup(surface) {
        match popup {
            PopupKind::Xdg(ref xdg) => {
                if !xdg.is_initial_configure_sent() {
                    xdg.send_configure().expect("initial configure failed");
                }
            }
            PopupKind::InputMethod(ref _input_method) => {}
        }
    }
}

impl Vwayland {
    fn unconstrain_popup(&self, popup: &PopupSurface) {
        let Ok(root) = find_popup_root_surface(&PopupKind::Xdg(popup.clone())) else {
            return;
        };
        let Some(window) = self
            .space
            .elements()
            .find(|w| w.toplevel().unwrap().wl_surface() == &root)
        else {
            return;
        };

        let output = self.output.clone();
        let output_geo = self.space.output_geometry(&output).unwrap();
        let window_geo = self.space.element_geometry(window).unwrap();

        let mut target = output_geo;
        target.loc -= get_popup_toplevel_coords(&PopupKind::Xdg(popup.clone()));
        target.loc -= window_geo.loc;

        popup.with_pending_state(|state| {
            state.geometry = state.positioner.get_unconstrained_geometry(target);
        });
    }
}

//
// Seat
//

impl SeatHandler for Vwayland {
    type KeyboardFocus = WlSurface;
    type PointerFocus = WlSurface;
    type TouchFocus = WlSurface;

    fn seat_state(&mut self) -> &mut SeatState<Vwayland> {
        &mut self.seat_state
    }

    fn cursor_image(&mut self, _seat: &Seat<Self>, _image: CursorImageStatus) {
        // v1: no cursor image rendering (input events work normally)
    }

    fn focus_changed(&mut self, seat: &Seat<Self>, focused: Option<&WlSurface>) {
        let dh = &self.display_handle;
        let client = focused.and_then(|s| dh.get_client(s.id()).ok());
        set_data_device_focus(dh, seat, client);
        self.text_input_state.focus_changed(focused);
    }
}

delegate_seat!(Vwayland);

//
// Data Device (clipboard selection; DnD grabs are bound but unused)
//

impl SelectionHandler for Vwayland {
    /// Payload of a compositor-owned clipboard selection: the raw UTF-8 bytes
    /// handed to the app by `send_selection`.
    type SelectionUserData = Arc<[u8]>;

    fn new_selection(
        &mut self,
        _ty: SelectionTarget,
        _source: Option<SelectionSource>,
        _seat: Seat<Self>,
    ) {
        // Observation hook for app-owned selections. The text itself is read on
        // demand (clipboard_get), so there is nothing to track here.
    }

    fn send_selection(
        &mut self,
        _ty: SelectionTarget,
        _mime_type: String,
        fd: OwnedFd,
        _seat: Seat<Self>,
        user_data: &Arc<[u8]>,
    ) {
        let data = user_data.clone();
        // This runs on the event loop, so the write must not happen here: a
        // blocking write to a client that never reads would freeze the whole
        // compositor.
        std::thread::spawn(move || {
            use std::io::Write;
            let mut file = std::fs::File::from(fd);
            // Wayland clients pass O_NONBLOCK fds in wl_data_source.send.
            // write_all() then bails out after a partial write, silently
            // truncating the selection (same bug class as WayVR's), so the flag
            // is cleared first.
            clear_nonblocking(&file);
            if let Err(e) = file.write_all(&data) {
                warn!(error = %e, "clipboard write to client failed");
            }
        });
    }
}

/// Clear `O_NONBLOCK` on `file` (see `send_selection`).
fn clear_nonblocking(file: &std::fs::File) {
    use std::os::unix::io::AsRawFd;
    let fd = file.as_raw_fd();
    // SAFETY: fcntl with F_GETFL/F_SETFL only reads and writes the fd's status
    // flags; both calls are infallible on a valid fd.
    let flags = unsafe { libc::fcntl(fd, libc::F_GETFL) };
    if flags >= 0 {
        unsafe { libc::fcntl(fd, libc::F_SETFL, flags & !libc::O_NONBLOCK) };
    }
}

impl DataDeviceHandler for Vwayland {
    fn data_device_state(&self) -> &DataDeviceState {
        &self.data_device_state
    }
}

impl ClientDndGrabHandler for Vwayland {}
impl ServerDndGrabHandler for Vwayland {}

delegate_data_device!(Vwayland);

//
// Output
//

impl OutputHandler for Vwayland {}
delegate_output!(Vwayland);
