//! Windowed (winit) backend: displayed as a GL window on the host display server.

use smithay::backend::allocator::Fourcc;
use smithay::backend::renderer::damage::OutputDamageTracker;
use smithay::backend::renderer::element::surface::WaylandSurfaceRenderElement;
use smithay::backend::renderer::gles::{GlesRenderer, GlesTexture};
use smithay::backend::renderer::{Bind, ExportMem, Offscreen};
use smithay::backend::winit::{self, WinitEvent, WinitEventLoop, WinitGraphicsBackend};
use smithay::reexports::winit::dpi::LogicalSize;
use smithay::reexports::winit::window::Window;
use smithay::utils::Rectangle;
use tracing::warn;

use crate::headless::CLEAR_COLOR;
use crate::state::Vwayland;
use crate::{Backend, CalloopData};

pub struct Windowed {
    pub backend: WinitGraphicsBackend<GlesRenderer>,
    pub damage_tracker: OutputDamageTracker,
}

impl Windowed {
    pub fn init(
        state: &Vwayland,
        width: i32,
        height: i32,
    ) -> Result<(Self, WinitEventLoop), String> {
        let attributes = Window::default_attributes()
            .with_inner_size(LogicalSize::new(width as f64, height as f64))
            .with_title(format!("vwayland ({})", state.id))
            .with_visible(true);

        let (backend, winit_loop) =
            winit::init_from_attributes::<GlesRenderer>(attributes).map_err(|e| {
                format!(
                    "failed to init winit/GL backend (windowed mode needs a host display server and GL): {e}"
                )
            })?;

        Ok((
            Self {
                backend,
                damage_tracker: OutputDamageTracker::from_output(&state.output),
            },
            winit_loop,
        ))
    }

    /// Render the current frame to an offscreen GL texture and return RGBA pixels.
    pub fn screenshot(&mut self, state: &Vwayland) -> Result<(u32, u32, Vec<u8>), String> {
        let output = state.output.clone();
        let mode = output.current_mode().ok_or("output has no mode")?;
        // Scale 1 and a 180° rotation do not change width/height, so physical == buffer size
        let buf_size: smithay::utils::Size<i32, smithay::utils::Buffer> =
            (mode.size.w, mode.size.h).into();

        let renderer = self.backend.renderer();
        let mut texture =
            Offscreen::<GlesTexture>::create_buffer(renderer, Fourcc::Abgr8888, buf_size)
                .map_err(|e| format!("offscreen buffer: {e:?}"))?;

        let mut target = renderer
            .bind(&mut texture)
            .map_err(|e| format!("bind offscreen: {e:?}"))?;

        let mut tracker = OutputDamageTracker::from_output(&output);
        smithay::desktop::space::render_output::<
            _,
            WaylandSurfaceRenderElement<GlesRenderer>,
            _,
            _,
        >(
            &output,
            renderer,
            &mut target,
            1.0,
            0,
            [&state.space],
            &[],
            &mut tracker,
            CLEAR_COLOR,
        )
        .map_err(|e| format!("render failed: {e:?}"))?;
        drop(target);

        let region = Rectangle::from_size(buf_size);
        let mapping = renderer
            .copy_texture(&texture, region, Fourcc::Abgr8888)
            .map_err(|e| format!("copy texture: {e:?}"))?;
        let bytes = renderer
            .map_texture(&mapping)
            .map_err(|e| format!("map texture: {e:?}"))?;

        // DRM ABGR8888 (u32 = 0xAABBGGRR) is [R,G,B,A] in little-endian memory.
        Ok((buf_size.w as u32, buf_size.h as u32, bytes.to_vec()))
    }
}

/// winit event loop callback
pub fn handle_event(event: WinitEvent, data: &mut CalloopData) {
    match event {
        WinitEvent::Resized { size, .. } => {
            data.state.set_output_size(size.w, size.h);
        }
        WinitEvent::Input(event) => data.state.process_input_event(event),
        WinitEvent::Redraw => {
            let Backend::Windowed(w) = &mut data.backend else {
                return;
            };
            let output = data.state.output.clone();

            let render_res = {
                match w.backend.bind() {
                    Ok((renderer, mut framebuffer)) => {
                        let res = smithay::desktop::space::render_output::<
                            _,
                            WaylandSurfaceRenderElement<GlesRenderer>,
                            _,
                            _,
                        >(
                            &output,
                            renderer,
                            &mut framebuffer,
                            1.0,
                            0,
                            [&data.state.space],
                            &[],
                            &mut w.damage_tracker,
                            CLEAR_COLOR,
                        );
                        res.map(|r| r.damage)
                    }
                    Err(e) => {
                        warn!(error = ?e, "winit bind failed");
                        return;
                    }
                }
            };
            // Borrows of the bind products end in the block above.
            match render_res {
                Ok(Some(damage)) if !damage.is_empty() => {
                    if let Err(e) = w.backend.submit(Some(&damage)) {
                        warn!(error = ?e, "winit submit failed");
                    }
                }
                Ok(_) => {}
                Err(e) => {
                    warn!(error = ?e, "winit render failed");
                }
            }

            data.state.send_frames();
            data.state.space.refresh();
            data.state.popups.cleanup();
            let _ = data.display_handle.flush_clients();

            // Schedule the next frame
            if let Backend::Windowed(w) = &mut data.backend {
                w.backend.window().request_redraw();
            }
        }
        WinitEvent::CloseRequested => {
            data.state.loop_signal.stop();
        }
        _ => (),
    }
}
