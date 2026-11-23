use std::str::FromStr;
use std::time::Duration;

use bitflags::bitflags;
use miette::miette;
use smithay::input::keyboard::keysyms::KEY_NoSymbol;
use smithay::input::keyboard::xkb::{
    keysym_from_name, keysym_get_name, KEYSYM_CASE_INSENSITIVE, KEYSYM_NO_FLAGS,
};
use smithay::input::keyboard::Keysym;
use tracing::warn;

use crate::input::ModKey;

#[derive(Debug, Default, PartialEq)]
pub struct Binds(pub Vec<Bind>);

/// A keybinding mode: while it's active, its binds replace the normal binds.
#[derive(Debug, PartialEq)]
pub struct BindMode {
    pub name: String,
    pub binds: Binds,
}

#[derive(Debug, Clone, PartialEq)]
pub struct Bind {
    pub key: Key,
    /// Actions to dispatch when the bind is pressed and/or released.
    ///
    /// Binds to a Lua function (`neori.bind(key, function)`) use [`Action::ScriptFunction`],
    /// with the function looked up in [`crate::ScriptHost`].
    pub action: BoundAction,
    pub repeat: bool,
    pub pass_through: Option<bool>,
    pub cooldown: Option<Duration>,
    pub allow_when_locked: bool,
    pub allow_inhibiting: bool,
    pub hotkey_overlay_title: Option<Option<String>>,
}

#[derive(Debug, Clone, PartialEq)]
pub enum BoundAction {
    Press(Action),
    Release(Action),
    Both { press: Action, release: Action },
}

impl Bind {
    pub fn has_press(&self) -> bool {
        matches!(
            self.action,
            BoundAction::Press(_) | BoundAction::Both { .. }
        )
    }

    pub fn has_release(&self) -> bool {
        matches!(
            self.action,
            BoundAction::Release(_) | BoundAction::Both { .. }
        )
    }

    pub fn press_action(&self) -> Option<&Action> {
        match &self.action {
            BoundAction::Press(action) => Some(action),
            BoundAction::Both { press, .. } => Some(press),
            BoundAction::Release(_) => None,
        }
    }

    pub fn release_action(&self) -> Option<&Action> {
        match &self.action {
            BoundAction::Release(action) => Some(action),
            BoundAction::Both { release, .. } => Some(release),
            BoundAction::Press(_) => None,
        }
    }

    pub fn into_action(self, pressed: bool) -> Option<Action> {
        match (self.action, pressed) {
            (BoundAction::Press(action), true) | (BoundAction::Release(action), false) => {
                Some(action)
            }
            (BoundAction::Both { press, .. }, true) => Some(press),
            (BoundAction::Both { release, .. }, false) => Some(release),
            _ => None,
        }
    }
}

#[derive(Debug, PartialEq, Eq, Clone, Copy, Hash)]
pub struct Key {
    pub trigger: Trigger,
    pub modifiers: Modifiers,
}

impl Key {
    /// Rewrites a keysym trigger into the equivalent evdev keycode so that the binding
    /// keeps matching the same physical key when the keyboard layout changes.
    ///
    /// The keysym name is looked up in the `us` layout, so this works for any key that
    /// exists on a standard US keyboard. Non-keysym triggers are returned unchanged.
    pub fn into_keycode(self) -> Result<Self, miette::Error> {
        let name = match self.trigger {
            Trigger::Keysym(keysym) => keysym_get_name(keysym),
            other => {
                return Ok(Self {
                    trigger: other,
                    modifiers: self.modifiers,
                });
            }
        };

        match self.try_into_keycode() {
            Some(key) => Ok(key),
            None => Err(miette!(
                "cannot use keycode for '{name}': no key with this name in the US layout"
            )),
        }
    }

    /// Like [`Key::into_keycode`], but returns `None` when the keysym has no key in the US
    /// layout (e.g. XF86 media keys) instead of erroring. Non-keysym triggers convert to
    /// themselves.
    ///
    /// Modifier keysyms (like `Alt_L`) also stay keysyms: modifier binds rely on knowing which
    /// modifier the trigger is, and modifier keys don't move between layouts anyway.
    pub fn try_into_keycode(self) -> Option<Self> {
        let trigger = match self.trigger {
            Trigger::Keysym(keysym) if is_keysym_modifier(keysym) => Trigger::Keysym(keysym),
            Trigger::Keysym(keysym) => {
                let name = keysym_get_name(keysym);
                Trigger::Keycode(crate::keycode::evdev_from_keysym_name(&name)?)
            }
            other => other,
        };

        Some(Self {
            trigger,
            modifiers: self.modifiers,
        })
    }
}

