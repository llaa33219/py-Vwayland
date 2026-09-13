//! Compositor state: Wayland display, smithay global state, output, app process tracking.

use std::collections::HashMap;
use std::ffi::OsString;
use std::fs::OpenOptions;
use std::path::{Path, PathBuf};
use std::process::{Child, Command, Stdio};
use std::sync::Arc;
use std::time::{Duration, Instant};

use smithay::desktop::{PopupManager, Space, Window, WindowSurfaceType};
use smithay::input::{Seat, SeatState};
use smithay::output::{Mode, Output, PhysicalProperties, Subpixel};
use smithay::reexports::calloop::{generic::Generic, EventLoop, Interest, LoopSignal, Mode as SourceMode, PostAction};
use smithay::reexports::wayland_server::backend::{ClientData, ClientId, DisconnectReason};
use smithay::reexports::wayland_server::protocol::wl_surface::WlSurface;
use smithay::reexports::wayland_server::{Display, DisplayHandle};
use smithay::utils::{Logical, Point, Size, Transform, SERIAL_COUNTER};
use smithay::wayland::compositor::{CompositorClientState, CompositorState};
use smithay::wayland::output::OutputManagerState;
use smithay::wayland::selection::data_device::DataDeviceState;
use smithay::wayland::shell::xdg::{ToplevelSurface, XdgShellState};
use smithay::wayland::shm::ShmState;
use smithay::wayland::socket::ListeningSocketSource;
use smithay::reexports::wayland_protocols::xdg::shell::server::xdg_toplevel;
use tracing::{info, warn};

use crate::CalloopData;

pub struct AppProc {
    pub child: Child,
    pub term_deadline: Option<Instant>,
}

pub struct Vwayland {
    pub id: String,
    pub runtime_dir: PathBuf,
    pub headless: bool,
    pub start_time: Instant,
    pub socket_name: OsString,
    pub display_handle: DisplayHandle,
    pub loop_signal: LoopSignal,

    pub space: Space<Window>,
    pub output: Output,
    pub popups: PopupManager,
    pub app: Option<AppProc>,

    // Smithay global state
    pub compositor_state: CompositorState,
    pub xdg_shell_state: XdgShellState,
    pub shm_state: ShmState,
    pub output_manager_state: OutputManagerState,
    pub seat_state: SeatState<Vwayland>,
    pub data_device_state: DataDeviceState,

    pub seat: Seat<Self>,
}

impl Vwayland {
    pub fn new(
        event_loop: &mut EventLoop<CalloopData>,
        display: Display<Self>,
        id: &str,
        runtime_dir: &Path,
        width: i32,
        height: i32,
        headless: bool,
    ) -> Self {
        let start_time = Instant::now();
        let dh = display.handle();

        let compositor_state = CompositorState::new::<Self>(&dh);
        let xdg_shell_state = XdgShellState::new::<Self>(&dh);
        let shm_state = ShmState::new::<Self>(&dh, vec![]);
        let output_manager_state = OutputManagerState::new_with_xdg_output::<Self>(&dh);
        let mut seat_state = SeatState::new();
        let data_device_state = DataDeviceState::new::<Self>(&dh);
        let popups = PopupManager::default();

        let mut seat: Seat<Self> = seat_state.new_wl_seat(&dh, "vwayland");
        seat.add_keyboard(Default::default(), 200, 25).unwrap();
        seat.add_pointer();

        let mut space = Space::default();

        // The single virtual output of this compositor
        let output = Output::new(
            "VWAYLAND-1".to_string(),
            PhysicalProperties {
                size: (0, 0).into(),
                subpixel: Subpixel::Unknown,
                make: "py-Vwayland".into(),
                model: "Virtual".into(),
            },
        );
        let mode = Mode {
            size: (width, height).into(),
            refresh: 60_000,
        };
        let _global = output.create_global::<Self>(&dh);
        // The winit (GL) mode comes out Y-flipped, hence Flipped180; headless is Normal.
        let transform = if headless { Transform::Normal } else { Transform::Flipped180 };
        output.change_current_state(Some(mode), Some(transform), None, Some((0, 0).into()));
        output.set_preferred(mode);
        space.map_output(&output, (0, 0));

        let socket_name = Self::init_wayland_listener(display, event_loop);
        let loop_signal = event_loop.get_signal();

        Self {
            id: id.to_string(),
            runtime_dir: runtime_dir.to_path_buf(),
            headless,
            start_time,
            socket_name,
            display_handle: dh,
            loop_signal,
            space,
            output,
            popups,
            app: None,
            compositor_state,
            xdg_shell_state,
            shm_state,
            output_manager_state,
            seat_state,
            data_device_state,
            seat,
        }
    }

    fn init_wayland_listener(
        display: Display<Vwayland>,
        event_loop: &mut EventLoop<CalloopData>,
    ) -> OsString {
        let listening_socket = ListeningSocketSource::new_auto().unwrap();
        let socket_name = listening_socket.socket_name().to_os_string();
        let loop_handle = event_loop.handle();

        loop_handle
            .insert_source(listening_socket, move |client_stream, _, data| {
                data.display_handle
                    .insert_client(client_stream, Arc::new(ClientState::default()))
                    .unwrap();
            })
            .expect("Failed to init the wayland event source.");

        loop_handle
            .insert_source(
                Generic::new(display, Interest::READ, SourceMode::Level),
                |_, display, data| {
                    // Safety: the display is not dropped before the loop ends.
                    unsafe {
                        display.get_mut().dispatch_clients(&mut data.state).unwrap();
                    }
                    Ok(PostAction::Continue)
                },
            )
            .unwrap();

        socket_name
    }

