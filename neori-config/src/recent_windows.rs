use smithay::input::keyboard::Keysym;

use crate::{Action, Bind, Color, FloatOrInt, Key, Modifiers, Trigger};

#[derive(Debug, PartialEq)]
pub struct RecentWindows {
    pub on: bool,
    pub debounce_ms: u16,
    pub open_delay_ms: u16,
    pub highlight: MruHighlight,
    pub previews: MruPreviews,
    pub binds: Vec<Bind>,
}

impl Default for RecentWindows {
    fn default() -> Self {
        RecentWindows {
            on: true,
            debounce_ms: 750,
            open_delay_ms: 150,
            highlight: MruHighlight::default(),
            previews: MruPreviews::default(),
            binds: default_binds(),
        }
    }
}

#[allow(dead_code)]
#[derive(Debug, Default, PartialEq)]
pub struct RecentWindowsPart {
    pub on: bool,
    pub off: bool,
    pub debounce_ms: Option<u16>,
    pub open_delay_ms: Option<u16>,
    pub highlight: Option<MruHighlightPart>,
    pub previews: Option<MruPreviewsPart>,
    pub binds: Option<MruBinds>,
}

#[derive(Debug, PartialEq)]
pub struct MruHighlight {
    pub active_color: Color,
    pub urgent_color: Color,
    pub padding: f64,
    pub corner_radius: f64,
}

impl Default for MruHighlight {
    fn default() -> Self {
        Self {
            active_color: Color::new_unpremul(0.6, 0.6, 0.6, 1.),
            urgent_color: Color::new_unpremul(1., 0.6, 0.6, 1.),
            padding: 30.,
            corner_radius: 0.,
        }
    }
}

#[allow(dead_code)]
#[derive(Debug, Default, PartialEq)]
pub struct MruHighlightPart {
    pub active_color: Option<Color>,
    pub urgent_color: Option<Color>,
    pub padding: Option<FloatOrInt<0, 65535>>,
    pub corner_radius: Option<FloatOrInt<0, 65535>>,
}

#[derive(Debug, Clone, Copy, PartialEq)]
pub struct MruPreviews {
    pub max_height: f64,
    pub max_scale: f64,
}

impl Default for MruPreviews {
    fn default() -> Self {
        Self {
            max_height: 480.,
            max_scale: 0.5,
        }
    }
}

#[allow(dead_code)]
#[derive(Debug, Default, PartialEq)]
pub struct MruPreviewsPart {
    pub max_height: Option<FloatOrInt<1, 65535>>,
    pub max_scale: Option<FloatOrInt<0, 1>>,
}

#[derive(Debug, Clone, PartialEq)]
pub struct MruBind {
    pub key: Key,
    pub action: MruAction,
    pub allow_inhibiting: bool,
    pub hotkey_overlay_title: Option<Option<String>>,
}

impl From<MruBind> for Bind {
    fn from(x: MruBind) -> Self {
        Self {
            key: x.key,
            action: Some(Action::from(x.action)),
            script_fn: None,
            repeat: true,
            cooldown: None,
            allow_when_locked: false,
            allow_inhibiting: x.allow_inhibiting,
            hotkey_overlay_title: x.hotkey_overlay_title,
        }
    }
}

pub use neori_ipc::{MruDirection, MruFilter, MruScope};

#[derive(Debug, Clone, PartialEq)]
pub enum MruAction {
    NextWindow(Option<MruScope>, MruFilter),
    PreviousWindow(Option<MruScope>, MruFilter),
}

impl From<MruAction> for Action {
    fn from(x: MruAction) -> Self {
        match x {
            MruAction::NextWindow(scope, filter) => Self::MruAdvance {
                direction: MruDirection::Forward,
                scope,
                filter: Some(filter),
            },
            MruAction::PreviousWindow(scope, filter) => Self::MruAdvance {
                direction: MruDirection::Backward,
                scope,
                filter: Some(filter),
            },
        }
    }
}

#[derive(Debug, Default, PartialEq)]
pub struct MruBinds(pub Vec<MruBind>);

fn default_binds() -> Vec<Bind> {
    let mut rv = Vec::new();

    let mut push = |trigger, base_mod, filter| {
        rv.push(Bind::from(MruBind {
            key: Key {
                trigger: Trigger::Keysym(trigger),
                modifiers: base_mod,
            },
            action: MruAction::NextWindow(None, filter),
            allow_inhibiting: true,
            hotkey_overlay_title: None,
        }));
        rv.push(Bind::from(MruBind {
            key: Key {
                trigger: Trigger::Keysym(trigger),
                modifiers: base_mod | Modifiers::SHIFT,
            },
            action: MruAction::PreviousWindow(None, filter),
            allow_inhibiting: true,
            hotkey_overlay_title: None,
        }));
    };

    for base_mod in [Modifiers::ALT, Modifiers::COMPOSITOR] {
        push(Keysym::Tab, base_mod, MruFilter::All);
        push(Keysym::grave, base_mod, MruFilter::AppId);
    }

    rv
}