#[derive(Debug, PartialEq, Eq, Clone, Copy, Hash)]
pub enum Trigger {
    Keysym(Keysym),
    /// A Linux evdev keycode. Unlike a keysym this does not change with the active
    /// keyboard layout.
    Keycode(u32),
    CompositorMod,
    Modifier(ModKey),
    MouseLeft,
    MouseRight,
    MouseMiddle,
    MouseBack,
    MouseForward,
    WheelScrollDown,
    WheelScrollUp,
    WheelScrollLeft,
    WheelScrollRight,
    TouchpadScrollDown,
    TouchpadScrollUp,
    TouchpadScrollLeft,
    TouchpadScrollRight,
    TabletStylusButton1,
    TabletStylusButton2,
    TabletStylusButton3,
}

/// Returns whether `keysym` is a modifier key like `Shift_L`.
///
/// This function matches what the neori config considers modifiers.
/// So, it does not correspond exactly to `keysym.is_modifier_key()`.
/// For example, unlike XKB, `Caps_Lock` and `ISO_Next_Group` aren't considered modifiers.
pub fn is_keysym_modifier(keysym: Keysym) -> bool {
    ModKey::from_keysym(keysym).is_some()
}

impl Trigger {
    pub fn is_modifier(self) -> bool {
        match self {
            Self::CompositorMod | Self::Modifier(_) => true,
            Self::Keysym(keysym) => is_keysym_modifier(keysym),
            _ => false,
        }
    }

    pub fn is_generic_modifier(self) -> bool {
        matches!(self, Self::CompositorMod | Self::Modifier(_))
    }

    /// The modifier flags that this trigger sets, if it is a modifier key.
    pub fn implied_modifier(self) -> Modifiers {
        match self {
            Self::CompositorMod => Modifiers::COMPOSITOR,
            Self::Modifier(modifier) => modifier.to_modifiers(),
            Self::Keysym(keysym) => ModKey::from_keysym(keysym)
                .map_or(Modifiers::empty(), |modifier| modifier.to_modifiers()),
            _ => Modifiers::empty(),
        }
    }

    pub fn is_keyboard(self) -> bool {
        matches!(
            self,
            Trigger::Keysym(_)
                | Trigger::Keycode(_)
                | Trigger::CompositorMod
                | Trigger::Modifier(_)
        )
    }

    pub fn is_scroll(self) -> bool {
        matches!(
            self,
            Trigger::WheelScrollDown
                | Trigger::WheelScrollUp
                | Trigger::WheelScrollLeft
                | Trigger::WheelScrollRight
                | Trigger::TouchpadScrollDown
                | Trigger::TouchpadScrollUp
                | Trigger::TouchpadScrollLeft
                | Trigger::TouchpadScrollRight
        )
    }
}

impl Key {
    /// Whether the two keys are triggered by the same key press.
    ///
    /// Two different keybinds can be triggered by the same key press. For example, a modifier key
    /// can be spelled as the modifier itself (`Alt`) or as its keysym (`Alt_L`).
    pub fn conflicts_with(&self, other: &Self) -> bool {
        // The trigger and the modifiers match exactly.
        //
        // All non-modifier triggers fall into this category.
        if *self == *other {
            return true;
        }

        // Modifiers include the trigger's implied modifier, so differing modifiers can never be
        // triggered by the same key press.
        if self.modifiers != other.modifiers {
            return false;
        }

        // Generic-modifier-only binds are press-order-independent, so only their modifier set
        // (which already includes the trigger implied modifier) has to match.
        (self.trigger.is_generic_modifier() && other.trigger.is_modifier())
            || (self.trigger.is_modifier() && other.trigger.is_generic_modifier())
    }
}

bitflags! {
    #[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
    pub struct Modifiers : u8 {
        const CTRL = 1;
        const SHIFT = 1 << 1;
        const ALT = 1 << 2;
        const SUPER = 1 << 3;
        const ISO_LEVEL3_SHIFT = 1 << 4;
        const ISO_LEVEL5_SHIFT = 1 << 5;
        const COMPOSITOR = 1 << 6;
    }
}

