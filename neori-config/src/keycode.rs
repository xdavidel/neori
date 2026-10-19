//! Mapping from keysym names to evdev keycodes.
//!
//! Keysyms depend on the active keyboard layout, so `Mod+q` stops matching when the
//! layout is switched to Hebrew. Keycodes do not change with the layout, so binding by
//! keycode keeps a shortcut working regardless of which layout is active.
//!
//! The table is generated from the `us`/`pc105` XKB layout: for every key it records
//! the unshifted keysym together with the Linux evdev code (`<linux/input-event-codes.h>`).
//! Note that evdev codes are 8 lower than the XKB keycodes used on the wire, so runtime
//! keycodes have to be converted before being compared against this table.

/// Keysym name (lowercase, as used in keybinding strings) to Linux evdev keycode.
const KEYSYM_TO_EVDEV: &[(&str, u32)] = &[
    ("0", 11),
    ("1", 2),
    ("2", 3),
    ("3", 4),
    ("4", 5),
    ("5", 6),
    ("6", 7),
    ("7", 8),
    ("8", 9),
    ("9", 10),
    ("a", 30),
    ("alt_l", 56),
    ("alt_r", 100),
    ("apostrophe", 40),
    ("b", 48),
    ("backslash", 43),
    ("backspace", 14),
    ("bracketleft", 26),
    ("bracketright", 27),
    ("c", 46),
    ("cancel", 128),
    ("caps_lock", 58),
    ("comma", 51),
    ("control_l", 29),
    ("control_r", 97),
    ("d", 32),
    ("delete", 111),
    ("down", 108),
    ("e", 18),
    ("end", 107),
    ("equal", 13),
    ("escape", 1),
    ("f", 33),
    ("f1", 59),
    ("f10", 68),
    ("f11", 87),
    ("f12", 88),
    ("f19", 189),
    ("f2", 60),
    ("f23", 193),
    ("f24", 85),
    ("f3", 61),
    ("f4", 62),
    ("f5", 63),
    ("f6", 64),
    ("f7", 65),
    ("f8", 66),
    ("f9", 67),
    ("find", 136),
    ("g", 34),
    ("grave", 41),
    ("h", 35),
    ("hangul", 122),
    ("hangul_hanja", 123),
    ("help", 138),
    ("henkan_mode", 92),
    ("hiragana", 91),
    ("hiragana_katakana", 93),
    ("home", 102),
    ("i", 23),
    ("insert", 110),
    ("iso_level3_shift", 84),
    ("iso_level5_shift", 195),
    ("j", 36),
    ("k", 37),
    ("katakana", 90),
    ("kp_add", 78),
    ("kp_begin", 76),
    ("kp_decimal", 121),
    ("kp_delete", 83),
    ("kp_divide", 98),
    ("kp_down", 80),
    ("kp_end", 79),
    ("kp_enter", 96),
    ("kp_equal", 117),
    ("kp_home", 71),
    ("kp_insert", 82),
    ("kp_left", 75),
    ("kp_multiply", 55),
    ("kp_next", 81),
    ("kp_prior", 73),
    ("kp_right", 77),
    ("kp_subtract", 74),
    ("kp_up", 72),
    ("l", 38),
    ("left", 105),
    ("less", 86),
    ("linefeed", 101),
    ("m", 50),
    ("menu", 127),
    ("minus", 12),
    ("muhenkan", 94),
    ("n", 49),
    ("next", 109),
    ("num_lock", 69),
    ("o", 24),
    ("p", 25),
    ("parenleft", 179),
    ("parenright", 180),
    ("pause", 119),
    ("period", 52),
    ("plusminus", 118),
    ("print", 99),
    ("prior", 104),
    ("q", 16),
    ("r", 19),
    ("redo", 129),
    ("return", 28),
    ("right", 106),
    ("s", 31),
    ("scroll_lock", 70),
    ("semicolon", 39),
    ("shift_l", 42),
    ("shift_r", 54),
    ("slash", 53),
    ("space", 57),
    ("sunfront", 132),
    ("sunprops", 130),
    ("super_l", 125),
    ("super_r", 126),
    ("t", 20),
    ("tab", 15),
    ("u", 22),
    ("undo", 131),
    ("up", 103),
    ("v", 47),
    ("w", 17),
    ("x", 45),
    ("xf86audiobassboost", 209),
    ("xf86audioforward", 208),
    ("xf86audiolowervolume", 114),
    ("xf86audiomedia", 226),
    ("xf86audiomicmute", 190),
    ("xf86audiomute", 113),
    ("xf86audionext", 163),
    ("xf86audiopause", 201),
    ("xf86audioplay", 164),
    ("xf86audiopreset", 213),
    ("xf86audioprev", 165),
    ("xf86audioraisevolume", 115),
    ("xf86audiorecord", 167),
    ("xf86audiorewind", 168),
    ("xf86audiostop", 166),
    ("xf86back", 158),
    ("xf86battery", 236),
    ("xf86bluetooth", 237),
    ("xf86brightnessauto", 244),
    ("xf86calculator", 140),
    ("xf86close", 174),
    ("xf86copy", 133),
    ("xf86cut", 137),
    ("xf86display", 227),
    ("xf86displayoff", 245),
    ("xf86documents", 235),
    ("xf86dos", 151),
    ("xf86eject", 161),
    ("xf86explorer", 144),
    ("xf86favorites", 156),
    ("xf86finance", 219),
    ("xf86forward", 159),
    ("xf86game", 220),
    ("xf86go", 218),
    ("xf86homepage", 172),
    ("xf86kbdbrightnessdown", 229),
    ("xf86kbdbrightnessup", 230),
    ("xf86kbdlightonoff", 228),
    ("xf86launch1", 148),
    ("xf86launch2", 149),
    ("xf86launch3", 202),
    ("xf86launch4", 203),
    ("xf86launch5", 184),
    ("xf86launch6", 185),
    ("xf86launch7", 186),
    ("xf86launch8", 187),
    ("xf86launch9", 188),
    ("xf86launcha", 120),
    ("xf86launchb", 204),
    ("xf86mail", 155),
    ("xf86mailforward", 233),
    ("xf86menukb", 139),
    ("xf86messenger", 216),
    ("xf86monbrightnesscycle", 243),
    ("xf86monbrightnessdown", 224),
    ("xf86monbrightnessup", 225),
    ("xf86mycomputer", 157),
    ("xf86new", 181),
    ("xf86next_vmode", 241),
    ("xf86open", 134),
    ("xf86paste", 135),
    ("xf86phone", 169),
    ("xf86poweroff", 116),
    ("xf86prev_vmode", 242),
    ("xf86reload", 173),
    ("xf86reply", 232),
    ("xf86rfkill", 247),
    ("xf86rotatewindows", 153),
    ("xf86save", 234),
    ("xf86screensaver", 152),
    ("xf86scrolldown", 178),
    ("xf86scrollup", 177),
    ("xf86search", 217),
    ("xf86send", 145),
    ("xf86shop", 221),
    ("xf86sleep", 142),
    ("xf86suspend", 205),
    ("xf86taskpane", 154),
    ("xf86tools", 171),
    ("xf86touchpadon", 192),
    ("xf86touchpadtoggle", 191),
    ("xf86uwb", 239),
    ("xf86wakeup", 143),
    ("xf86webcam", 212),
    ("xf86wlan", 238),
    ("xf86wwan", 246),
    ("xf86www", 150),
    ("xf86xfer", 147),
    ("y", 21),
    ("z", 44),
];

