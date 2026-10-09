//! vwayland-compositor: the virtual Wayland compositor for py-Vwayland.
//!
//! Runs in headless (pixman software rendering) or windowed (winit + GL) mode,
//! controlled over a Unix socket (ipc.sock) with a JSON-line protocol.

mod clipboard;
mod handlers;
mod headless;
mod inject;
mod ipc;
mod state;
mod windowed;

use std::path::PathBuf;
use std::time::Duration;

use smithay::reexports::calloop::{
    timer::{TimeoutAction, Timer},
    EventLoop,
};
use smithay::reexports::wayland_server::{Display, DisplayHandle};
use tracing::{error, info};

use crate::state::Vwayland;

pub const VERSION: &str = env!("CARGO_PKG_VERSION");

/// Top-level data shared by the event loop callbacks.
pub struct CalloopData {
    pub state: Vwayland,
    pub display_handle: DisplayHandle,
    pub backend: Backend,
}

pub enum Backend {
    Headless(headless::Headless),
    Windowed(windowed::Windowed),
}

struct Args {
    id: String,
    runtime_dir: PathBuf,
    width: i32,
    height: i32,
    headless: bool,
}

fn parse_args() -> Result<Args, String> {
    let mut id = None;
    let mut runtime_dir = None;
    let mut width = None;
    let mut height = None;
    let mut headless = true; // headless is the default

    let mut it = std::env::args().skip(1);
    while let Some(arg) = it.next() {
        match arg.as_str() {
            "--id" => id = Some(it.next().ok_or("--id requires a value")?),
            "--runtime-dir" => runtime_dir = Some(PathBuf::from(it.next().ok_or("--runtime-dir requires a value")?)),
            "--width" => width = Some(it.next().ok_or("--width requires a value")?.parse().map_err(|_| "invalid --width")?),
            "--height" => height = Some(it.next().ok_or("--height requires a value")?.parse().map_err(|_| "invalid --height")?),
            "--headless" => headless = true,
            "--windowed" => headless = false,
            "--version" | "-V" => {
                println!("vwayland-compositor {VERSION}");
                std::process::exit(0);
            }
            "--help" | "-h" => {
                println!("vwayland-compositor {VERSION}");
                println!("usage: vwayland-compositor --id ID --runtime-dir DIR [--width W] [--height H] [--headless|--windowed]");
                std::process::exit(0);
            }
            other => return Err(format!("unknown argument: {other}")),
        }
    }

    Ok(Args {
        id: id.ok_or("missing --id")?,
        runtime_dir: runtime_dir.ok_or("missing --runtime-dir")?,
        width: width.unwrap_or(1280),
        height: height.unwrap_or(720),
        headless,
    })
}

fn main() -> Result<(), Box<dyn std::error::Error>> {
    tracing_subscriber::fmt()
        .with_env_filter(
            tracing_subscriber::EnvFilter::try_from_default_env()
                .unwrap_or_else(|_| tracing_subscriber::EnvFilter::new("info")),
        )
        .init();

    let args = parse_args().map_err(|e| format!("argument error: {e}"))?;

    // Prepare the instance runtime directory. The wayland socket and ipc.sock
    // are created here.
    std::fs::create_dir_all(&args.runtime_dir)?;
    {
        use std::os::unix::fs::PermissionsExt;
        std::fs::set_permissions(&args.runtime_dir, std::fs::Permissions::from_mode(0o700))?;
    }
    // Changing XDG_RUNTIME_DIR would hide the host Wayland display from winit,
    // so pin WAYLAND_DISPLAY to an absolute path to preserve the host
    // connection. (X11's DISPLAY works regardless of XDG_RUNTIME_DIR.)
    if let (Ok(xdg), Ok(wd)) = (std::env::var("XDG_RUNTIME_DIR"), std::env::var("WAYLAND_DISPLAY")) {
        if !std::path::Path::new(&wd).is_absolute() {
            std::env::set_var("WAYLAND_DISPLAY", std::path::Path::new(&xdg).join(&wd));
        }
    }
    // Make ListeningSocketSource create its wayland-N socket in this directory.
    std::env::set_var("XDG_RUNTIME_DIR", &args.runtime_dir);

    let mut event_loop: EventLoop<CalloopData> = EventLoop::try_new()?;
    let display: Display<Vwayland> = Display::new()?;
    let display_handle = display.handle();

    let state = Vwayland::new(
        &mut event_loop,
        display,
        &args.id,
        &args.runtime_dir,
        args.width,
        args.height,
        args.headless,
    );

    let backend = if args.headless {
        Backend::Headless(headless::Headless::new(&state)?)
    } else {
        let (windowed, winit_loop) = windowed::Windowed::init(&state, args.width, args.height)?;
        event_loop
            .handle()
            .insert_source(winit_loop, move |event, _, data| {
                windowed::handle_event(event, data);
            })?;
        Backend::Windowed(windowed)
    };

    let mut data = CalloopData {
        state,
        display_handle,
        backend,
    };

    ipc::init(&mut event_loop, &args.runtime_dir.join("ipc.sock"))?;

    // Reaper timer for the child (app) process
    event_loop
        .handle()
        .insert_source(Timer::immediate(), |_, _, data| {
            data.state.reap_app();
            TimeoutAction::ToDuration(Duration::from_millis(200))
        })?;

    // Headless frame timer: clients must receive frame callbacks to keep drawing.
    event_loop
        .handle()
        .insert_source(Timer::immediate(), |_, _, data| {
            if let Backend::Headless(h) = &mut data.backend {
                h.frame(&mut data.state);
                let _ = data.display_handle.flush_clients();
            }
            TimeoutAction::ToDuration(Duration::from_millis(16))
        })?;

    std::fs::write(
        args.runtime_dir.join("compositor.pid"),
        std::process::id().to_string(),
    )
    .ok();

    info!(
        id = %args.id,
        headless = args.headless,
        width = args.width,
        height = args.height,
        "vwayland-compositor ready"
    );

    if let Err(e) = event_loop.run(None, &mut data, |_| {}) {
        error!(?e, "event loop terminated with error");
    }

    // Shutdown cleanup
    let _ = std::fs::remove_file(args.runtime_dir.join("ipc.sock"));
    let _ = std::fs::remove_file(args.runtime_dir.join("compositor.pid"));
    info!("vwayland-compositor exited");
    Ok(())
}