#[derive(Debug, Default, Clone, PartialEq)]
pub struct SwitchBinds {
    pub lid_open: Option<SwitchAction>,
    pub lid_close: Option<SwitchAction>,
    pub tablet_mode_on: Option<SwitchAction>,
    pub tablet_mode_off: Option<SwitchAction>,
}

#[derive(Debug, Clone, PartialEq)]
pub struct SwitchAction {
    pub spawn: Vec<String>,
}

pub use neori_ipc::{Action, WorkspaceReferenceArg as WorkspaceReference};

impl FromStr for Key {
    type Err = miette::Error;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        if s.contains('\0') {
            return Err(miette!("invalid key with a null byte: {s:?}"));
        }

        let mut written_modifiers: Vec<(Modifiers, &str)> = Vec::new();

        let mut split = s.split('+');
        let key = split.next_back().unwrap();

        for part in split {
            let part = part.trim();
            let modifier = if part.eq_ignore_ascii_case("mod") {
                Modifiers::COMPOSITOR
            } else if part.eq_ignore_ascii_case("ctrl") || part.eq_ignore_ascii_case("control") {
                Modifiers::CTRL
            } else if part.eq_ignore_ascii_case("shift") {
                Modifiers::SHIFT
            } else if part.eq_ignore_ascii_case("alt") {
                Modifiers::ALT
            } else if part.eq_ignore_ascii_case("super") || part.eq_ignore_ascii_case("win") {
                Modifiers::SUPER
            } else if part.eq_ignore_ascii_case("iso_level3_shift")
                || part.eq_ignore_ascii_case("mod5")
            {
                Modifiers::ISO_LEVEL3_SHIFT
            } else if part.eq_ignore_ascii_case("iso_level5_shift")
                || part.eq_ignore_ascii_case("mod3")
            {
                Modifiers::ISO_LEVEL5_SHIFT
            } else {
                return Err(miette!("invalid modifier: {part}"));
            };

            written_modifiers.push((modifier, part));
        }

