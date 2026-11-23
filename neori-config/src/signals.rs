//! Signal handlers registered from Lua.
//!
//! A signal is a name plus a list of Lua functions. Global signals are connected with
//! `neori.connect(name, fn)` (or `neori.on`), and object signals with `obj:connect(name, fn)` on a
//! `Window`, `Workspace` or `Output`. The compositor emits the built-in signals (`window-closed`,
//! `workspace-focused`, …); Lua can emit any signal with `neori.emit` / `obj:emit`.
//!
//! A built-in event about an object also goes to the object's own handlers under the event's
//! short name: `window-closed` reaches `w:connect("closed", fn)`, `monitor-removed` reaches
//! `out:connect("removed", fn)`.

use std::collections::HashMap;
use std::sync::{Arc, Mutex};

/// The kinds of objects that can carry signals.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ObjectKind {
    Window,
    Workspace,
    Output,
    Device,
    Layer,
}

/// The object a signal is attached to.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub enum SignalTarget {
    /// A window, by its id.
    Window(u64),
    /// A workspace, by its id.
    Workspace(u64),
    /// An output, by its connector name. Output handlers survive the output being unplugged and
    /// plugged back in.
    Output(String),
    /// An input device, by its sysname. Its handlers are dropped when it's removed: another
    /// device may get the same sysname later.
    Device(String),
    /// A layer-shell surface, by its id.
    Layer(u64),
}

impl SignalTarget {
    pub fn kind(&self) -> ObjectKind {
        match self {
            SignalTarget::Window(_) => ObjectKind::Window,
            SignalTarget::Workspace(_) => ObjectKind::Workspace,
            SignalTarget::Output(_) => ObjectKind::Output,
            SignalTarget::Device(_) => ObjectKind::Device,
            SignalTarget::Layer(_) => ObjectKind::Layer,
        }
    }
}

/// Lua functions connected to global and object signals.
#[derive(Debug, Default)]
pub struct Signals {
    global: HashMap<String, Vec<mlua::Function>>,
    objects: HashMap<SignalTarget, HashMap<String, Vec<mlua::Function>>>,
}

pub type SharedSignals = Arc<Mutex<Signals>>;

/// Removes `fun` from `handlers`, or every handler if `fun` is `None`. Returns whether anything
/// was removed.
fn remove(handlers: &mut Vec<mlua::Function>, fun: Option<&mlua::Function>) -> bool {
    let before = handlers.len();
    match fun {
        Some(fun) => handlers.retain(|f| f != fun),
        None => handlers.clear(),
    }
    handlers.len() != before
}

impl Signals {
    /// Connects `fun` to the signal `name`, globally or on `target`.
    pub fn connect(&mut self, target: Option<SignalTarget>, name: &str, fun: mlua::Function) {
        let map = match target {
            None => &mut self.global,
            Some(target) => self.objects.entry(target).or_default(),
        };
        map.entry(name.to_owned()).or_default().push(fun);
    }

    /// Disconnects `fun` from the signal `name`, or every handler of `name` if `fun` is `None`.
    /// Returns whether anything was disconnected.
    pub fn disconnect(
        &mut self,
        target: Option<&SignalTarget>,
        name: &str,
        fun: Option<&mlua::Function>,
    ) -> bool {
        let removed = match target {
            None => self.global.get_mut(name).is_some_and(|fs| remove(fs, fun)),
            Some(target) => self
                .objects
                .get_mut(target)
                .and_then(|signals| signals.get_mut(name))
                .is_some_and(|fs| remove(fs, fun)),
        };
        self.prune(target, name);
        removed
    }

    /// Drops empty handler lists so that [`Signals::handlers`] stays cheap.
    fn prune(&mut self, target: Option<&SignalTarget>, name: &str) {
        match target {
            None => {
                if self.global.get(name).is_some_and(Vec::is_empty) {
                    self.global.remove(name);
                }
            }
            Some(target) => {
                if let Some(signals) = self.objects.get_mut(target) {
                    if signals.get(name).is_some_and(Vec::is_empty) {
                        signals.remove(name);
                    }
                    if signals.is_empty() {
                        self.objects.remove(target);
                    }
                }
            }
        }
    }

    /// The handlers of the signal `name`, globally or on `target`, in connection order.
    pub fn handlers(&self, target: Option<&SignalTarget>, name: &str) -> Vec<mlua::Function> {
        let handlers = match target {
            None => self.global.get(name),
            Some(target) => self.objects.get(target).and_then(|s| s.get(name)),
        };
        handlers.cloned().unwrap_or_default()
    }

