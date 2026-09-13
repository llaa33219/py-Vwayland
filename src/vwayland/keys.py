"""Key/button name → evdev code mappings.

evdev codes are a stable Linux kernel ABI (input-event-codes.h), so the
protocol exchanges keys in these codes.
"""

from __future__ import annotations

from .errors import VwaylandError

# Mouse buttons (BTN_*)
BUTTONS: dict[str, int] = {
    "left": 0x110,
    "right": 0x111,
    "middle": 0x112,
    "side": 0x113,
    "extra": 0x114,
    "forward": 0x115,
    "back": 0x116,
}

_LETTERS = {
    "a": 30, "b": 48, "c": 46, "d": 32, "e": 18, "f": 33, "g": 34,
    "h": 35, "i": 23, "j": 36, "k": 37, "l": 38, "m": 50, "n": 49,
    "o": 24, "p": 25, "q": 16, "r": 19, "s": 31, "t": 20, "u": 22,
    "v": 47, "w": 17, "x": 45, "y": 21, "z": 44,
}

_DIGITS = {
    "1": 2, "2": 3, "3": 4, "4": 5, "5": 6,
    "6": 7, "7": 8, "8": 9, "9": 10, "0": 11,
}

_NAMED: dict[str, int] = {
    "esc": 1, "escape": 1,
    "minus": 12, "-": 12,
    "equal": 13, "=": 13,
    "backspace": 14,
    "tab": 15,
    "leftbrace": 26, "[": 26,
    "rightbrace": 27, "]": 27,
    "enter": 28, "return": 28,
    "ctrl": 29, "control": 29, "leftctrl": 29,
    "semicolon": 39, ";": 39,
    "apostrophe": 40, "'": 40,
    "grave": 41, "`": 41,
    "shift": 42, "leftshift": 42,
    "backslash": 43, "\\": 43,
    "comma": 51, ",": 51,
    "dot": 52, ".": 52, "period": 52,
    "slash": 53, "/": 53,
    "rightshift": 54,
    "kpasterisk": 55,
    "alt": 56, "leftalt": 56,
    "space": 57, " ": 57,
    "capslock": 58,
    "f1": 59, "f2": 60, "f3": 61, "f4": 62, "f5": 63, "f6": 64,
    "f7": 65, "f8": 66, "f9": 67, "f10": 68, "f11": 87, "f12": 88,
    "numlock": 69, "scrolllock": 70,
    "kp7": 71, "kp8": 72, "kp9": 73, "kpminus": 74,
    "kp4": 75, "kp5": 76, "kp6": 77, "kpplus": 78,
    "kp1": 79, "kp2": 80, "kp3": 81, "kp0": 82, "kpdot": 83,
    "kpenter": 96, "rightctrl": 97, "kpslash": 98,
    "printscreen": 99, "sysrq": 99,
    "rightalt": 100,
    "home": 102, "up": 103, "pageup": 104,
    "left": 105, "right": 106,
    "end": 107, "down": 108, "pagedown": 109,
    "insert": 110, "delete": 111,
    "pause": 119,
    "super": 125, "meta": 125, "leftmeta": 125, "win": 125,
    "rightmeta": 126,
    "menu": 139,
}

KEYS: dict[str, int] = {**_LETTERS, **_DIGITS, **_NAMED}

# US-layout shift mapping for type_text
SHIFT_CHARS: dict[str, str] = {
    "~": "`", "!": "1", "@": "2", "#": "3", "$": "4", "%": "5",
    "^": "6", "&": "7", "*": "8", "(": "9", ")": "0", "_": "-",
    "+": "=", "{": "[", "}": "]", "|": "\\", ":": ";", '"': "'",
    "<": ",", ">": ".", "?": "/",
}


def resolve_key(key: "str | int") -> int:
    """Convert a key name ("enter", "a", "ctrl", "f5", ...) or evdev code (int) to a code."""
    if isinstance(key, int):
        if not 0 <= key <= 767:
            raise VwaylandError(f"invalid evdev key code: {key}")
        return key
    name = key.lower()
    if len(name) > 1:
        name = name.strip()
    if name.startswith("key_"):
        name = name[4:]
    if name in KEYS:
        return KEYS[name]
    raise VwaylandError(f"unknown key name: {key!r}")


def resolve_button(button: "str | int") -> int:
    """Convert a button name ("left"/"right"/"middle"/...) or evdev button code to a code."""
    if isinstance(button, int):
        if not 0x110 <= button <= 0x117:
            raise VwaylandError(f"invalid evdev button code: {button:#x}")
        return button
    name = button.lower()
    if len(name) > 1:
        name = name.strip()
    if name.startswith("btn_"):
        name = name[4:]
    if name in BUTTONS:
        return BUTTONS[name]
    raise VwaylandError(f"unknown button name: {button!r}")
