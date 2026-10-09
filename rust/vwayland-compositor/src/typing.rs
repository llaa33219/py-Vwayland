//! Unicode text typing (`type_text`).
//!
//! Two layers, in priority order:
//!
//! - **layer C (IME)**: the focused app has an enabled `zwp_text_input_v3`, so
//!   the text is committed with `commit_string` + `done`, exactly what a real
//!   input method does. See [`crate::text_input`].
//! - **layer B (keymap typing)**: the universal fallback. The text is typed as
//!   *real key events*: the xkb keymap is temporarily replaced by a synthetic
//!   one whose scratch keycodes carry the wanted keysyms, the keycodes are
//!   injected, then the original keymap is restored. This is the `wtype`
//!   technique internalized in the compositor; `wl_keyboard.key` only carries
//!   keycodes, so the client resolves them through the keymap we just sent.
//!
//! Unlike a clipboard paste, layer B works in paste-blocked fields and is
//! indistinguishable from human typing.

use std::collections::HashMap;
use std::time::Duration;

use smithay::input::keyboard::xkb::Keysym;
use smithay::input::keyboard::XkbConfig;
use tracing::warn;

use crate::state::Vwayland;

/// Characters typed per keymap swap (one scratch key per character).
const CHUNK: usize = 32;

/// First xkb keycode used for the synthetic keys (xkb keycodes start at 8).
const FIRST_KEYCODE: u32 = 9;

/// Upper bound for the total time a single `type_text` call may spend sleeping
/// between characters. The wait is synchronous (the caller blocks on the event
/// loop), so it is capped instead of being unbounded.
const MAX_TOTAL_WAIT: Duration = Duration::from_secs(10);

/// Typing method reported back to the IPC client.
pub const METHOD_IME: &str = "ime";
pub const METHOD_KEYS: &str = "keys";

/// Type `text` into the focused app, choosing the highest-priority layer that
/// the app supports. Returns the method used ("ime" or "keys").
pub fn type_text(state: &mut Vwayland, text: &str, interval_ms: u64) -> Result<&'static str, String> {
    // Layer C: a single atomic commit_string, so interval_ms does not apply.
    if state.commit_text_input(text) {
        return Ok(METHOD_IME);
    }
    if text.is_empty() {
        return Ok(METHOD_KEYS);
    }
    type_with_keys(state, text, interval_ms)?;
    Ok(METHOD_KEYS)
}

/// Layer B: type the text as key events through a temporary keymap.
fn type_with_keys(state: &mut Vwayland, text: &str, interval_ms: u64) -> Result<(), String> {
    let Some(keyboard) = state.seat.get_keyboard() else {
        return Err("no keyboard in this compositor".to_string());
    };
    if keyboard.current_focus().is_none() {
        return Err("no focused surface".to_string());
    }

    let characters: Vec<char> = text.chars().collect();
    let interval = per_char_interval(characters.len(), interval_ms);
    for chunk in characters.chunks(CHUNK) {
        let (keymap, codes) = scratch_keymap(chunk);
        keyboard
            .set_keymap_from_string(state, keymap)
            .map_err(|e| format!("temporary keymap rejected: {e}"))?;
        for code in codes {
            state.inject_xkb_key(code, true);
            state.inject_xkb_key(code, false);
            if !interval.is_zero() {
                std::thread::sleep(interval);
            }
        }
    }
    // Restore the keymap installed at startup (the same XkbConfig::default()
    // that `add_keyboard` got); also runs when the loop above returned early.
    if let Err(e) = keyboard.set_xkb_config(state, XkbConfig::default()) {
        warn!(error = %e, "failed to restore the default keymap after typing");
    }
    Ok(())
}

/// Delay between two characters, reduced so that the whole call stays inside
/// [`MAX_TOTAL_WAIT`].
fn per_char_interval(characters: usize, interval_ms: u64) -> Duration {
    if characters == 0 || interval_ms == 0 {
        return Duration::ZERO;
    }
    let gaps = (characters - 1) as u64;
    let total = interval_ms.saturating_mul(gaps);
    let cap_ms = MAX_TOTAL_WAIT.as_millis() as u64;
    Duration::from_millis(if total > cap_ms {
        cap_ms / gaps.max(1)
    } else {
        interval_ms
    })
}

