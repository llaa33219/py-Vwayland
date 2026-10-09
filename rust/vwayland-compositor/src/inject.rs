//! Input injection: synthesizes pointer/keyboard events received over IPC
//! directly into the seat. In winit windowed mode, real host input events are
//! also processed here.

use smithay::backend::input::{
    AbsolutePositionEvent, Axis, AxisSource, ButtonState, Event, InputBackend, InputEvent,
    KeyState, KeyboardKeyEvent, PointerAxisEvent, PointerButtonEvent,
};
use smithay::input::keyboard::FilterResult;
use smithay::input::pointer::{AxisFrame, ButtonEvent, MotionEvent};
use smithay::reexports::wayland_server::protocol::wl_surface::WlSurface;
use smithay::utils::{Logical, Point, SERIAL_COUNTER};

use crate::state::Vwayland;

impl Vwayland {
    fn now_msec(&self) -> u32 {
        self.start_time.elapsed().as_millis() as u32
    }

    fn clamp_to_output(&self, pos: Point<f64, Logical>) -> Point<f64, Logical> {
        let size = self.output_size();
        let w = (size.w - 1).max(0) as f64;
        let h = (size.h - 1).max(0) as f64;
        (pos.x.clamp(0.0, w), pos.y.clamp(0.0, h)).into()
    }

    /// IPC: move the pointer to absolute coordinates
    pub fn inject_pointer_move(&mut self, x: f64, y: f64) {
        let pos = self.clamp_to_output((x, y).into());
        let serial = SERIAL_COUNTER.next_serial();
        let time = self.now_msec();
        let under = self.surface_under(pos);
        let pointer = self.seat.get_pointer().unwrap();
        pointer.motion(self, under, &MotionEvent { location: pos, serial, time });
        pointer.frame(self);
    }

    /// IPC: pointer button (evdev button code, e.g. BTN_LEFT = 0x110)
    pub fn inject_pointer_button(&mut self, button: u32, pressed: bool) {
        let serial = SERIAL_COUNTER.next_serial();
        let time = self.now_msec();
        let state = if pressed { ButtonState::Pressed } else { ButtonState::Released };

        if pressed {
            self.focus_window_under_pointer(serial);
        }

        let pointer = self.seat.get_pointer().unwrap();
        pointer.button(self, &ButtonEvent { button, state, serial, time });
        pointer.frame(self);
    }

    /// IPC: scroll (unit: wheel detents; positive dy = up)
    pub fn inject_pointer_axis(&mut self, dx: f64, dy: f64) {
        let time = self.now_msec();
        let mut frame = AxisFrame::new(time).source(AxisSource::Wheel);
        if dx != 0.0 {
            frame = frame
                .value(Axis::Horizontal, dx * 15.0)
                .v120(Axis::Horizontal, (dx * 120.0) as i32);
        }
        if dy != 0.0 {
            frame = frame
                .value(Axis::Vertical, dy * 15.0)
                .v120(Axis::Vertical, (dy * 120.0) as i32);
        }
        let pointer = self.seat.get_pointer().unwrap();
        pointer.axis(self, frame);
        pointer.frame(self);
    }

    /// IPC: key input (the protocol uses evdev key codes; smithay expects xkb
    /// codes (+8), so convert here)
    pub fn inject_key(&mut self, code: u32, pressed: bool) {
        let serial = SERIAL_COUNTER.next_serial();
        let time = self.now_msec();
        let state = if pressed { KeyState::Pressed } else { KeyState::Released };
        self.seat.get_keyboard().unwrap().input::<(), _>(
            self,
            (code + 8).into(),
            state,
            serial,
            time,
            |_, _, _| FilterResult::Forward,
        );
    }

    /// Inject a key event using an **xkb** keycode (no evdev +8 conversion).
    ///
    /// Used by the typing engine, which owns both sides of the temporary
    /// keymap it installs, so the scratch keycodes it produces are already in
    /// the convention `input()` expects.
    pub fn inject_xkb_key(&mut self, keycode: u32, pressed: bool) {
        let serial = SERIAL_COUNTER.next_serial();
        let time = self.now_msec();
        let state = if pressed { KeyState::Pressed } else { KeyState::Released };
        self.seat.get_keyboard().unwrap().input::<(), _>(
            self,
            keycode.into(),
            state,
            serial,
            time,
            |_, _, _| FilterResult::Forward,
        );
    }