        let trigger = if key.eq_ignore_ascii_case("MouseLeft") {
            Trigger::MouseLeft
        } else if key.eq_ignore_ascii_case("MouseRight") {
            Trigger::MouseRight
        } else if key.eq_ignore_ascii_case("MouseMiddle") {
            Trigger::MouseMiddle
        } else if key.eq_ignore_ascii_case("MouseBack") {
            Trigger::MouseBack
        } else if key.eq_ignore_ascii_case("MouseForward") {
            Trigger::MouseForward
        } else if key.eq_ignore_ascii_case("WheelScrollDown") {
            Trigger::WheelScrollDown
        } else if key.eq_ignore_ascii_case("WheelScrollUp") {
            Trigger::WheelScrollUp
        } else if key.eq_ignore_ascii_case("WheelScrollLeft") {
            Trigger::WheelScrollLeft
        } else if key.eq_ignore_ascii_case("WheelScrollRight") {
            Trigger::WheelScrollRight
        } else if key.eq_ignore_ascii_case("TouchpadScrollDown") {
            Trigger::TouchpadScrollDown
        } else if key.eq_ignore_ascii_case("TouchpadScrollUp") {
            Trigger::TouchpadScrollUp
        } else if key.eq_ignore_ascii_case("TouchpadScrollLeft") {
            Trigger::TouchpadScrollLeft
        } else if key.eq_ignore_ascii_case("TouchpadScrollRight") {
            Trigger::TouchpadScrollRight
        } else if key.eq_ignore_ascii_case("TabletStylusButton1") {
            Trigger::TabletStylusButton1
        } else if key.eq_ignore_ascii_case("TabletStylusButton2") {
            Trigger::TabletStylusButton2
        } else if key.eq_ignore_ascii_case("TabletStylusButton3") {
            Trigger::TabletStylusButton3
        } else if key.eq_ignore_ascii_case("Mod") {
            Trigger::CompositorMod
        } else if key.eq_ignore_ascii_case("Ctrl") || key.eq_ignore_ascii_case("Control") {
            Trigger::Modifier(ModKey::Ctrl)
        } else if key.eq_ignore_ascii_case("Shift") {
            Trigger::Modifier(ModKey::Shift)
        } else if key.eq_ignore_ascii_case("Alt") {
            Trigger::Modifier(ModKey::Alt)
        } else if key.eq_ignore_ascii_case("Super") || key.eq_ignore_ascii_case("Win") {
            Trigger::Modifier(ModKey::Super)
        } else if key.eq_ignore_ascii_case("ISO_Level3_Shift") || key.eq_ignore_ascii_case("Mod5") {
            Trigger::Modifier(ModKey::IsoLevel3Shift)
        } else if key.eq_ignore_ascii_case("ISO_Level5_Shift") || key.eq_ignore_ascii_case("Mod3") {
            Trigger::Modifier(ModKey::IsoLevel5Shift)
        } else {
            let mut keysym = keysym_from_name(key, KEYSYM_CASE_INSENSITIVE);

            let with_xf86;
            let mut final_key = key;
            if keysym.raw() == KEY_NoSymbol {
                // If the key didn't match, try it with an XF86 prefix.
                // This way, you can write e.g. AudioMute instead of XF86AudioMute.
                //
                // This is actually needed for the xdg-desktop-portal-gnome implementation of the
                // GlobalShortcuts portal, as it will request non-XF86-prefixed names. Mutter then
                // does this same exact prefixing:
                // https://gitlab.gnome.org/GNOME/mutter/-/blob/b48811767189fb5446bd1b831e98271a21866e09/src/core/meta-accel-parse.c#L302-311
                with_xf86 = format!("XF86{key}");
                keysym = keysym_from_name(&with_xf86, KEYSYM_CASE_INSENSITIVE);
                if keysym.raw() != KEY_NoSymbol {
                    final_key = &with_xf86;
                }
            }

            // The keyboard event handling code can receive either
            // XF86ScreenSaver or XF86Screensaver, because there is no
            // case mapping defined between these keysyms. If we just
            // use the case-insensitive version of keysym_from_name it
            // is not possible to bind the uppercase version, because the
            // case-insensitive match prefers the lowercase version when
            // there is a choice.
            //
            // Therefore, when we match this key with the initial
            // case-insensitive match we try a further case-sensitive match
            // (so that either key can be bound). If that fails, we change
            // to the uppercase version because:
            //
            // - A comment in xkb_keysym_from_name (in libxkbcommon) tells us that the uppercase
            //   version is the "best" of the two. [0]
            // - The xkbcommon crate only has a constant for ScreenSaver. [1]
            //
            // [0]: https://github.com/xkbcommon/libxkbcommon/blob/45a118d5325b051343b4b174f60c1434196fa7d4/src/keysym.c#L276
            // [1]: https://docs.rs/xkbcommon/latest/xkbcommon/xkb/keysyms/index.html#:~:text=KEY%5FXF86ScreenSaver
            //
            // See https://github.com/niri-wm/niri/issues/1969
            if keysym == Keysym::XF86_Screensaver {
                keysym = keysym_from_name(final_key, KEYSYM_NO_FLAGS);
                if keysym.raw() == KEY_NoSymbol {
                    keysym = Keysym::XF86_ScreenSaver;
                }
            }
            if keysym.raw() == KEY_NoSymbol {
                return Err(miette!("invalid key: {key}"));
            }
            Trigger::Keysym(keysym)
        };

        let mut modifiers = trigger.implied_modifier();
        for (modifier, spelling) in written_modifiers {
            if modifiers.contains(modifier) {
                warn!("redundant modifier `{spelling}` in keybind `{s}`");
            }

            modifiers |= modifier;
        }

        Ok(Key { trigger, modifiers })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parse_xf86_screensaver() {
        assert_eq!(
            "XF86ScreenSaver".parse::<Key>().unwrap(),
            Key {
                trigger: Trigger::Keysym(Keysym::XF86_ScreenSaver),
                modifiers: Modifiers::empty(),
            },
        );
        assert_eq!(
            "XF86Screensaver".parse::<Key>().unwrap(),
            Key {
                trigger: Trigger::Keysym(Keysym::XF86_Screensaver),
                modifiers: Modifiers::empty(),
            }
        );
        assert_eq!(
            "xf86screensaver".parse::<Key>().unwrap(),
            Key {
                trigger: Trigger::Keysym(Keysym::XF86_ScreenSaver),
                modifiers: Modifiers::empty(),
            }
        );

        // And now with the auto-prepended XF86 prefix.
        assert_eq!(
            "ScreenSaver".parse::<Key>().unwrap(),
            Key {
                trigger: Trigger::Keysym(Keysym::XF86_ScreenSaver),
                modifiers: Modifiers::empty(),
            },
        );
        assert_eq!(
            "Screensaver".parse::<Key>().unwrap(),
            Key {
                trigger: Trigger::Keysym(Keysym::XF86_Screensaver),
                modifiers: Modifiers::empty(),
            }
        );
        assert_eq!(
            "screensaver".parse::<Key>().unwrap(),
            Key {
                trigger: Trigger::Keysym(Keysym::XF86_ScreenSaver),
                modifiers: Modifiers::empty(),
            }
        );
    }

