//! Headless backend: pixman software rendering, no display server/GPU required.

use pixman::{FormatCode, Image};
use smithay::backend::renderer::damage::OutputDamageTracker;
use smithay::backend::renderer::element::surface::WaylandSurfaceRenderElement;
use smithay::backend::renderer::pixman::PixmanRenderer;
use smithay::backend::renderer::Bind;
use tracing::warn;

use crate::state::Vwayland;

pub const CLEAR_COLOR: [f32; 4] = [0.0, 0.0, 0.0, 1.0];

pub struct Headless {
    pub renderer: PixmanRenderer,
    /// CPU-side render target (A8R8G8B8). Screenshots are read straight from here.
    pub image: Image<'static, 'static>,
    pub damage_tracker: OutputDamageTracker,
}

fn make_image(width: i32, height: i32) -> Result<Image<'static, 'static>, String> {
    Image::new(FormatCode::A8R8G8B8, width as usize, height as usize, true)
        .map_err(|e| format!("failed to create pixman image {width}x{height}: {e:?}"))
}

impl Headless {
    pub fn new(state: &Vwayland) -> Result<Self, String> {
        let size = state.output_size();
        Ok(Self {
            renderer: PixmanRenderer::new().map_err(|e| format!("pixman renderer: {e:?}"))?,
            image: make_image(size.w, size.h)?,
            damage_tracker: OutputDamageTracker::from_output(&state.output),
        })
    }

    /// Recreate the render target to match a new output size
    pub fn resize(&mut self, state: &Vwayland) -> Result<(), String> {
        let size = state.output_size();
        self.image = make_image(size.w, size.h)?;
        self.damage_tracker = OutputDamageTracker::from_output(&state.output);
        Ok(())
    }

    fn render(&mut self, state: &Vwayland, tracker: &mut OutputDamageTracker) -> Result<(), String> {
        let output = state.output.clone();
        let mut target = self
            .renderer
            .bind(&mut self.image)
            .map_err(|e| format!("bind render target: {e:?}"))?;
        smithay::desktop::space::render_output::<
            _,
            WaylandSurfaceRenderElement<PixmanRenderer>,
            _,
            _,
        >(
            &output,
            &mut self.renderer,
            &mut target,
            1.0,
            0,
            [&state.space],
            &[],
            tracker,
            CLEAR_COLOR,
        )
        .map_err(|e| format!("render failed: {e:?}"))?;
        Ok(())
    }

    /// Called from the 60Hz frame timer. Redraws on damage and sends frame callbacks.
    pub fn frame(&mut self, state: &mut Vwayland) {
        let mut tracker = std::mem::replace(
            &mut self.damage_tracker,
            OutputDamageTracker::from_output(&state.output),
        );
        let res = self.render(state, &mut tracker);
        self.damage_tracker = tracker;
        if let Err(e) = res {
            warn!(error = %e, "headless frame render failed");
        }
        state.send_frames();
        state.space.refresh();
        state.popups.cleanup();
    }

    /// Return the current frame as RGBA pixels: (width, height, rgba_bytes)
    pub fn screenshot(&mut self, state: &Vwayland) -> Result<(u32, u32, Vec<u8>), String> {
        // Full redraw (fresh tracker = full damage)
        let mut tracker = OutputDamageTracker::from_output(&state.output);
        self.render(state, &mut tracker)?;

        let w = self.image.width() as u32;
        let h = self.image.height() as u32;
        let stride = self.image.stride(); // bytes per row
        let base = unsafe { self.image.data() } as *const u8;
        if base.is_null() {
            return Err("pixman image has no data".to_string());
        }

        let mut rgba = Vec::with_capacity((w * h * 4) as usize);
        for row in 0..h as usize {
            let row_ptr = unsafe { base.add(row * stride) } as *const u32;
            for col in 0..w as usize {
                // A8R8G8B8 = 0xAARRGGBB
                let px = unsafe { *row_ptr.add(col) };
                rgba.push(((px >> 16) & 0xff) as u8); // R
                rgba.push(((px >> 8) & 0xff) as u8); // G
                rgba.push((px & 0xff) as u8); // B
                rgba.push(((px >> 24) & 0xff) as u8); // A
            }
        }
        Ok((w, h, rgba))
    }
}