/// Returns the evdev keycode for `name`, a keysym name as used in keybinding strings.
pub fn evdev_from_keysym_name(name: &str) -> Option<u32> {
    let name = name.to_ascii_lowercase();
    KEYSYM_TO_EVDEV
        .binary_search_by(|(key, _)| key.cmp(&name.as_str()))
        .ok()
        .map(|i| KEYSYM_TO_EVDEV[i].1)
}

/// Returns the keysym name for `code`, a Linux evdev keycode.
pub fn keysym_name_from_evdev(code: u32) -> Option<&'static str> {
    KEYSYM_TO_EVDEV
        .iter()
        .find(|(_, evdev)| *evdev == code)
        .map(|(name, _)| *name)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn evdev_codes_match_the_kernel() {
        assert_eq!(evdev_from_keysym_name("q"), Some(16));
        assert_eq!(evdev_from_keysym_name("Q"), Some(16));
        assert_eq!(evdev_from_keysym_name("slash"), Some(53));
        assert_eq!(evdev_from_keysym_name("Return"), Some(28));
        assert_eq!(evdev_from_keysym_name("F1"), Some(59));
        assert_eq!(evdev_from_keysym_name("space"), Some(57));
        assert_eq!(evdev_from_keysym_name("Super_L"), Some(125));
        assert_eq!(evdev_from_keysym_name("definitely-not-a-key"), None);
    }

    #[test]
    fn names_round_trip() {
        for (name, code) in KEYSYM_TO_EVDEV {
            assert_eq!(evdev_from_keysym_name(name), Some(*code));
            assert_eq!(keysym_name_from_evdev(*code), Some(*name));
        }
    }

    #[test]
    fn table_is_sorted_for_binary_search() {
        for pair in KEYSYM_TO_EVDEV.windows(2) {
            assert!(
                pair[0].0 < pair[1].0,
                "unsorted: {} then {}",
                pair[0].0,
                pair[1].0
            );
        }
    }
}