    #[test]
    fn parse_iso_level_shifts() {
        assert_eq!(
            "ISO_Level3_Shift+A".parse::<Key>().unwrap(),
            Key {
                trigger: Trigger::Keysym(Keysym::a),
                modifiers: Modifiers::ISO_LEVEL3_SHIFT
            },
        );
        assert_eq!(
            "Mod5+A".parse::<Key>().unwrap(),
            Key {
                trigger: Trigger::Keysym(Keysym::a),
                modifiers: Modifiers::ISO_LEVEL3_SHIFT
            },
        );

        assert_eq!(
            "ISO_Level5_Shift+A".parse::<Key>().unwrap(),
            Key {
                trigger: Trigger::Keysym(Keysym::a),
                modifiers: Modifiers::ISO_LEVEL5_SHIFT
            },
        );
        assert_eq!(
            "Mod3+A".parse::<Key>().unwrap(),
            Key {
                trigger: Trigger::Keysym(Keysym::a),
                modifiers: Modifiers::ISO_LEVEL5_SHIFT
            },
        );
    }

    #[test]
    fn parse_bare_modifier_spellings() {
        // Each modifier can be spelled with its canonical name or an alias. A bare modifier key
        // parses to `Trigger::Modifier`.
        for (text, modifier) in [
            ("Ctrl", ModKey::Ctrl),
            ("Control", ModKey::Ctrl),
            ("Shift", ModKey::Shift),
            ("Alt", ModKey::Alt),
            ("Super", ModKey::Super),
            ("Win", ModKey::Super),
            ("Mod5", ModKey::IsoLevel3Shift),
            ("ISO_Level3_Shift", ModKey::IsoLevel3Shift),
            ("Mod3", ModKey::IsoLevel5Shift),
            ("ISO_Level5_Shift", ModKey::IsoLevel5Shift),
        ] {
            assert_eq!(
                text.parse::<Key>().unwrap(),
                Key {
                    trigger: Trigger::Modifier(modifier),
                    modifiers: modifier.to_modifiers(),
                },
                "parsing `{text}`",
            );
        }
    }

    #[test]
    fn parse_mod_as_trigger() {
        // The mod key keeps its own trigger, which is resolved to a real modifier at runtime.
        assert_eq!(
            "Mod".parse::<Key>().unwrap(),
            Key {
                trigger: Trigger::CompositorMod,
                modifiers: Modifiers::COMPOSITOR,
            },
        );

        // It can also be used together with other held modifiers.
        assert_eq!(
            "Ctrl+Mod".parse::<Key>().unwrap(),
            Key {
                trigger: Trigger::CompositorMod,
                modifiers: Modifiers::CTRL | Modifiers::COMPOSITOR,
            },
        );

        // `Mod` also works as a held modifier of another trigger.
        assert_eq!(
            "Mod+Control_L".parse::<Key>().unwrap(),
            Key {
                trigger: Trigger::Keysym(Keysym::Control_L),
                modifiers: Modifiers::COMPOSITOR | Modifiers::CTRL,
            },
        );
    }

    #[test]
    fn parse_multi_modifier_triggers() {
        // A modifier key as the trigger with another held modifier, either spelled as the modifier
        // itself or as its keysym.
        assert_eq!(
            "Ctrl+Alt".parse::<Key>().unwrap(),
            Key {
                trigger: Trigger::Modifier(ModKey::Alt),
                modifiers: Modifiers::CTRL | Modifiers::ALT,
            },
        );
        assert_eq!(
            "Ctrl+Alt_L".parse::<Key>().unwrap(),
            Key {
                trigger: Trigger::Keysym(Keysym::Alt_L),
                modifiers: Modifiers::CTRL | Modifiers::ALT,
            },
        );
    }

