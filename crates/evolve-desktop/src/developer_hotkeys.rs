//! Preset, conflict-checked Win32 global hotkeys for the optional external overlay.
//! Only active while Evolve is foreground and the Developer offline tools are armed.
use std::fmt;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub(super) enum Action {
    Aim,
    Save,
    Return,
    XMinus,
    XPlus,
    YMinus,
    YPlus,
    ZMinus,
    ZPlus,
}
impl Action {
    pub const ALL: [Self; 9] = [
        Self::Aim,
        Self::Save,
        Self::Return,
        Self::XMinus,
        Self::XPlus,
        Self::YMinus,
        Self::YPlus,
        Self::ZMinus,
        Self::ZPlus,
    ];
    pub fn index(self) -> usize {
        match self {
            Self::Aim => 0,
            Self::Save => 1,
            Self::Return => 2,
            Self::XMinus => 3,
            Self::XPlus => 4,
            Self::YMinus => 5,
            Self::YPlus => 6,
            Self::ZMinus => 7,
            Self::ZPlus => 8,
        }
    }
    pub fn from_index(i: usize) -> Option<Self> {
        Self::ALL.get(i).copied()
    }
    pub fn label(self) -> &'static str {
        match self {
            Self::Aim => "Teleport to Crosshair",
            Self::Save => "Save position",
            Self::Return => "Return to saved",
            Self::XMinus => "X −20",
            Self::XPlus => "X +20",
            Self::YMinus => "Y −20",
            Self::YPlus => "Y +20",
            Self::ZMinus => "Z −20",
            Self::ZPlus => "Z +20",
        }
    }
}

#[derive(Debug, Default, Clone, Copy, Eq, PartialEq, Hash)]
pub(super) struct Keybind(pub u8);

// Explicitly loaded presets. Disabled by default so nothing steals keys until
// the user opts in. F6–F12, then Ctrl+F6 and Ctrl+F7.
pub(super) const SUGGESTED: [u8; 9] = [22, 23, 24, 25, 26, 27, 28, 1, 2];

impl Keybind {
    // None, then Ctrl/Alt/Shift/F6..F12 and unmodified F6..F12.
    // No mouse hooks, arbitrary keys, or debugger-based input capture.
    pub const OPTIONS: [Self; 29] = [
        Self(0),
        Self(1),
        Self(2),
        Self(3),
        Self(4),
        Self(5),
        Self(6),
        Self(7),
        Self(8),
        Self(9),
        Self(10),
        Self(11),
        Self(12),
        Self(13),
        Self(14),
        Self(15),
        Self(16),
        Self(17),
        Self(18),
        Self(19),
        Self(20),
        Self(21),
        Self(22),
        Self(23),
        Self(24),
        Self(25),
        Self(26),
        Self(27),
        Self(28),
    ];
    pub fn win32(self) -> Option<(u32, u32)> {
        if self.0 == 0 || self.0 > 28 {
            return None;
        }
        let i = (self.0 - 1) as u32;
        let mods = match i / 7 {
            0 => 0x0002, // MOD_CONTROL
            1 => 0x0001, // MOD_ALT
            2 => 0x0004, // MOD_SHIFT
            _ => 0,
        };
        Some((mods, 0x75 + i % 7)) // F6=VK_F6 (0x75), ... F12=0x7B
    }
}
impl fmt::Display for Keybind {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        if self.0 == 0 {
            return f.write_str("Disabled");
        }
        let i = (self.0 - 1) as usize;
        let prefix = match i / 7 {
            0 => "Ctrl+",
            1 => "Alt+",
            2 => "Shift+",
            _ => "",
        };
        write!(f, "{prefix}F{}", 6 + i % 7)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn hotkeys_are_valid() {
        assert_eq!(Keybind(0).win32(), None);
        let mut seen = std::collections::HashSet::new();
        for key in SUGGESTED {
            assert!(seen.insert(Keybind(key).win32().unwrap()));
        }
        assert_eq!(SUGGESTED.len(), Action::ALL.len());
        assert_eq!(Keybind(1).win32(), Some((2, 0x75)));
        assert_eq!(Keybind(28).win32(), Some((0, 0x7B)));
        for action in Action::ALL {
            assert_eq!(Action::from_index(action.index()), Some(action));
        }
    }
}
