//! Live access to compositor state from Lua callbacks.
//!
//! The Lua API lives in this crate, which can't depend on the compositor. Instead, the compositor
//! implements [`ScriptContext`] and installs itself with [`with_script_context`] for the duration
//! of every Lua callback. Userdata methods like `w:title()` then look up the current state through
//! [`with_context`] instead of reading a snapshot taken before the callback started, and changes
//! like `w:close()` or `neori.action(...)` are carried out right away.
//!
//! # Safety
//!
//! The context is handed to Lua as a lifetime-erased pointer in a thread-local, valid only while
//! [`with_script_context`] runs. Two rules keep this sound:
//!
//! - The pointer is installed and removed by a guard, so it never outlives the borrow it came from,
//!   even if the callback panics.
//! - [`with_context`] lends out the context to one caller at a time. A nested call (for example a
//!   context method that ends up running Lua again) gets `None` instead of a second `&mut`.

use std::cell::Cell;
use std::marker::PhantomData;
use std::ptr::NonNull;
use std::time::Duration;

use neori_ipc::{Action, OutputAction};

use crate::parse_lua::{
    DeviceOp, DeviceSnapshot, OutputSnapshot, PointerSnapshot, WindowSnapshot, WorkspaceSnapshot,
};

/// Compositor state that Lua callbacks can query.
///
/// Every method defaults to "nothing there", so test contexts only need to implement what they
/// use. The compositor implements all of them.
pub trait ScriptContext {
    /// Every mapped window.
    fn windows(&mut self) -> Vec<WindowSnapshot> {
        Vec::new()
    }
    /// The mapped window with this id, if it still exists.
    fn window(&mut self, id: u64) -> Option<WindowSnapshot> {
        self.windows().into_iter().find(|w| w.id == id)
    }
    /// The keyboard-focused window, if any.
    fn focused_window(&mut self) -> Option<WindowSnapshot> {
        self.windows().into_iter().find(|w| w.is_focused)
    }

    /// Every connected output.
    fn outputs(&mut self) -> Vec<OutputSnapshot> {
        Vec::new()
    }
    /// The output with this connector name, if it's still connected.
    fn output(&mut self, name: &str) -> Option<OutputSnapshot> {
        self.outputs().into_iter().find(|o| o.name == name)
    }
    /// The focused output, if any.
    fn focused_output(&mut self) -> Option<OutputSnapshot> {
        self.outputs().into_iter().find(|o| o.is_focused)
    }

    /// Every workspace, in global index order.
    fn workspaces(&mut self) -> Vec<WorkspaceSnapshot> {
        Vec::new()
    }
    /// The workspace with this id, if it still exists.
    fn workspace(&mut self, id: u64) -> Option<WorkspaceSnapshot> {
        self.workspaces().into_iter().find(|ws| ws.id == id)
    }
    /// The focused workspace, if any.
    fn focused_workspace(&mut self) -> Option<WorkspaceSnapshot> {
        self.workspaces().into_iter().find(|ws| ws.is_focused)
    }

    /// The pointer position, if there is a pointer.
    fn pointer(&mut self) -> Option<PointerSnapshot> {
        None
    }

    /// Every input device.
    fn devices(&mut self) -> Vec<DeviceSnapshot> {
        Vec::new()
    }
    /// The input device with this sysname, if it's still connected.
    fn device(&mut self, sysname: &str) -> Option<DeviceSnapshot> {
        self.devices().into_iter().find(|d| d.sysname == sysname)
    }

    /// The name of the active keyboard layout.
    fn keyboard_layout(&mut self) -> Option<String> {
        None
    }

    /// Carries out `action` right away.
    fn do_action(&mut self, action: Action);
    /// Changes the configuration of the output with this connector name right away.
    fn output_action(&mut self, output: &str, action: OutputAction);
    /// Moves the cursor to these global logical coordinates.
    fn warp(&mut self, x: f64, y: f64);
    /// Changes a setting of the input device with this sysname right away.
    fn device_op(&mut self, sysname: &str, op: DeviceOp);

    /// The name of the active keybinding mode, if any.
    fn current_mode(&mut self) -> Option<String> {
        None
    }

    /// Arms Lua timer `id` to fire after `timeout`, replacing its countdown if it's already
    /// armed.
    fn arm_timer(&mut self, id: u64, timeout: Duration) {
        let _ = (id, timeout);
    }
    /// Disarms Lua timer `id`, if it's armed.
    fn disarm_timer(&mut self, id: u64) {
        let _ = id;
    }
}

type ContextPtr = NonNull<dyn ScriptContext + 'static>;

thread_local! {
    /// The context of the Lua callback currently running on this thread.
    static CONTEXT: Cell<Option<ContextPtr>> = const { Cell::new(None) };
    /// Whether [`with_context`] has currently lent out the context.
    static BORROWED: Cell<bool> = const { Cell::new(false) };
}

