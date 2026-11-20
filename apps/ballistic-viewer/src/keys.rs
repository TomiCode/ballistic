//! Keys by name, for the settings (`ArrowUp`, …) and the controls screen
//! (`Up`, …), and the original's `DirectInput` scan codes for its defaults.

use winit::keyboard::KeyCode;

/// The keys that can be bound: the window's code, the settings' name, what
/// the controls screen shows, and the `DirectInput` scan code (0: none).
const KEYS: &[(KeyCode, &str, &str, u32)] = &[
    (KeyCode::ArrowUp, "ArrowUp", "Up", 200),
    (KeyCode::ArrowDown, "ArrowDown", "Down", 208),
    (KeyCode::ArrowLeft, "ArrowLeft", "Left", 203),
    (KeyCode::ArrowRight, "ArrowRight", "Right", 205),
    (KeyCode::ShiftLeft, "ShiftLeft", "Shift", 42),
    (KeyCode::ShiftRight, "ShiftRight", "R-Shift", 54),
    (KeyCode::ControlLeft, "ControlLeft", "Ctrl", 29),
    (KeyCode::ControlRight, "ControlRight", "R-Ctrl", 157),
    (KeyCode::AltLeft, "AltLeft", "Alt", 56),
    (KeyCode::AltRight, "AltRight", "Alt Gr", 184),
    (KeyCode::Space, "Space", "Space", 57),
    (KeyCode::Enter, "Enter", "Enter", 28),
    (KeyCode::Tab, "Tab", "Tab", 15),
    (KeyCode::Backspace, "Backspace", "Backspace", 14),
    (KeyCode::Insert, "Insert", "Insert", 210),
    (KeyCode::Delete, "Delete", "Delete", 211),
    (KeyCode::Home, "Home", "Home", 199),
    (KeyCode::End, "End", "End", 207),
    (KeyCode::PageUp, "PageUp", "Page Up", 201),
    (KeyCode::PageDown, "PageDown", "Page Down", 209),
    (KeyCode::KeyA, "KeyA", "A", 30),
    (KeyCode::KeyB, "KeyB", "B", 48),
    (KeyCode::KeyC, "KeyC", "C", 46),
    (KeyCode::KeyD, "KeyD", "D", 32),
    (KeyCode::KeyE, "KeyE", "E", 18),
    (KeyCode::KeyF, "KeyF", "F", 33),
    (KeyCode::KeyG, "KeyG", "G", 34),
    (KeyCode::KeyH, "KeyH", "H", 35),
    (KeyCode::KeyI, "KeyI", "I", 23),
    (KeyCode::KeyJ, "KeyJ", "J", 36),
    (KeyCode::KeyK, "KeyK", "K", 37),
    (KeyCode::KeyL, "KeyL", "L", 38),
    (KeyCode::KeyM, "KeyM", "M", 50),
    (KeyCode::KeyN, "KeyN", "N", 49),
    (KeyCode::KeyO, "KeyO", "O", 24),
    (KeyCode::KeyP, "KeyP", "P", 25),
    (KeyCode::KeyQ, "KeyQ", "Q", 16),
    (KeyCode::KeyR, "KeyR", "R", 19),
    (KeyCode::KeyS, "KeyS", "S", 31),
    (KeyCode::KeyT, "KeyT", "T", 20),
    (KeyCode::KeyU, "KeyU", "U", 22),
    (KeyCode::KeyV, "KeyV", "V", 47),
    (KeyCode::KeyW, "KeyW", "W", 17),
    (KeyCode::KeyX, "KeyX", "X", 45),
    (KeyCode::KeyY, "KeyY", "Y", 21),
    (KeyCode::KeyZ, "KeyZ", "Z", 44),
    (KeyCode::Digit0, "Digit0", "0", 11),
    (KeyCode::Digit1, "Digit1", "1", 2),
    (KeyCode::Digit2, "Digit2", "2", 3),
    (KeyCode::Digit3, "Digit3", "3", 4),
    (KeyCode::Digit4, "Digit4", "4", 5),
    (KeyCode::Digit5, "Digit5", "5", 6),
    (KeyCode::Digit6, "Digit6", "6", 7),
    (KeyCode::Digit7, "Digit7", "7", 8),
    (KeyCode::Digit8, "Digit8", "8", 9),
    (KeyCode::Digit9, "Digit9", "9", 10),
    (KeyCode::Numpad0, "Numpad0", "Num 0", 82),
    (KeyCode::Numpad1, "Numpad1", "Num 1", 79),
    (KeyCode::Numpad2, "Numpad2", "Num 2", 80),
    (KeyCode::Numpad3, "Numpad3", "Num 3", 81),
    (KeyCode::Numpad4, "Numpad4", "Num 4", 75),
    (KeyCode::Numpad5, "Numpad5", "Num 5", 76),
    (KeyCode::Numpad6, "Numpad6", "Num 6", 77),
    (KeyCode::Numpad7, "Numpad7", "Num 7", 71),
    (KeyCode::Numpad8, "Numpad8", "Num 8", 72),
    (KeyCode::Numpad9, "Numpad9", "Num 9", 73),
    (KeyCode::NumpadEnter, "NumpadEnter", "Num Enter", 156),
    (KeyCode::Comma, "Comma", ",", 51),
    (KeyCode::Period, "Period", ".", 52),
    (KeyCode::Slash, "Slash", "/", 53),
    (KeyCode::Semicolon, "Semicolon", ";", 39),
    (KeyCode::Quote, "Quote", "'", 40),
    (KeyCode::Minus, "Minus", "-", 12),
    (KeyCode::Equal, "Equal", "=", 13),
    (KeyCode::BracketLeft, "BracketLeft", "[", 26),
    (KeyCode::BracketRight, "BracketRight", "]", 27),
    (KeyCode::Backslash, "Backslash", "\\", 43),
];

