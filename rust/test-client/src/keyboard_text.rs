//! Key -> character translation for the test client (layer B observability).
//!
//! `wl_keyboard.key` only carries keycodes, so the character a press stands for
//! depends on the keymap that is current *at event time*: the compositor's
//! typing engine installs a temporary keymap whose scratch keycodes carry
//! Unicode keysyms, types, then restores the original one. The xkb state is
//! therefore rebuilt on every `wl_keyboard.keymap` event instead of being
//! parsed once at startup.

use std::collections::HashSet;
use std::os::fd::OwnedFd;

use wayland_client::protocol::wl_keyboard;
use wayland_client::{Connection, Dispatch, QueueHandle, WEnum};
use xkbcommon::xkb::{
    Context, Keycode, KeyDirection, Keymap, ModMask, State, CONTEXT_NO_FLAGS,
    KEYMAP_COMPILE_NO_FLAGS, KEYMAP_FORMAT_TEXT_V1, LAYOUT_INVALID,
};

/// xkb keycodes are evdev codes plus this offset.
const XKB_KEYCODE_OFFSET: u32 = 8;

pub struct KeyboardText {
    context: Context,
    /// Translation state of the current keymap; absent until the first keymap.
    state: Option<State>,
    /// Keycodes currently held down, so an auto-repeat is not typed twice.
    depressed: HashSet<u32>,
}

impl KeyboardText {
    pub fn new() -> Self {
        Self {
            context: Context::new(CONTEXT_NO_FLAGS),
            state: None,
            depressed: HashSet::new(),
        }
    }

    /// Compiles the keymap the compositor just sent and makes it the current
    /// one. On failure the previous state is kept, so a broken keymap degrades
    /// to no output instead of killing the client. The descriptor is owned by
    /// the event and closed here in every case.
    pub fn on_keymap(
        &mut self,
        format: WEnum<wl_keyboard::KeymapFormat>,
        fd: OwnedFd,
        size: u32,
    ) {
        if !matches!(format, WEnum::Value(wl_keyboard::KeymapFormat::XkbV1)) || size == 0 {
            return;
        }
        // SAFETY: the descriptor is the keymap file of `size` bytes just
        // received from the compositor; it is mapped read-only, never written.
        let compiled = unsafe {
            Keymap::new_from_fd(
                &self.context,
                fd,
                size as usize,
                KEYMAP_FORMAT_TEXT_V1,
                KEYMAP_COMPILE_NO_FLAGS,
            )
        };
        match compiled {
            Ok(Some(keymap)) => {
                self.state = Some(State::new(&keymap));
                self.depressed.clear();
            }
            Ok(None) | Err(_) => crate::log_line("typed-keymap-error".to_string()),
        }
    }

    /// Feeds the modifier/layout state, needed so shifted keys translate right.
    pub fn on_modifiers(&mut self, depressed: u32, latched: u32, locked: u32, group: u32) {
        if let Some(state) = self.state.as_mut() {
            state.update_mask(
                depressed as ModMask,
                latched as ModMask,
                locked as ModMask,
                group,
                LAYOUT_INVALID,
                LAYOUT_INVALID,
            );
        }
    }

    /// Runs the key through the xkb state machine and returns the character it
    /// typed, if any. Releases only advance the state; non-printable presses
    /// (modifiers, arrows, function keys) produce nothing.
    pub fn on_key(&mut self, key: u32, pressed: bool) -> Option<char> {
        // The compositor reports evdev codes in `wl_keyboard.key` while its
        // keymap uses the xkb convention, so the code has to be shifted before
        // it can be resolved.
        let code = Keycode::new(key + XKB_KEYCODE_OFFSET);
        let character = self.state.as_mut().and_then(|state| {
            state.update_key(
                code,
                if pressed { KeyDirection::Down } else { KeyDirection::Up },
            );
            char::from_u32(state.key_get_utf32(code)).filter(|c| is_printable(*c))
        });
        if pressed {
            if !self.depressed.insert(key) {
                return None;
            }
            character
        } else {
            self.depressed.remove(&key);
            None
        }
    }
}

/// Whitespace counts as typed text; other control characters (Escape, BackSpace,
/// modifiers) have no textual meaning and are dropped.
fn is_printable(character: char) -> bool {
    character == '\n' || character == '\t' || !character.is_control()
}

impl Dispatch<wl_keyboard::WlKeyboard, ()> for crate::AppState {
    fn event(
        state: &mut Self,
        _proxy: &wl_keyboard::WlKeyboard,
        event: wl_keyboard::Event,
        _data: &(),
        _conn: &Connection,
        _qh: &QueueHandle<Self>,
    ) {
        match event {
            wl_keyboard::Event::Enter { .. } => crate::log_line("keyboard_enter".to_string()),
            wl_keyboard::Event::Key {
                key,
                state: pressed,
                ..
            } => {
                crate::log_line(format!("key {key} {pressed:?}"));
                let is_pressed = matches!(
                    pressed,
                    wayland_client::WEnum::Value(wl_keyboard::KeyState::Pressed)
                );
                if let Some(character) = state.keyboard_text.on_key(key, is_pressed) {
                    crate::log_line(format!("typed {character}"));
                }
            }
            wl_keyboard::Event::Modifiers {
                mods_depressed,
                mods_latched,
                mods_locked,
                group,
                ..
            } => {
                crate::log_line(format!("modifiers {mods_depressed}"));
                state
                    .keyboard_text
                    .on_modifiers(mods_depressed, mods_latched, mods_locked, group);
            }
            wl_keyboard::Event::Keymap { format, fd, size } => {
                state.keyboard_text.on_keymap(format, fd, size);
            }
            wl_keyboard::Event::Leave { .. } => crate::log_line("keyboard_leave".to_string()),
            _ => {}
        }
    }
}