/// Build a keymap that maps one scratch keycode per character of `chunk`, plus
/// the keycodes in the same order as `chunk`.
///
/// The skeleton is the one used by `wtype`: scratch keycodes and the standard
/// ("complete") types/compatibility includes, so no modifier state is involved
/// and every key produces its keysym at level 1.
fn scratch_keymap(chunk: &[char]) -> (String, Vec<u32>) {
    let mut codes = Vec::with_capacity(chunk.len());
    let mut symbols = String::new();
    let mut keycodes_section = String::new();
    let mut assigned: HashMap<char, u32> = HashMap::new();
    for character in chunk {
        let symbol = keysym_symbol(keysym_of(*character));
        let code = match assigned.get(character) {
            // A repeated character reuses the key it already got.
            Some(code) => *code,
            None => {
                let code = FIRST_KEYCODE + assigned.len() as u32;
                assigned.insert(*character, code);
                let key = assigned.len(); // 1-based name index, like wtype
                keycodes_section.push_str(&format!("    <K{key}> = {code};\n"));
                symbols.push_str(&format!("    key <K{key}> {{ [{symbol}] }};\n"));
                code
            }
        };
        codes.push(code);
    }

    let mut keymap = String::from("xkb_keymap {\n  xkb_keycodes \"vwayland_type\" {\n    minimum = 8;\n");
    keymap.push_str(&format!(
        "    maximum = {};\n{}  }};\n",
        FIRST_KEYCODE + assigned.len() as u32,
        keycodes_section
    ));
    keymap.push_str(
        "  xkb_types \"vwayland_type\" { include \"complete\" };\n  xkb_compatibility \"vwayland_type\" { include \"complete\" };\n  xkb_symbols \"vwayland_type\" {\n",
    );
    keymap.push_str(&symbols);
    keymap.push_str("  };\n};\n");

    (keymap, codes)
}

/// Keysym a character is typed with (same mapping as `xkb_utf32_to_keysym`,
/// with the control characters that have a keysym of their own).
fn keysym_of(character: char) -> Keysym {
    match character {
        '\n' | '\r' => Keysym::Return,
        '\t' => Keysym::Tab,
        '\u{8}' => Keysym::BackSpace,
        '\u{1b}' => Keysym::Escape,
        '\u{7f}' => Keysym::Delete,
        _ => Keysym::from_char(character),
    }
}

/// Keymap source form of a keysym: its name, or the `UXXXX` / `0x…` notation
/// the keymap parser accepts for keysyms without a name (Unicode keysyms).
fn keysym_symbol(keysym: Keysym) -> String {
    let raw: u32 = keysym.into();
    if (0x0100_0000..=0x0110_ffff).contains(&raw) {
        return format!("U{:04X}", raw - 0x0100_0000);
    }
    match keysym.name() {
        // xkeysym prefixes the table names with "XK_", the keymap syntax does not.
        Some(name) => name.strip_prefix("XK_").unwrap_or(name).to_string(),
        None => format!("0x{raw:08x}"),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use smithay::input::keyboard::xkb;

    /// The keymap must compile and resolve back to exactly the keysym we asked
    /// for — this is what makes layer B deliver the right character.
    #[test]
    fn scratch_keymap_compiles_with_the_expected_keysyms() {
        let text = ['a', 'A', ' ', '\n', '안', 'Ω', 'ß'];
        let (keymap, codes) = scratch_keymap(&text);
        assert_eq!(codes.len(), text.len());

        let context = xkb::Context::new(xkb::CONTEXT_NO_FLAGS);
        let compiled = xkb::Keymap::new_from_string(
            &context,
            keymap.clone(),
            xkb::KEYMAP_FORMAT_TEXT_V1,
            xkb::KEYMAP_COMPILE_NO_FLAGS,
        )
        .expect("generated keymap must be valid XKB");

        for (character, code) in text.iter().zip(codes) {
            let syms = compiled.key_get_syms_by_level(code.into(), 0, 0);
            assert_eq!(
                syms.first().copied(),
                Some(keysym_of(*character)),
                "wrong keysym for {character:?}"
            );
        }
    }

    #[test]
    fn repeated_characters_share_one_keycode() {
        let (keymap, codes) = scratch_keymap(&['x', 'y', 'x']);
        assert_eq!(codes[0], codes[2]);
        assert_ne!(codes[0], codes[1]);
        assert_eq!(keymap.matches("xkb_symbols").count(), 1);
    }

    #[test]
    fn per_char_interval_is_capped() {
        assert_eq!(per_char_interval(5, 0), Duration::ZERO);
        assert_eq!(per_char_interval(0, 10), Duration::ZERO);
        assert_eq!(per_char_interval(5, 10), Duration::from_millis(10));
        assert!(per_char_interval(100_000, 1_000) <= MAX_TOTAL_WAIT);
    }
}