    #[test]
    fn parse_normalizes_redundant_modifiers() {
        let key = |text: &str| text.parse::<Key>().unwrap();

        // Modifier triggers can be written with or without the redundant modifier.
        assert_eq!(key("Alt+Alt_L"), key("Alt_L"));
        assert_eq!(key("Alt+Alt_R"), key("Alt_R"));
        assert_eq!(key("Alt+Alt"), key("Alt"));
        assert_eq!(key("Alt+Alt+Alt_L"), key("Alt_L"));
        assert_eq!(key("Ctrl+Shift+Control_L"), key("Shift+Control_L"));
        assert_eq!(key("Mod+Mod"), key("Mod"));

        // Duplicate modifiers are fine with non-modifier triggers.
        assert_eq!(key("Alt+Alt+Q"), key("Alt+Q"));
        assert_eq!(key("Mod+Mod+Q"), key("Mod+Q"));

        // Still works if spelled differently.
        assert_eq!(key("Ctrl+Control+Q"), key("Ctrl+Q"));
        assert_eq!(key("Super+Win+Q"), key("Super+Q"));
    }

    #[test]
    fn parse_modifier_trigger_includes_implied_modifier() {
        // The modifier that the trigger implies is part of the key.
        assert_eq!(
            "Alt_L".parse::<Key>().unwrap(),
            Key {
                trigger: Trigger::Keysym(Keysym::Alt_L),
                modifiers: Modifiers::ALT,
            },
        );
        assert_eq!(
            "Alt+Alt".parse::<Key>().unwrap(),
            Key {
                trigger: Trigger::Modifier(ModKey::Alt),
                modifiers: Modifiers::ALT,
            },
        );
        // `Mod` is only resolved at runtime, so no trigger implies it.
        assert_eq!(
            "Alt+Mod".parse::<Key>().unwrap(),
            Key {
                trigger: Trigger::CompositorMod,
                modifiers: Modifiers::ALT | Modifiers::COMPOSITOR,
            },
        );
    }

    #[test]
    fn key_conflicts() {
        let key = |text: &str| text.parse::<Key>().unwrap();

        // Alt_L and Alt_R conflict with Alt, and vice versa.
        let alt = key("Alt");
        let alt_l = key("Alt_L");
        let alt_r = key("Alt_R");
        assert!(alt.conflicts_with(&alt_l));
        assert!(alt.conflicts_with(&alt_r));
        assert!(alt_l.conflicts_with(&alt));
        assert!(alt_r.conflicts_with(&alt));

        // The left and right keysyms are different keys.
        assert!(!alt_l.conflicts_with(&alt_r));

        // Different modifiers mean different binds, even for the same trigger.
        assert!(!alt.conflicts_with(&key("Ctrl+Alt_L")));

        // Non-overlapping triggers don't conflict, even with the same modifiers.
        assert!(!key("Mod+Q").conflicts_with(&key("Mod+L")));
        assert!(!key("Q").conflicts_with(&key("Ctrl")));

        // Identical keys are duplicates.
        assert!(key("Q").conflicts_with(&key("Q")));
        assert!(alt.conflicts_with(&alt));

        // The compositor mod key is only resolved at runtime, so `Mod` isn't compared against the
        // mod key's own spellings.
        let compositor_mod = key("Mod");
        assert!(!compositor_mod.conflicts_with(&key("Super_L")));
        assert!(!compositor_mod.conflicts_with(&key("Super")));

        // Generic-modifier-only keys are order-independent.
        assert!(key("Ctrl+Alt").conflicts_with(&key("Alt+Ctrl")));
        assert!(key("Mod+Shift+Alt").conflicts_with(&key("Shift+Alt+Mod")));
        assert!(key("Ctrl+Alt").conflicts_with(&key("Alt+Control_L")));
        assert!(key("Ctrl+Alt_L").conflicts_with(&key("Alt+Ctrl")));

        assert!(!key("Ctrl+Alt").conflicts_with(&key("Ctrl+Alt+A")));
    }

    #[test]
    fn null_byte() {
        // Shouldn't panic.
        let _ = "W\0".parse::<Key>();
    }
}