/// A key's settings name, if it can be bound.
pub fn name(code: KeyCode) -> Option<&'static str> {
    KEYS.iter().find(|k| k.0 == code).map(|k| k.1)
}

pub fn from_name(name: &str) -> Option<KeyCode> {
    KEYS.iter().find(|k| k.1 == name).map(|k| k.0)
}

/// The settings name of a `DirectInput` scan code.
pub fn name_of_scan(scan: u32) -> Option<&'static str> {
    KEYS.iter().find(|k| k.3 == scan).map(|k| k.1)
}

/// What the controls screen shows for a settings name.
pub fn display(name: &str) -> &str {
    KEYS.iter().find(|k| k.1 == name).map_or(name, |k| k.2)
}

/// The six bindings (forward, backward, left, right, rotate, lift): the
/// settings' keys, or the game's defaults (`default_scans`) where they have
/// none.
pub fn bindings(settings: &[String], default_scans: &[u32]) -> [KeyCode; 6] {
    let fallback = [
        KeyCode::ArrowUp,
        KeyCode::ArrowDown,
        KeyCode::ArrowLeft,
        KeyCode::ArrowRight,
        KeyCode::ShiftLeft,
        KeyCode::Space,
    ];
    std::array::from_fn(|i| {
        settings
            .get(i)
            .and_then(|n| from_name(n))
            .or_else(|| {
                default_scans
                    .get(i)
                    .and_then(|&s| from_name(name_of_scan(s)?))
            })
            .unwrap_or(fallback[i])
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn names_round_trip_and_defaults_fill_in() {
        assert_eq!(from_name(name(KeyCode::KeyW).unwrap()), Some(KeyCode::KeyW));
        assert_eq!(display("ShiftLeft"), "Shift");
        let b = bindings(&["KeyW".into()], &[200, 208, 203, 205, 42, 57]);
        assert_eq!(b[0], KeyCode::KeyW);
        assert_eq!(b[4], KeyCode::ShiftLeft);
        assert_eq!(b[5], KeyCode::Space);
    }
}