    /// Whether any global handler is connected to `name`.
    pub fn has_global(&self, name: &str) -> bool {
        self.global.contains_key(name)
    }

    /// Whether any handler is connected to the global signal `name`, or to `object_name` on an
    /// object of `kind`.
    pub fn has_handlers(&self, name: &str, kind: Option<ObjectKind>, object_name: &str) -> bool {
        self.global.contains_key(name)
            || kind.is_some_and(|kind| {
                self.objects.iter().any(|(target, signals)| {
                    target.kind() == kind && signals.contains_key(object_name)
                })
            })
    }

    /// Whether any object handlers are connected at all.
    pub fn has_objects(&self) -> bool {
        !self.objects.is_empty()
    }

    /// Every global signal with handlers, and how many.
    pub fn global_counts(&self) -> impl Iterator<Item = (&str, usize)> {
        self.global
            .iter()
            .map(|(name, fs)| (name.as_str(), fs.len()))
    }

    /// Every signal of `target` with handlers, and how many.
    pub fn object_counts(&self, target: &SignalTarget) -> impl Iterator<Item = (&str, usize)> {
        self.objects
            .get(target)
            .into_iter()
            .flat_map(|signals| signals.iter().map(|(name, fs)| (name.as_str(), fs.len())))
    }

    /// How many handlers are connected to the signal `name` across all objects of `kind`.
    pub fn object_total(&self, kind: ObjectKind, name: &str) -> usize {
        self.objects
            .iter()
            .filter(|(target, _)| target.kind() == kind)
            .filter_map(|(_, signals)| signals.get(name))
            .map(Vec::len)
            .sum()
    }

    /// Disconnects every handler of `target`, for an object that no longer exists.
    pub fn remove_target(&mut self, target: &SignalTarget) {
        self.objects.remove(target);
    }

    /// Disconnects everything.
    pub fn clear(&mut self) {
        self.global.clear();
        self.objects.clear();
    }
}

/// A signal that the compositor emits.
#[derive(Debug)]
pub struct BuiltinSignal {
    pub name: &'static str,
    /// What happened, for `neori.signals()`.
    pub description: &'static str,
    /// What the handlers receive, for `neori.signals()`.
    pub args: &'static str,
    /// The kind of object that also gets this signal under its short name
    /// ([`object_signal`]). `None` when there is none, or when a handler couldn't be connected
    /// to the object in time (`window-opened`: there's no `Window` before it opens).
    pub object: Option<ObjectKind>,
}

const fn builtin(
    name: &'static str,
    description: &'static str,
    args: &'static str,
    object: Option<ObjectKind>,
) -> BuiltinSignal {
    BuiltinSignal {
        name,
        description,
        args,
        object,
    }
}