/// Restores the previously installed context when dropped.
struct InstallGuard<'a> {
    previous: Option<ContextPtr>,
    previous_borrowed: bool,
    _borrow: PhantomData<&'a mut ()>,
}

impl Drop for InstallGuard<'_> {
    fn drop(&mut self) {
        CONTEXT.with(|c| c.set(self.previous));
        BORROWED.with(|b| b.set(self.previous_borrowed));
    }
}

/// Runs `f` with `ctx` available to Lua through [`with_context`].
///
/// Call this around every invocation of a Lua callback.
pub fn with_script_context<R>(ctx: &mut dyn ScriptContext, f: impl FnOnce() -> R) -> R {
    let ptr: NonNull<dyn ScriptContext + '_> = NonNull::from(ctx);
    // SAFETY: only the lifetime is erased. The guard below removes the pointer before this
    // function returns, so it's never used after the borrow of `ctx` ends.
    let ptr: ContextPtr = unsafe { std::mem::transmute(ptr) };

    let _guard = InstallGuard {
        previous: CONTEXT.with(|c| c.replace(Some(ptr))),
        previous_borrowed: BORROWED.with(|b| b.replace(false)),
        _borrow: PhantomData,
    };
    f()
}

/// Clears the borrow flag when dropped.
struct BorrowGuard;

impl Drop for BorrowGuard {
    fn drop(&mut self) {
        BORROWED.with(|b| b.set(false));
    }
}

/// Runs `f` with the context of the current Lua callback.
///
/// Returns `None` outside of a callback (for example while the config itself is being evaluated),
/// or if the context is already lent out further up the stack.
pub fn with_context<R>(f: impl FnOnce(&mut dyn ScriptContext) -> R) -> Option<R> {
    let ptr = CONTEXT.with(Cell::get)?;
    if BORROWED.with(|b| b.replace(true)) {
        return None;
    }
    let _guard = BorrowGuard;
    // SAFETY: the pointer is valid while installed (see `with_script_context`), and the borrow
    // flag guarantees that this is the only `&mut` to it right now.
    let ctx = unsafe { &mut *ptr.as_ptr() };
    Some(f(ctx))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[derive(Default)]
    struct Fake {
        windows: Vec<WindowSnapshot>,
        lookups: usize,
    }

    impl ScriptContext for Fake {
        fn do_action(&mut self, _: Action) {}
        fn output_action(&mut self, _: &str, _: OutputAction) {}
        fn warp(&mut self, _: f64, _: f64) {}
        fn device_op(&mut self, _: &str, _: DeviceOp) {}

        fn windows(&mut self) -> Vec<WindowSnapshot> {
            self.lookups += 1;
            self.windows.clone()
        }

        fn window(&mut self, id: u64) -> Option<WindowSnapshot> {
            self.lookups += 1;
            self.windows.iter().find(|w| w.id == id).cloned()
        }

        fn focused_window(&mut self) -> Option<WindowSnapshot> {
            self.lookups += 1;
            self.windows.iter().find(|w| w.is_focused).cloned()
        }
    }

    fn window(id: u64) -> WindowSnapshot {
        WindowSnapshot {
            id,
            ..Default::default()
        }
    }

    #[test]
    fn no_context_outside_callbacks() {
        assert!(with_context(|ctx| ctx.windows()).is_none());
    }

    #[test]
    fn context_is_available_inside_and_removed_after() {
        let mut fake = Fake {
            windows: vec![window(1), window(2)],
            ..Default::default()
        };
        let n = with_script_context(&mut fake, || with_context(|ctx| ctx.windows().len()));
        assert_eq!(n, Some(2));
        assert_eq!(fake.lookups, 1);
        assert!(with_context(|ctx| ctx.windows()).is_none());
    }

    #[test]
    fn nested_borrow_is_refused() {
        let mut fake = Fake::default();
        let inner = with_script_context(&mut fake, || {
            with_context(|_| with_context(|ctx| ctx.windows()).is_some())
        });
        assert_eq!(inner, Some(false));
    }

    #[test]
    fn nested_install_restores_the_outer_context() {
        let mut outer = Fake {
            windows: vec![window(1)],
            ..Default::default()
        };
        let mut inner = Fake {
            windows: vec![window(1), window(2), window(3)],
            ..Default::default()
        };
        let counts = with_script_context(&mut outer, || {
            let inner_count =
                with_script_context(&mut inner, || with_context(|c| c.windows().len()));
            let outer_count = with_context(|c| c.windows().len());
            (inner_count, outer_count)
        });
        assert_eq!(counts, (Some(3), Some(1)));
    }

    #[test]
    fn context_is_removed_after_a_panic() {
        let mut fake = Fake::default();
        let result = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
            with_script_context(&mut fake, || panic!("boom"));
        }));
        assert!(result.is_err());
        assert!(with_context(|ctx| ctx.windows()).is_none());
    }
}
