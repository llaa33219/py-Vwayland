//! `zwp_text_input_v3` support for the test client (layer C observability).
//!
//! The compositor's typing engine inserts text through `commit_string` on the
//! text input it considers enabled, which it detects from a `commit` following
//! an `enable`. The client side therefore behaves like a minimal editable
//! widget: enable itself when focused, log whatever the IME commits.
//!
//! `enable`/`disable` are double-buffered in this protocol — they only take
//! effect on the next `commit` — so both are always followed by a `commit`.
//! Nothing else is ever sent: a `commit` is only required after new state, and
//! there is no other state to set.

use wayland_client::{event_created_child, Connection, Dispatch, QueueHandle};
use wayland_protocols::wp::text_input::zv3::client::{
    zwp_text_input_manager_v3, zwp_text_input_v3,
};

pub struct TextInput {
    /// Whether the current text input has an `enable` pending or applied.
    enabled: bool,
}

impl TextInput {
    pub fn new() -> Self {
        Self { enabled: false }
    }

    fn on_enter(&mut self, proxy: &zwp_text_input_v3::ZwpTextInputV3) {
        if self.enabled {
            return;
        }
        self.enabled = true;
        proxy.enable();
        proxy.commit();
    }

    fn on_leave(&mut self, proxy: &zwp_text_input_v3::ZwpTextInputV3) {
        if !self.enabled {
            return;
        }
        self.enabled = false;
        proxy.disable();
        proxy.commit();
    }
}

impl Dispatch<zwp_text_input_manager_v3::ZwpTextInputManagerV3, ()> for crate::AppState {
    fn event(
        _state: &mut Self,
        _proxy: &zwp_text_input_manager_v3::ZwpTextInputManagerV3,
        _event: zwp_text_input_manager_v3::Event,
        _data: &(),
        _conn: &Connection,
        _qh: &QueueHandle<Self>,
    ) {
    }

    event_created_child!(crate::AppState, zwp_text_input_manager_v3::ZwpTextInputManagerV3, [
        zwp_text_input_manager_v3::REQ_GET_TEXT_INPUT_OPCODE => (zwp_text_input_v3::ZwpTextInputV3, ()),
    ]);
}

impl Dispatch<zwp_text_input_v3::ZwpTextInputV3, ()> for crate::AppState {
    fn event(
        state: &mut Self,
        proxy: &zwp_text_input_v3::ZwpTextInputV3,
        event: zwp_text_input_v3::Event,
        _data: &(),
        _conn: &Connection,
        _qh: &QueueHandle<Self>,
    ) {
        match event {
            zwp_text_input_v3::Event::Enter { .. } => state.text_input.on_enter(proxy),
            zwp_text_input_v3::Event::Leave { .. } => state.text_input.on_leave(proxy),
            zwp_text_input_v3::Event::CommitString { text: Some(text) } => {
                crate::log_line(format!("text-input {text}"));
            }
            // done/surrounding_text/content_type/preedit_string: state an input
            // method pushes to the client, which this client does not edit.
            _ => {}
        }
    }
}