    /// Give keyboard focus to the window under the pointer on click
    fn focus_window_under_pointer(&mut self, serial: smithay::utils::Serial) {
        let pointer = self.seat.get_pointer().unwrap();
        if pointer.is_grabbed() {
            return;
        }
        let keyboard = self.seat.get_keyboard().unwrap();
        if let Some((window, _loc)) = self
            .space
            .element_under(pointer.current_location())
            .map(|(w, l)| (w.clone(), l))
        {
            self.space.raise_element(&window, true);
            keyboard.set_focus(
                self,
                Some(window.toplevel().unwrap().wl_surface().clone()),
                serial,
            );
            self.space.elements().for_each(|window| {
                window.toplevel().unwrap().send_pending_configure();
            });
        } else {
            keyboard.set_focus(self, Option::<WlSurface>::None, serial);
        }
    }

    /// Windowed mode: process real input events coming from the host
    pub fn process_input_event<I: InputBackend>(&mut self, event: InputEvent<I>) {
        match event {
            InputEvent::Keyboard { event, .. } => {
                let serial = SERIAL_COUNTER.next_serial();
                let time = Event::time_msec(&event);
                self.seat.get_keyboard().unwrap().input::<(), _>(
                    self,
                    event.key_code(),
                    event.state(),
                    serial,
                    time,
                    |_, _, _| FilterResult::Forward,
                );
            }
            InputEvent::PointerMotion { .. } => {}
            InputEvent::PointerMotionAbsolute { event, .. } => {
                let output_geo = self.space.output_geometry(&self.output).unwrap();
                let pos = event.position_transformed(output_geo.size) + output_geo.loc.to_f64();
                let serial = SERIAL_COUNTER.next_serial();
                let pointer = self.seat.get_pointer().unwrap();
                let under = self.surface_under(pos);
                pointer.motion(
                    self,
                    under,
                    &MotionEvent {
                        location: pos,
                        serial,
                        time: event.time_msec(),
                    },
                );
                pointer.frame(self);
            }
            InputEvent::PointerButton { event, .. } => {
                let serial = SERIAL_COUNTER.next_serial();
                if ButtonState::Pressed == event.state() {
                    self.focus_window_under_pointer(serial);
                }
                let pointer = self.seat.get_pointer().unwrap();
                pointer.button(
                    self,
                    &ButtonEvent {
                        button: event.button_code(),
                        state: event.state(),
                        serial,
                        time: event.time_msec(),
                    },
                );
                pointer.frame(self);
            }
            InputEvent::PointerAxis { event, .. } => {
                let source = event.source();
                let horizontal_amount = event
                    .amount(Axis::Horizontal)
                    .unwrap_or_else(|| event.amount_v120(Axis::Horizontal).unwrap_or(0.0) * 15.0 / 120.);
                let vertical_amount = event
                    .amount(Axis::Vertical)
                    .unwrap_or_else(|| event.amount_v120(Axis::Vertical).unwrap_or(0.0) * 15.0 / 120.);
                let horizontal_amount_discrete = event.amount_v120(Axis::Horizontal);
                let vertical_amount_discrete = event.amount_v120(Axis::Vertical);

                let mut frame = AxisFrame::new(event.time_msec()).source(source);
                if horizontal_amount != 0.0 {
                    frame = frame.value(Axis::Horizontal, horizontal_amount);
                    if let Some(discrete) = horizontal_amount_discrete {
                        frame = frame.v120(Axis::Horizontal, discrete as i32);
                    }
                }
                if vertical_amount != 0.0 {
                    frame = frame.value(Axis::Vertical, vertical_amount);
                    if let Some(discrete) = vertical_amount_discrete {
                        frame = frame.v120(Axis::Vertical, discrete as i32);
                    }
                }
                if source == AxisSource::Finger {
                    if event.amount(Axis::Horizontal) == Some(0.0) {
                        frame = frame.stop(Axis::Horizontal);
                    }
                    if event.amount(Axis::Vertical) == Some(0.0) {
                        frame = frame.stop(Axis::Vertical);
                    }
                }

                let pointer = self.seat.get_pointer().unwrap();
                pointer.axis(self, frame);
                pointer.frame(self);
            }
            _ => {}
        }
    }
}