/// Every signal that the compositor emits.
pub const BUILTIN_SIGNALS: &[BuiltinSignal] = &[
    builtin("startup", "the compositor started", "nothing", None),
    builtin(
        "monitor-added",
        "a monitor was connected",
        "connector name, Output",
        Some(ObjectKind::Output),
    ),
    builtin(
        "monitor-removed",
        "a monitor was disconnected",
        "connector name, Output",
        Some(ObjectKind::Output),
    ),
    builtin("lid-open", "the laptop lid was opened", "nothing", None),
    builtin("lid-closed", "the laptop lid was closed", "nothing", None),
    builtin(
        "tablet-mode-on",
        "tablet mode was turned on",
        "nothing",
        None,
    ),
    builtin(
        "tablet-mode-off",
        "tablet mode was turned off",
        "nothing",
        None,
    ),
    builtin(
        "window-opening",
        "a window is about to open; w:open_* methods set where",
        "Window (only app_id and title are known yet)",
        None,
    ),
    builtin("window-opened", "a window opened", "Window", None),
    builtin(
        "window-closed",
        "a window closed",
        "Window (its last known state)",
        Some(ObjectKind::Window),
    ),
    builtin(
        "window-focused",
        "a window got keyboard focus",
        "Window",
        Some(ObjectKind::Window),
    ),
    builtin(
        "workspace-focused",
        "a workspace became the active one",
        "Workspace",
        Some(ObjectKind::Workspace),
    ),
    builtin(
        "workspace-added",
        "a workspace was created",
        "Workspace",
        None,
    ),
    builtin(
        "workspace-removed",
        "a workspace was removed",
        "Workspace (its last known state)",
        Some(ObjectKind::Workspace),
    ),
    builtin(
        "output-focused",
        "an output became the active one",
        "Output",
        Some(ObjectKind::Output),
    ),
    builtin(
        "layout-changed",
        "the keyboard layout changed",
        "layout name",
        None,
    ),
    builtin(
        "device-added",
        "an input device was connected",
        "Device",
        None,
    ),
    builtin(
        "device-removed",
        "an input device was disconnected",
        "Device (its last known state)",
        Some(ObjectKind::Device),
    ),
    builtin(
        "layer-added",
        "a layer-shell surface (a panel, bar, launcher, …) appeared",
        "LayerSurface",
        None,
    ),
    builtin(
        "layer-removed",
        "a layer-shell surface went away",
        "LayerSurface (its last known state)",
        Some(ObjectKind::Layer),
    ),
    builtin(
        "mode-changed",
        "a keybinding mode was entered or left",
        "new mode name or nil, old mode name or nil",
        None,
    ),
    builtin(
        "config-reloaded",
        "the config was reloaded; runs in the newly loaded config",
        "nothing",
        None,
    ),
    builtin(
        "shutdown",
        "the compositor is about to exit",
        "nothing",
        None,
    ),
    builtin(
        "window-title-changed",
        "a window's title changed",
        "Window, old title",
        Some(ObjectKind::Window),
    ),
    builtin(
        "window-fullscreen-changed",
        "a window entered or left fullscreen",
        "Window, was fullscreen",
        Some(ObjectKind::Window),
    ),
    builtin(
        "window-floating-changed",
        "a window moved between the floating and the tiling layout",
        "Window, was floating",
        Some(ObjectKind::Window),
    ),
    builtin(
        "window-urgent-changed",
        "a window started or stopped requesting attention",
        "Window, was urgent",
        Some(ObjectKind::Window),
    ),
    builtin(
        "window-workspace-changed",
        "a window moved to another workspace",
        "Window, old workspace ({ idx, name })",
        Some(ObjectKind::Window),
    ),
    builtin(
        "workspace-name-changed",
        "a workspace was named, renamed or unnamed",
        "Workspace, old name",
        Some(ObjectKind::Workspace),
    ),
    builtin(
        "output-mode-changed",
        "an output's mode changed",
        "Output, old mode",
        Some(ObjectKind::Output),
    ),
];

/// The built-in signal called `name`, accepting `_` in place of `-`.
pub fn find_builtin(name: &str) -> Option<&'static BuiltinSignal> {
    let same = |builtin: &str| {
        builtin.len() == name.len()
            && builtin
                .bytes()
                .zip(name.bytes())
                .all(|(b, n)| b == n || (b == b'-' && n == b'_'))
    };
    BUILTIN_SIGNALS.iter().find(|s| same(s.name))
}

/// The canonical name of a built-in global signal, accepting `_` in place of `-`.
pub fn builtin_signal(name: &str) -> Option<&'static str> {
    find_builtin(name).map(|s| s.name)
}

/// The built-in signals that also reach objects of `kind`.
pub fn builtin_object_signals(kind: ObjectKind) -> impl Iterator<Item = &'static BuiltinSignal> {
    BUILTIN_SIGNALS
        .iter()
        .filter(move |s| s.object == Some(kind))
}

/// Every built-in signal name, for error messages.
pub fn builtin_signal_names() -> String {
    let names: Vec<_> = BUILTIN_SIGNALS.iter().map(|s| s.name).collect();
    names.join(", ")
}

/// The name under which the built-in global signal `event` reaches its subject's own handlers:
/// the part after the first `-` (`window-closed` → `closed`).
pub fn object_signal(event: &str) -> &str {
    event.split_once('-').map_or(event, |(_, rest)| rest)
}

/// Whether the built-in global signal `event` means that its subject is gone for good, so its
/// object handlers can be dropped.
pub fn ends_target(event: &str) -> bool {
    matches!(
        event,
        "window-closed" | "workspace-removed" | "device-removed" | "layer-removed"
    )
}

#[cfg(test)]
mod tests {
    use mlua::Lua;

    use super::*;

    fn fun(lua: &Lua) -> mlua::Function {
        lua.create_function(|_, ()| Ok(())).unwrap()
    }

    #[test]
    fn connect_and_disconnect_one_handler() {
        let lua = Lua::new();
        let (a, b) = (fun(&lua), fun(&lua));
        let mut signals = Signals::default();
        signals.connect(None, "x", a.clone());
        signals.connect(None, "x", b.clone());
        assert_eq!(signals.handlers(None, "x"), [a.clone(), b.clone()]);

        assert!(signals.disconnect(None, "x", Some(&a)));
        assert_eq!(signals.handlers(None, "x"), std::slice::from_ref(&b));
        assert!(!signals.disconnect(None, "x", Some(&a)));

        assert!(signals.disconnect(None, "x", Some(&b)));
        assert!(!signals.has_global("x"));
    }