    /// Current logical output size (scale 1 assumed)
    pub fn output_size(&self) -> Size<i32, Logical> {
        self.output
            .current_mode()
            .map(|m| m.size.to_logical(1))
            .unwrap_or((1280, 720).into())
    }

    /// Change the output size and reconfigure every open toplevel to the new
    /// fullscreen size.
    pub fn set_output_size(&mut self, width: i32, height: i32) {
        let mode = Mode {
            size: (width, height).into(),
            refresh: 60_000,
        };
        self.output.change_current_state(Some(mode), None, None, None);
        self.output.set_preferred(mode);

        let size: Size<i32, Logical> = (width, height).into();
        for window in self.space.elements() {
            if let Some(toplevel) = window.toplevel() {
                toplevel.with_pending_state(|state| {
                    state.size = Some(size);
                });
                toplevel.send_pending_configure();
            }
        }
    }

    /// Map a new toplevel fullscreen at (0,0) and give it keyboard focus.
    pub fn map_toplevel_fullscreen(&mut self, surface: &ToplevelSurface) {
        let size = self.output_size();
        surface.with_pending_state(|state| {
            state.size = Some(size);
            state.states.set(xdg_toplevel::State::Fullscreen);
        });
        surface.send_configure();

        let serial = SERIAL_COUNTER.next_serial();
        if let Some(keyboard) = self.seat.get_keyboard() {
            keyboard.set_focus(self, Some(surface.wl_surface().clone()), serial);
        }
    }

    pub fn surface_under(&self, pos: Point<f64, Logical>) -> Option<(WlSurface, Point<f64, Logical>)> {
        self.space.element_under(pos).and_then(|(window, location)| {
            window
                .surface_under(pos - location.to_f64(), WindowSurfaceType::ALL)
                .map(|(s, p)| (s, (p + location).to_f64()))
        })
    }

    /// Send frame callbacks (so clients keep drawing)
    pub fn send_frames(&mut self) {
        let output = self.output.clone();
        let now = self.start_time.elapsed();
        self.space.elements().for_each(|window| {
            window.send_frame(&output, now, Some(Duration::ZERO), |_, _| Some(output.clone()));
        });
    }

    // ---- App process management ----

    pub fn app_pid(&mut self) -> Option<u32> {
        self.reap_app();
        self.app.as_ref().map(|a| a.child.id())
    }

    pub fn launch_app(
        &mut self,
        argv: &[String],
        env: &HashMap<String, String>,
        cwd: Option<&str>,
    ) -> Result<u32, String> {
        self.reap_app();
        if self.app.is_some() {
            return Err("an app is already running in this compositor".to_string());
        }
        if argv.is_empty() || argv[0].is_empty() {
            return Err("empty argv".to_string());
        }

        let log = OpenOptions::new()
            .create(true)
            .append(true)
            .open(self.runtime_dir.join("app.log"))
            .map_err(|e| format!("cannot open app.log: {e}"))?;
        let log_err = log.try_clone().map_err(|e| e.to_string())?;

        let mut cmd = Command::new(&argv[0]);
        cmd.args(&argv[1..])
            .env("WAYLAND_DISPLAY", &self.socket_name)
            .env("XDG_RUNTIME_DIR", &self.runtime_dir)
            .env_remove("DISPLAY") // prevent X11 fallback (no XWayland support)
            .envs(env)
            .stdin(Stdio::null())
            .stdout(Stdio::from(log))
            .stderr(Stdio::from(log_err));
        if let Some(dir) = cwd {
            cmd.current_dir(dir);
        }

        let child = cmd
            .spawn()
            .map_err(|e| format!("failed to spawn '{}': {e}", argv[0]))?;
        let pid = child.id();
        info!(pid, argv = ?argv, "launched app");
        self.app = Some(AppProc {
            child,
            term_deadline: None,
        });
        Ok(pid)
    }

    /// Send SIGTERM and escalate to SIGKILL after a 3s grace period (handled by
    /// reap_app). Returns false if no app was running.
    pub fn close_app(&mut self) -> bool {
        self.reap_app();
        match &mut self.app {
            Some(app) => {
                unsafe {
                    libc::kill(app.child.id() as i32, libc::SIGTERM);
                }
                app.term_deadline = Some(Instant::now() + Duration::from_secs(3));
                true
            }
            None => false,
        }
    }

    /// Reap exited children + SIGKILL when the SIGTERM grace period expired.
    pub fn reap_app(&mut self) {
        let mut clear = false;
        if let Some(app) = &mut self.app {
            match app.child.try_wait() {
                Ok(Some(status)) => {
                    info!(?status, "app exited");
                    clear = true;
                }
                Ok(None) => {
                    if let Some(deadline) = app.term_deadline {
                        if Instant::now() >= deadline {
                            warn!("app did not exit on SIGTERM; sending SIGKILL");
                            let _ = app.child.kill();
                            let _ = app.child.wait();
                            clear = true;
                        }
                    }
                }
                Err(e) => {
                    warn!(?e, "failed to poll app status");
                    clear = true;
                }
            }
        }
        if clear {
            self.app = None;
        }
    }
}

#[derive(Default)]
pub struct ClientState {
    pub compositor_state: CompositorClientState,
}

impl ClientData for ClientState {
    fn initialized(&self, _client_id: ClientId) {}
    fn disconnected(&self, _client_id: ClientId, _reason: DisconnectReason) {}
}