    #[test]
    fn disconnect_without_a_function_removes_all() {
        let lua = Lua::new();
        let mut signals = Signals::default();
        signals.connect(None, "x", fun(&lua));
        signals.connect(None, "x", fun(&lua));
        assert!(signals.disconnect(None, "x", None));
        assert!(signals.handlers(None, "x").is_empty());
        assert!(!signals.disconnect(None, "x", None));
    }

    #[test]
    fn object_signals_are_separate_per_target() {
        let lua = Lua::new();
        let f = fun(&lua);
        let mut signals = Signals::default();
        let w1 = SignalTarget::Window(1);
        let w2 = SignalTarget::Window(2);
        signals.connect(Some(w1.clone()), "closed", f.clone());

        assert_eq!(
            signals.handlers(Some(&w1), "closed"),
            std::slice::from_ref(&f)
        );
        assert!(signals.handlers(Some(&w2), "closed").is_empty());
        assert!(signals.handlers(None, "closed").is_empty());
        assert!(signals
            .handlers(Some(&SignalTarget::Workspace(1)), "closed")
            .is_empty());

        assert!(signals.disconnect(Some(&w1), "closed", Some(&f)));
        assert!(!signals.has_objects());
    }

    #[test]
    fn remove_target_drops_only_that_object() {
        let lua = Lua::new();
        let mut signals = Signals::default();
        let w1 = SignalTarget::Window(1);
        let out = SignalTarget::Output("DP-1".into());
        signals.connect(Some(w1.clone()), "closed", fun(&lua));
        signals.connect(Some(out.clone()), "focused", fun(&lua));
        signals.remove_target(&w1);
        assert!(signals.handlers(Some(&w1), "closed").is_empty());
        assert_eq!(signals.handlers(Some(&out), "focused").len(), 1);
    }

    #[test]
    fn counts() {
        let lua = Lua::new();
        let mut signals = Signals::default();
        let (w1, w2) = (SignalTarget::Window(1), SignalTarget::Window(2));
        let ws = SignalTarget::Workspace(1);
        signals.connect(None, "x", fun(&lua));
        signals.connect(None, "x", fun(&lua));
        signals.connect(Some(w1.clone()), "closed", fun(&lua));
        signals.connect(Some(w2.clone()), "closed", fun(&lua));
        signals.connect(Some(w2.clone()), "poke", fun(&lua));
        signals.connect(Some(ws.clone()), "closed", fun(&lua));

        assert_eq!(signals.global_counts().collect::<Vec<_>>(), [("x", 2)]);
        let mut w2_counts: Vec<_> = signals.object_counts(&w2).collect();
        w2_counts.sort();
        assert_eq!(w2_counts, [("closed", 1), ("poke", 1)]);
        assert_eq!(signals.object_counts(&SignalTarget::Window(3)).count(), 0);
        // Only windows count, not the workspace with a signal of the same name.
        assert_eq!(signals.object_total(ObjectKind::Window, "closed"), 2);
    }

    #[test]
    fn builtin_table_is_consistent() {
        for s in BUILTIN_SIGNALS {
            assert_eq!(builtin_signal(s.name), Some(s.name));
            assert_eq!(builtin_signal(&s.name.replace('-', "_")), Some(s.name));
            // Object signals end their object only through the events listed in ends_target.
            if ends_target(s.name) {
                assert!(s.object.is_some(), "{}", s.name);
            }
        }
        assert_eq!(builtin_signal("window-closedx"), None);
        assert_eq!(builtin_signal("window_closed_"), None);
        let window: Vec<_> = builtin_object_signals(ObjectKind::Window)
            .map(|s| object_signal(s.name))
            .collect();
        assert_eq!(
            window,
            [
                "closed",
                "focused",
                "title-changed",
                "fullscreen-changed",
                "floating-changed",
                "urgent-changed",
                "workspace-changed",
            ]
        );
    }

    #[test]
    fn signal_names() {
        assert_eq!(builtin_signal("window_closed"), Some("window-closed"));
        assert_eq!(builtin_signal("my-signal"), None);
        assert_eq!(object_signal("window-closed"), "closed");
        assert_eq!(object_signal("monitor-removed"), "removed");
        assert_eq!(object_signal("output-focused"), "focused");
        assert!(ends_target("window-closed"));
        assert!(!ends_target("monitor-removed"));
    }
}
