use std::any::Any;
use std::collections::hash_map::Entry;
use std::collections::HashSet;
use std::time::Duration;

use calloop::timer::{TimeoutAction, Timer};
use input::event::gesture::GestureEventCoordinates as _;
use neori_config::{
    Action, Bind, Binds, Config, Key, ModKey, Modifiers, MruDirection, SwitchBinds, Trigger,
};
use neori_ipc::LayoutSwitchTarget;
use smithay::backend::input::{
    AbsolutePositionEvent, Axis, AxisSource, ButtonState, Device, DeviceCapability, Event,
    GestureBeginEvent, GestureEndEvent, GesturePinchUpdateEvent as _, GestureSwipeUpdateEvent as _,
    InputEvent, KeyState, KeyboardKeyEvent, Keycode, MouseButton, PointerAxisEvent,
    PointerButtonEvent, PointerMotionEvent, ProximityState, Switch, SwitchState, SwitchToggleEvent,
    TabletToolButtonEvent, TabletToolEvent, TabletToolProximityEvent, TabletToolTipEvent,
    TabletToolTipState, TouchEvent,
};
use smithay::backend::libinput::LibinputInputBackend;
use smithay::input::dnd::DnDGrab;
use smithay::input::keyboard::{keysyms, FilterResult, Keysym, Layout, ModifiersState};
use smithay::input::pointer::{
    AxisFrame, ButtonEvent, CursorIcon, CursorImageStatus, Focus, GestureHoldBeginEvent,
    GestureHoldEndEvent, GesturePinchBeginEvent, GesturePinchEndEvent, GesturePinchUpdateEvent,
    GestureSwipeBeginEvent, GestureSwipeEndEvent, GestureSwipeUpdateEvent,
    GrabStartData as PointerGrabStartData, MotionEvent, PointerGrab, RelativeMotionEvent,
};
use smithay::input::touch::{
    DownEvent, GrabStartData as TouchGrabStartData, MotionEvent as TouchMotionEvent, UpEvent,
};
use smithay::input::SeatHandler;
use smithay::output::Output;
use smithay::reexports::wayland_server::protocol::wl_data_source::WlDataSource;
use smithay::reexports::wayland_server::protocol::wl_surface::WlSurface;
use smithay::utils::{Logical, Point, Rectangle, Transform, SERIAL_COUNTER};
use smithay::wayland::keyboard_shortcuts_inhibit::KeyboardShortcutsInhibitor;
use smithay::wayland::pointer_constraints::{with_pointer_constraint, PointerConstraint};
use smithay::wayland::tablet_manager::{TabletDescriptor, TabletSeatTrait};
use touch_overview_grab::TouchOverviewGrab;

use self::move_grab::MoveGrab;
use self::pick_color_grab::PickColorGrab;
use self::pick_window_grab::PickWindowGrab;
use self::resize_grab::ResizeGrab;
use self::spatial_movement_grab::SpatialMovementGrab;
#[cfg(feature = "dbus")]
use crate::dbus::freedesktop_a11y::KbMonBlock;
use crate::layout::scrolling::ScrollDirection;
use crate::layout::{ActivateWindow, LayoutElement as _};
use crate::neori::{CastTarget, PointerVisibility, State};
use crate::ui::mru::{WindowMru, WindowMruUi};
use crate::ui::screenshot_ui::ScreenshotUi;
use crate::utils::spawning::{spawn, spawn_sh};
use crate::utils::{center, get_monotonic_time, CastSessionId, ResizeEdge};

pub mod backend_ext;
pub mod move_grab;
pub mod pick_color_grab;
pub mod pick_window_grab;
pub mod resize_grab;
pub mod scroll_swipe_gesture;
pub mod scroll_tracker;
pub mod shake;
pub mod spatial_movement_grab;
pub mod swipe_tracker;
pub mod touch_overview_grab;
pub mod touch_resize_grab;

use backend_ext::{NeoriInputBackend as InputBackend, NeoriInputDevice as _};

pub const DOUBLE_CLICK_TIME: Duration = Duration::from_millis(400);

#[derive(Debug, Clone, Copy, PartialEq)]
pub struct TabletData {
    pub aspect_ratio: f64,
}

pub enum PointerOrTouchStartData<D: SeatHandler> {
    Pointer(PointerGrabStartData<D>),
    Touch(TouchGrabStartData<D>),
}

impl<D: SeatHandler> PointerOrTouchStartData<D> {
    pub fn location(&self) -> Point<f64, Logical> {
        match self {
            PointerOrTouchStartData::Pointer(x) => x.location,
            PointerOrTouchStartData::Touch(x) => x.location,
        }
    }

    pub fn unwrap_pointer(&self) -> &PointerGrabStartData<D> {
        match self {
            PointerOrTouchStartData::Pointer(x) => x,
            PointerOrTouchStartData::Touch(_) => panic!("start_data is not Pointer"),
        }
    }

    pub fn unwrap_touch(&self) -> &TouchGrabStartData<D> {
        match self {
            PointerOrTouchStartData::Pointer(_) => panic!("start_data is not Touch"),
            PointerOrTouchStartData::Touch(x) => x,
        }
    }

    pub fn is_pointer(&self) -> bool {
        matches!(self, Self::Pointer(_))
    }

    pub fn is_touch(&self) -> bool {
        matches!(self, Self::Touch(_))
    }
}

impl State {
    pub fn process_input_event<I: InputBackend + 'static>(&mut self, event: InputEvent<I>)
    where
        I::Device: 'static, // Needed for downcasting.
    {
        let _span = tracy_client::span!("process_input_event");

        // Make sure some logic like workspace clean-up has a chance to run before doing actions.
        self.neori.advance_animations();

        if self.neori.monitors_active {
            // Notify the idle-notifier of activity.
            if should_notify_activity(&event) {
                self.neori.notify_activity();
            }
        } else {
            // Power on monitors if they were off.
            if should_activate_monitors(&event) {
                self.neori.activate_monitors(&mut self.backend);

                // Notify the idle-notifier of activity only if we're also powering on the
                // monitors.
                self.neori.notify_activity();
            }
        }

        if should_reset_pointer_inactivity_timer(&event) {
            self.neori.reset_pointer_inactivity_timer();
        }

        let hide_hotkey_overlay =
            self.neori.hotkey_overlay.is_open() && should_hide_hotkey_overlay(&event);

        let hide_exit_confirm_dialog =
            self.neori.exit_confirm_dialog.is_open() && should_hide_exit_confirm_dialog(&event);

        let mut consumed_by_a11y = false;
        use InputEvent::*;
        match event {
            DeviceAdded { device } => self.on_device_added(device),
            DeviceRemoved { device } => self.on_device_removed(device),
            Keyboard { event } => self.on_keyboard::<I>(event, &mut consumed_by_a11y),
            PointerMotion { event } => self.on_pointer_motion::<I>(event),
            PointerMotionAbsolute { event } => self.on_pointer_motion_absolute::<I>(event),
            PointerButton { event } => self.on_pointer_button::<I>(event),
            PointerAxis { event } => self.on_pointer_axis::<I>(event),
            TabletToolAxis { event } => self.on_tablet_tool_axis::<I>(event),
            TabletToolTip { event } => self.on_tablet_tool_tip::<I>(event),
            TabletToolProximity { event } => self.on_tablet_tool_proximity::<I>(event),
            TabletToolButton { event } => self.on_tablet_tool_button::<I>(event),
            GestureSwipeBegin { event } => self.on_gesture_swipe_begin::<I>(event),
            GestureSwipeUpdate { event } => self.on_gesture_swipe_update::<I>(event),
            GestureSwipeEnd { event } => self.on_gesture_swipe_end::<I>(event),
            GesturePinchBegin { event } => self.on_gesture_pinch_begin::<I>(event),
            GesturePinchUpdate { event } => self.on_gesture_pinch_update::<I>(event),
            GesturePinchEnd { event } => self.on_gesture_pinch_end::<I>(event),
            GestureHoldBegin { event } => self.on_gesture_hold_begin::<I>(event),
            GestureHoldEnd { event } => self.on_gesture_hold_end::<I>(event),
            TouchDown { event } => self.on_touch_down::<I>(event),
            TouchMotion { event } => self.on_touch_motion::<I>(event),
            TouchUp { event } => self.on_touch_up::<I>(event),
            TouchCancel { event } => self.on_touch_cancel::<I>(event),
            TouchFrame { event } => self.on_touch_frame::<I>(event),
            SwitchToggle { event } => self.on_switch_toggle::<I>(event),
            Special(_) => (),
        }

        // Don't hide overlays if consumed by a11y, so that you can use the screen reader
        // navigation keys.
        if consumed_by_a11y {
            return;
        }

        // Do this last so that screenshot still gets it.
        if hide_hotkey_overlay && self.neori.hotkey_overlay.hide() {
            self.neori.queue_redraw_all();
        }

        if hide_exit_confirm_dialog && self.neori.exit_confirm_dialog.hide() {
            self.neori.queue_redraw_all();
        }
    }

    pub fn process_libinput_event(&mut self, event: &mut InputEvent<LibinputInputBackend>) {
        let _span = tracy_client::span!("process_libinput_event");

        match event {
            InputEvent::DeviceAdded { device } => {
                self.neori.devices.insert(device.clone());

                if device.has_capability(input::DeviceCapability::TabletTool) {
                    match device.size() {
                        Some((w, h)) => {
                            let aspect_ratio = w / h;
                            let data = TabletData { aspect_ratio };
                            self.neori.tablets.insert(device.clone(), data);
                        }
                        None => {
                            warn!("tablet tool device has no size");
                        }
                    }
                }

                if device.has_capability(input::DeviceCapability::Keyboard) {
                    if let Some(led_state) = self
                        .neori
                        .seat
                        .get_keyboard()
                        .map(|keyboard| keyboard.led_state())
                    {
                        device.led_update(led_state.into());
                    }
                }

                if device.has_capability(input::DeviceCapability::Touch) {
                    self.neori.touch.insert(device.clone());
                }

                apply_libinput_settings(&self.neori.config.borrow().input, device);
            }
            InputEvent::DeviceRemoved { device } => {
                self.neori.touch.remove(device);
                self.neori.tablets.remove(device);
                self.neori.devices.remove(device);
            }
            _ => (),
        }
    }

    fn on_device_added(&mut self, device: impl Device) {
        if device.has_capability(DeviceCapability::TabletTool) {
            let tablet_seat = self.neori.seat.tablet_seat();

            let desc = TabletDescriptor::from(&device);
            tablet_seat.add_tablet::<Self>(&self.neori.display_handle, &desc);
        }
        if device.has_capability(DeviceCapability::Touch) && self.neori.seat.get_touch().is_none() {
            self.neori.seat.add_touch();
        }
    }

    fn on_device_removed(&mut self, device: impl Device) {
        if device.has_capability(DeviceCapability::TabletTool) {
            let tablet_seat = self.neori.seat.tablet_seat();

            let desc = TabletDescriptor::from(&device);
            tablet_seat.remove_tablet(&desc);

            // If there are no tablets in seat we can remove all tools
            if tablet_seat.count_tablets() == 0 {
                tablet_seat.clear_tools();
            }
        }
        if device.has_capability(DeviceCapability::Touch) && self.neori.touch.is_empty() {
            self.neori.seat.remove_touch();
        }
    }

    /// Computes the rectangle that covers all outputs in global space.
    fn global_bounding_rectangle(&self) -> Option<Rectangle<i32, Logical>> {
        self.neori.global_space.outputs().fold(
            None,
            |acc: Option<Rectangle<i32, Logical>>, output| {
                self.neori
                    .global_space
                    .output_geometry(output)
                    .map(|geo| acc.map(|acc| acc.merge(geo)).unwrap_or(geo))
            },
        )
    }

    /// Computes the cursor position for the tablet event.
    ///
    /// This function handles the tablet output mapping, as well as coordinate clamping and aspect
    /// ratio correction.
    fn compute_tablet_position<I: InputBackend>(
        &self,
        event: &(impl Event<I> + TabletToolEvent<I>),
    ) -> Option<Point<f64, Logical>>
    where
        I::Device: 'static,
    {
        let device_output = event.device().output(self);
        let device_output = device_output.filter(|output| self.neori.output_exists(output));
        let device_output = device_output.as_ref();
        let mapped_output = device_output.or_else(|| self.neori.output_for_tablet());

        // If the tablet is configured to map to the focused window, use that window's geometry on
        // the mapped output (or on the focused output if no specific output is mapped).
        let map_to_focused_window = self
            .neori
            .config
            .borrow()
            .input
            .tablet
            .map_to_focused_window;
        // But only if the keyboard focus is on the layout, so that it doesn't trigger on the lock
        // screen and such.
        let window_target = if map_to_focused_window && self.neori.keyboard_focus.is_layout() {
            let output = mapped_output.or_else(|| self.neori.layout.active_output());
            output.and_then(|output| {
                let monitor = self.neori.layout.monitor_for_output(output)?;
                let mut rect = monitor.active_window_visual_rectangle()?;
                let output_geo = self.neori.global_space.output_geometry(output)?;
                rect.loc += output_geo.loc.to_f64();
                Some((rect, output))
            })
        } else {
            None
        };

        let (target_geo, keep_ratio, px, transform) = if let Some((rect, output)) = window_target {
            (
                rect,
                true,
                1. / output.current_scale().fractional_scale(),
                output.current_transform(),
            )
        } else if let Some(output) = mapped_output {
            let geo = self.neori.global_space.output_geometry(output).unwrap();
            (
                geo.to_f64(),
                true,
                1. / output.current_scale().fractional_scale(),
                output.current_transform(),
            )
        } else {
            let geo = self.global_bounding_rectangle()?.to_f64();

            // FIXME: this 1 px size should ideally somehow be computed for the rightmost output
            // corresponding to the position on the right when clamping.
            let output = self.neori.global_space.outputs().next().unwrap();
            let scale = output.current_scale().fractional_scale();

            // Do not keep ratio for the unified mode as this is what OpenTabletDriver expects.
            (geo, false, 1. / scale, Transform::Normal)
        };

        let mut pos = {
            let size = transform.invert().transform_size(target_geo.size);
            transform.transform_point_in(event.position_transformed(size.to_i32_round()), &size)
        };

        if keep_ratio {
            pos.x /= target_geo.size.w;
            pos.y /= target_geo.size.h;

            let device = event.device();
            if let Some(device) = (&device as &dyn Any).downcast_ref::<input::Device>() {
                if let Some(data) = self.neori.tablets.get(device) {
                    // This code does the same thing as mutter with "keep aspect ratio" enabled.
                    let size = transform.invert().transform_size(target_geo.size);
                    let output_aspect_ratio = size.w / size.h;
                    let ratio = data.aspect_ratio / output_aspect_ratio;

                    if ratio > 1. {
                        pos.x *= ratio;
                    } else {
                        pos.y /= ratio;
                    }
                }
            };

            pos.x *= target_geo.size.w;
            pos.y *= target_geo.size.h;
        }

        pos.x = pos.x.clamp(0.0, target_geo.size.w - px);
        pos.y = pos.y.clamp(0.0, target_geo.size.h - px);
        Some(pos + target_geo.loc)
    }

    fn is_inhibiting_shortcuts(&self) -> bool {
        self.neori
            .keyboard_focus
            .surface()
            .and_then(|surface| {
                self.neori
                    .keyboard_shortcuts_inhibiting_surfaces
                    .get(surface)
            })
            .is_some_and(KeyboardShortcutsInhibitor::is_active)
    }

    fn on_keyboard<I: InputBackend>(
        &mut self,
        event: I::KeyboardKeyEvent,
        consumed_by_a11y: &mut bool,
    ) {
        let mod_key = self.backend.mod_key(&self.neori.config.borrow());

        let serial = SERIAL_COUNTER.next_serial();
        let time = Event::time_msec(&event);
        let pressed = event.state() == KeyState::Pressed;

        // Stop bind key repeat on any release. This won't work 100% correctly in cases like:
        // 1. Press Mod
        // 2. Press Left (repeat starts)
        // 3. Press PgDown (new repeat starts)
        // 4. Release Left (PgDown repeat stops)
        // But it's good enough for now.
        // FIXME: handle this properly.
        if !pressed {
            if let Some(token) = self.neori.bind_repeat_timer.take() {
                self.neori.event_loop.remove(token);
            }
        }

        if pressed {
            self.hide_cursor_if_needed();
        }

        let is_inhibiting_shortcuts = self.is_inhibiting_shortcuts();

        // Accessibility modifier grabs should override XKB state changes (e.g. Caps Lock), so we
        // need to process them before keyboard.input() below.
        //
        // Other accessibility-grabbed keys should still update our XKB state, but not cause any
        // other changes.
        #[cfg(feature = "dbus")]
        let block = {
            let block = self.a11y_process_key(
                Duration::from_millis(u64::from(time)),
                event.key_code(),
                event.state(),
            );
            if block != KbMonBlock::Pass {
                *consumed_by_a11y = true;
            }
            // The accessibility modifier first press must not change XKB state, so we return
            // early here.
            if block == KbMonBlock::ModifierFirstPress {
                return;
            }
            block
        };
        #[cfg(not(feature = "dbus"))]
        let _ = consumed_by_a11y;

        let Some(Some(bind)) = self.neori.seat.get_keyboard().unwrap().input(
            self,
            event.key_code(),
            event.state(),
            serial,
            time,
            |this, mods, keysym| {
                let key_code = event.key_code();
                let modified = keysym.modified_sym();
                let raw = keysym.raw_latin_sym_or_raw_current_sym();
                let modifiers = modifiers_from_state(*mods);

                // After updating XKB state from accessibility-grabbed keys, return right away and
                // don't handle them.
                #[cfg(feature = "dbus")]
                if block != KbMonBlock::Pass {
                    // HACK: there's a slight problem with this code. Here we filter out keys
                    // consumed by accessibility from getting sent to the Wayland client. However,
                    // the Wayland client can still receive these keys from the wl_keyboard
                    // enter/modifiers events. In particular, this can easily happen when opening
                    // the Orca actions menu with Orca + Shift + A: in most cases, when this menu
                    // opens, Shift is still held down, so the menu receives it in
                    // wl_keyboard.enter/modifiers. Then the menu won't react to Enter presses
                    // until the user taps Shift again to "release" it (since the initial Shift
                    // release will be intercepted here).
                    //
                    // I don't think there's any good way of dealing with this apart from keeping a
                    // separate xkb state for accessibility, so that we can track the pressed
                    // modifiers without accidentally leaking them to wl_keyboard.enter. So for now
                    // let's forward modifier releases to the clients here to deal with the most
                    // common case.
                    if !pressed
                        && matches!(
                            modified,
                            Keysym::Shift_L
                                | Keysym::Shift_R
                                | Keysym::Control_L
                                | Keysym::Control_R
                                | Keysym::Super_L
                                | Keysym::Super_R
                                | Keysym::Alt_L
                                | Keysym::Alt_R
                        )
                    {
                        return FilterResult::Forward;
                    } else {
                        return FilterResult::Intercept(None);
                    }
                }

                if this.neori.exit_confirm_dialog.is_open() && pressed {
                    if raw == Some(Keysym::Return) {
                        info!("quitting after confirming exit dialog");
                        this.neori.stop_signal.stop();
                    }

                    // Don't send this press to any clients.
                    this.neori.suppressed_keys.insert(key_code);
                    return FilterResult::Intercept(None);
                }

                // Check if all modifiers were released while the MRU UI was open. If so, close the
                // UI (which will also transfer the focus to the current MRU UI selection).
                if this.neori.window_mru_ui.is_open() && !pressed && modifiers.is_empty() {
                    this.do_action(Action::MruConfirm, false);

                    if this.neori.suppressed_keys.remove(&key_code) {
                        return FilterResult::Intercept(None);
                    } else {
                        return FilterResult::Forward;
                    }
                }

                if pressed && raw == Some(Keysym::Escape) {
                    // Cancel certain grabs on Escape.
                    let pointer = this.neori.seat.get_pointer().unwrap();
                    if pointer
                        .with_grab(|_, grab| Self::grab_can_be_cancelled_with_esc(grab))
                        .unwrap_or(false)
                    {
                        pointer.unset_grab(this, serial, time);
                        this.neori.suppressed_keys.insert(key_code);
                        return FilterResult::Intercept(None);
                    }
                }

                if let Some(Keysym::space) = raw {
                    this.neori.screenshot_ui.set_space_down(pressed);
                }

                let res = {
                    let config = this.neori.config.borrow();
                    let bindings =
                        make_binds_iter(&config, &mut this.neori.window_mru_ui, modifiers);

                    should_intercept_key(
                        &mut this.neori.suppressed_keys,
                        bindings,
                        mod_key,
                        key_code,
                        modified,
                        raw,
                        pressed,
                        *mods,
                        &this.neori.screenshot_ui,
                        this.neori.config.borrow().input.disable_power_key_handling,
                        is_inhibiting_shortcuts,
                    )
                };

                if matches!(res, FilterResult::Forward) {
                    // If we didn't find any bind, try other hardcoded keys.
                    if this.neori.keyboard_focus.is_overview() && pressed {
                        if let Some(bind) = raw.and_then(|raw| hardcoded_overview_bind(raw, *mods))
                        {
                            this.neori.suppressed_keys.insert(key_code);
                            return FilterResult::Intercept(Some(bind));
                        }
                    }

                    // Interaction with the active window, immediately update the active window's
                    // focus timestamp without waiting for a possible pending MRU lock-in delay.
                    this.neori.mru_apply_keyboard_commit();
                }

                res
            },
        ) else {
            return;
        };

        if !pressed {
            return;
        }

        self.handle_bind(bind.clone());

        self.start_key_repeat(bind);
    }

    fn start_key_repeat(&mut self, bind: Bind) {
        if !bind.repeat {
            return;
        }

        // Stop the previous key repeat if any.
        if let Some(token) = self.neori.bind_repeat_timer.take() {
            self.neori.event_loop.remove(token);
        }

        let config = self.neori.config.borrow();
        let config = &config.input.keyboard;

        let repeat_rate = config.repeat_rate;
        if repeat_rate == 0 {
            return;
        }
        let repeat_duration = Duration::from_secs_f64(1. / f64::from(repeat_rate));

        let repeat_timer =
            Timer::from_duration(Duration::from_millis(u64::from(config.repeat_delay)));

        let token = self
            .neori
            .event_loop
            .insert_source(repeat_timer, move |_, _, state| {
                state.handle_bind(bind.clone());
                TimeoutAction::ToDuration(repeat_duration)
            })
            .unwrap();

        self.neori.bind_repeat_timer = Some(token);
    }

    fn hide_cursor_if_needed(&mut self) {
        // If the pointer is already invisible, don't reset it back to Hidden causing one frame
        // of hover.
        if !self.neori.pointer_visibility.is_visible() {
            return;
        }

        if !self.neori.config.borrow().cursor.hide_when_typing {
            return;
        }

        // neori keeps this set only while actively using a tablet, which means the cursor position
        // is likely to change almost immediately, causing pointer_visibility to just flicker back
        // and forth.
        if self.neori.tablet_cursor_location.is_some() {
            return;
        }

        self.neori.pointer_visibility = PointerVisibility::Hidden;
        self.neori.queue_redraw_all();
    }

    pub fn handle_bind(&mut self, bind: Bind) {
        let Some(cooldown) = bind.cooldown else {
            self.do_action(bind.action, bind.allow_when_locked);
            return;
        };

        // Check this first so that it doesn't trigger the cooldown.
        if self.neori.is_locked() && !(bind.allow_when_locked || allowed_when_locked(&bind.action))
        {
            return;
        }

        match self.neori.bind_cooldown_timers.entry(bind.key) {
            // The bind is on cooldown.
            Entry::Occupied(_) => (),
            Entry::Vacant(entry) => {
                let timer = Timer::from_duration(cooldown);
                let token = self
                    .neori
                    .event_loop
                    .insert_source(timer, move |_, _, state| {
                        if state.neori.bind_cooldown_timers.remove(&bind.key).is_none() {
                            error!("bind cooldown timer entry disappeared");
                        }
                        TimeoutAction::Drop
                    })
                    .unwrap();
                entry.insert(token);

                self.do_action(bind.action, bind.allow_when_locked);
            }
        }
    }

    pub fn do_action(&mut self, action: Action, allow_when_locked: bool) {
        if self.neori.is_locked() && !(allow_when_locked || allowed_when_locked(&action)) {
            return;
        }

        if let Some(touch) = self.neori.seat.get_touch() {
            touch.cancel(self);
        }

        match action {
            Action::Quit(skip_confirmation) => {
                if !skip_confirmation && self.neori.exit_confirm_dialog.show() {
                    self.neori.queue_redraw_all();
                    return;
                }

                info!("quitting as requested");
                self.neori.stop_signal.stop()
            }
            Action::ChangeVt(vt) => {
                self.backend.change_vt(vt);
                // Changing VT may not deliver the key releases, so clear the state.
                self.neori.suppressed_keys.clear();
            }
            Action::Suspend => {
                self.backend.suspend();
                // Suspend may not deliver the key releases, so clear the state.
                self.neori.suppressed_keys.clear();
            }
            Action::PowerOffMonitors => {
                self.neori.deactivate_monitors(&mut self.backend);
            }
            Action::PowerOnMonitors => {
                self.neori.activate_monitors(&mut self.backend);
            }
            Action::ToggleDebugTint => {
                self.backend.toggle_debug_tint();
                self.neori.queue_redraw_all();
            }
            Action::DebugToggleOpaqueRegions => {
                self.neori.debug_draw_opaque_regions = !self.neori.debug_draw_opaque_regions;
                self.neori.queue_redraw_all();
            }
            Action::DebugToggleDamage => {
                self.neori.debug_toggle_damage();
            }
            Action::Spawn(command) => {
                let (token, _) = self.neori.activation_state.create_external_token(None);
                spawn(command, Some(token.clone()));
            }
            Action::SpawnSh(command) => {
                let (token, _) = self.neori.activation_state.create_external_token(None);
                spawn_sh(command, Some(token.clone()));
            }
            Action::DoScreenTransition(delay_ms) => {
                self.backend.with_primary_renderer(|renderer| {
                    self.neori.do_screen_transition(renderer, delay_ms);
                });
            }
            Action::ScreenshotScreen(write_to_disk, show_pointer, path) => {
                let active = self.neori.layout.active_output().cloned();
                if let Some(active) = active {
                    self.backend.with_primary_renderer(|renderer| {
                        if let Err(err) = self.neori.screenshot(
                            renderer,
                            &active,
                            write_to_disk,
                            show_pointer,
                            path,
                        ) {
                            warn!("error taking screenshot: {err:?}");
                        }
                    });
                }
            }
            Action::ConfirmScreenshot { write_to_disk } => {
                self.confirm_screenshot(write_to_disk);
            }
            Action::CancelScreenshot => {
                if !self.neori.screenshot_ui.is_open() {
                    return;
                }

                self.neori.screenshot_ui.close();
                self.neori
                    .cursor_manager
                    .set_cursor_image(CursorImageStatus::default_named());
                self.neori.queue_redraw_all();
            }
            Action::ScreenshotTogglePointer => {
                self.neori.screenshot_ui.toggle_pointer();
                self.neori.queue_redraw_all();
            }
            Action::Screenshot(show_cursor, path) => {
                self.open_screenshot_ui(show_cursor, path);
                self.neori.cancel_mru();
            }
            Action::ScreenshotWindow(write_to_disk, show_pointer, path) => {
                let focus = self.neori.layout.focus_with_output();
                if let Some((mapped, output)) = focus {
                    self.backend.with_primary_renderer(|renderer| {
                        if let Err(err) = self.neori.screenshot_window(
                            renderer,
                            output,
                            mapped,
                            write_to_disk,
                            show_pointer,
                            path,
                        ) {
                            warn!("error taking screenshot: {err:?}");
                        }
                    });
                }
            }
            Action::ScreenshotWindowById {
                id,
                write_to_disk,
                show_pointer,
                path,
            } => {
                let mut windows = self.neori.layout.windows();
                let window = windows.find(|(_, m)| m.id().get() == id);
                if let Some((Some(monitor), mapped)) = window {
                    let output = monitor.output();
                    self.backend.with_primary_renderer(|renderer| {
                        if let Err(err) = self.neori.screenshot_window(
                            renderer,
                            output,
                            mapped,
                            write_to_disk,
                            show_pointer,
                            path,
                        ) {
                            warn!("error taking screenshot: {err:?}");
                        }
                    });
                }
            }
            Action::ToggleKeyboardShortcutsInhibit => {
                if let Some(inhibitor) = self.neori.keyboard_focus.surface().and_then(|surface| {
                    self.neori
                        .keyboard_shortcuts_inhibiting_surfaces
                        .get(surface)
                }) {
                    if inhibitor.is_active() {
                        inhibitor.inactivate();
                    } else {
                        inhibitor.activate();
                    }
                }
            }
            Action::CloseWindow => {
                if let Some(mapped) = self.neori.layout.focus() {
                    mapped.toplevel().send_close();
                }
            }
            Action::CloseWindowById(id) => {
                let window = self
                    .neori
                    .layout
                    .windows()
                    .find(|(_, m)| m.id().get() == id);
                if let Some((_, mapped)) = window {
                    mapped.toplevel().send_close();
                }
            }
            Action::FullscreenWindow => {
                let focus = self.neori.layout.focus().map(|m| m.window.clone());
                if let Some(window) = focus {
                    self.neori.layout.toggle_fullscreen(&window);
                    // FIXME: granular
                    self.neori.queue_redraw_all();
                }
            }
            Action::FullscreenWindowById(id) => {
                let window = self
                    .neori
                    .layout
                    .windows()
                    .find(|(_, m)| m.id().get() == id);
                let window = window.map(|(_, m)| m.window.clone());
                if let Some(window) = window {
                    self.neori.layout.toggle_fullscreen(&window);
                    // FIXME: granular
                    self.neori.queue_redraw_all();
                }
            }
            Action::ToggleWindowedFullscreen => {
                let focus = self.neori.layout.focus().map(|m| m.window.clone());
                if let Some(window) = focus {
                    self.neori.layout.toggle_windowed_fullscreen(&window);
                    // FIXME: granular
                    self.neori.queue_redraw_all();
                }
            }
            Action::ToggleWindowedFullscreenById(id) => {
                let window = self
                    .neori
                    .layout
                    .windows()
                    .find(|(_, m)| m.id().get() == id);
                let window = window.map(|(_, m)| m.window.clone());
                if let Some(window) = window {
                    self.neori.layout.toggle_windowed_fullscreen(&window);
                    // FIXME: granular
                    self.neori.queue_redraw_all();
                }
            }
            Action::FocusWindow(id) => {
                let window = self
                    .neori
                    .layout
                    .windows()
                    .find(|(_, m)| m.id().get() == id);
                let window = window.map(|(_, m)| m.window.clone());
                if let Some(window) = window {
                    self.focus_window(&window);
                }
            }
            Action::FocusWindowInColumn(index) => {
                self.neori.layout.focus_window_in_column(index);
                self.maybe_warp_cursor_to_focus();
                self.neori.layer_shell_on_demand_focus = None;
                // FIXME: granular
                self.neori.queue_redraw_all();
            }
            Action::FocusWindowPrevious => {
                let current = self.neori.layout.focus().map(|win| win.id());
                if let Some(window) = self
                    .neori
                    .layout
                    .windows()
                    .map(|(_, win)| win)
                    .filter(|win| Some(win.id()) != current)
                    .max_by_key(|win| win.get_focus_timestamp())
                    .map(|win| win.window.clone())
                {
                    // Commit current focus so repeated focus-window-previous works as expected.
                    self.neori.mru_apply_keyboard_commit();

                    self.focus_window(&window);
                }
            }
            Action::SwitchLayout(action) => {
                let keyboard = &self.neori.seat.get_keyboard().unwrap();
                keyboard.with_xkb_state(self, |mut state| match action {
                    LayoutSwitchTarget::Next => state.cycle_next_layout(),
                    LayoutSwitchTarget::Prev => state.cycle_prev_layout(),
                    LayoutSwitchTarget::Index(layout) => {
                        let num_layouts = state.xkb().lock().unwrap().layouts().count();
                        if usize::from(layout) >= num_layouts {
                            warn!("requested layout doesn't exist")
                        } else {
                            state.set_layout(Layout(layout.into()))
                        }
                    }
                });
            }
            Action::MoveColumnLeft => {
                if self.neori.screenshot_ui.is_open() {
                    self.neori.screenshot_ui.move_left();
                } else {
                    self.neori.layout.move_left();
                    self.maybe_warp_cursor_to_focus();
                }

                // FIXME: granular
                self.neori.queue_redraw_all();
            }
            Action::MoveColumnRight => {
                if self.neori.screenshot_ui.is_open() {
                    self.neori.screenshot_ui.move_right();
                } else {
                    self.neori.layout.move_right();
                    self.maybe_warp_cursor_to_focus();
                }

                // FIXME: granular
                self.neori.queue_redraw_all();
            }
            Action::MoveColumnToFirst => {
                self.neori.layout.move_column_to_first();
                self.maybe_warp_cursor_to_focus();
                // FIXME: granular
                self.neori.queue_redraw_all();
            }
            Action::MoveColumnToLast => {
                self.neori.layout.move_column_to_last();
                self.maybe_warp_cursor_to_focus();
                // FIXME: granular
                self.neori.queue_redraw_all();
            }
            Action::MoveColumnLeftOrToMonitorLeft => {
                if self.neori.screenshot_ui.is_open() {
                    self.neori.screenshot_ui.move_left();
                } else if let Some(output) = self.neori.output_left() {
                    if self.neori.layout.move_column_left_or_to_output(&output)
                        && !self.maybe_warp_cursor_to_focus_centered()
                    {
                        self.move_cursor_to_output(&output);
                    } else {
                        self.maybe_warp_cursor_to_focus();
                    }
                } else {
                    self.neori.layout.move_left();
                    self.maybe_warp_cursor_to_focus();
                }

                // FIXME: granular
                self.neori.queue_redraw_all();
            }
            Action::MoveColumnRightOrToMonitorRight => {
                if self.neori.screenshot_ui.is_open() {
                    self.neori.screenshot_ui.move_right();
                } else if let Some(output) = self.neori.output_right() {
                    if self.neori.layout.move_column_right_or_to_output(&output)
                        && !self.maybe_warp_cursor_to_focus_centered()
                    {
                        self.move_cursor_to_output(&output);
                    } else {
                        self.maybe_warp_cursor_to_focus();
                    }
                } else {
                    self.neori.layout.move_right();
                    self.maybe_warp_cursor_to_focus();
                }

                // FIXME: granular
                self.neori.queue_redraw_all();
            }
            Action::MoveWindowDown => {
                if self.neori.screenshot_ui.is_open() {
                    self.neori.screenshot_ui.move_down();
                } else {
                    self.neori.layout.move_down();
                    self.maybe_warp_cursor_to_focus();
                }

                // FIXME: granular
                self.neori.queue_redraw_all();
            }
            Action::MoveWindowUp => {
                if self.neori.screenshot_ui.is_open() {
                    self.neori.screenshot_ui.move_up();
                } else {
                    self.neori.layout.move_up();
                    self.maybe_warp_cursor_to_focus();
                }

                // FIXME: granular
                self.neori.queue_redraw_all();
            }
            Action::MoveWindowDownOrToWorkspaceDown => {
                if self.neori.screenshot_ui.is_open() {
                    self.neori.screenshot_ui.move_down();
                } else {
                    self.neori.layout.move_down_or_to_workspace_down();
                    self.maybe_warp_cursor_to_focus();
                }
                // FIXME: granular
                self.neori.queue_redraw_all();
            }
            Action::MoveWindowUpOrToWorkspaceUp => {
                if self.neori.screenshot_ui.is_open() {
                    self.neori.screenshot_ui.move_up();
                } else {
                    self.neori.layout.move_up_or_to_workspace_up();
                    self.maybe_warp_cursor_to_focus();
                }
                // FIXME: granular
                self.neori.queue_redraw_all();
            }
            Action::ConsumeOrExpelWindowLeft => {
                self.neori.layout.consume_or_expel_window_left(None);
                self.maybe_warp_cursor_to_focus();
                // FIXME: granular
                self.neori.queue_redraw_all();
            }
            Action::ConsumeOrExpelWindowLeftById(id) => {
                let window = self
                    .neori
                    .layout
                    .windows()
                    .find(|(_, m)| m.id().get() == id);
                let window = window.map(|(_, m)| m.window.clone());
                if let Some(window) = window {
                    self.neori
                        .layout
                        .consume_or_expel_window_left(Some(&window));
                    self.maybe_warp_cursor_to_focus();
                    // FIXME: granular
                    self.neori.queue_redraw_all();
                }
            }
            Action::ConsumeOrExpelWindowRight => {
                self.neori.layout.consume_or_expel_window_right(None);
                self.maybe_warp_cursor_to_focus();
                // FIXME: granular
                self.neori.queue_redraw_all();
            }
            Action::ConsumeOrExpelWindowRightById(id) => {
                let window = self
                    .neori
                    .layout
                    .windows()
                    .find(|(_, m)| m.id().get() == id);
                let window = window.map(|(_, m)| m.window.clone());
                if let Some(window) = window {
                    self.neori
                        .layout
                        .consume_or_expel_window_right(Some(&window));
                    self.maybe_warp_cursor_to_focus();
                    // FIXME: granular
                    self.neori.queue_redraw_all();
                }
            }
            Action::FocusColumnLeft => {
                self.neori.layout.focus_left();
                self.maybe_warp_cursor_to_focus();
                self.neori.layer_shell_on_demand_focus = None;
                // FIXME: granular
                self.neori.queue_redraw_all();
            }
            Action::FocusColumnLeftUnderMouse => {
                if let Some((output, ws)) = self.neori.workspace_under_cursor(true) {
                    let ws_id = ws.id();
                    let ws = {
                        let mut workspaces = self.neori.layout.workspaces_mut();
                        workspaces.find(|ws| ws.id() == ws_id).unwrap()
                    };
                    ws.focus_left();
                    self.maybe_warp_cursor_to_focus();
                    self.neori.layer_shell_on_demand_focus = None;
                    self.neori.queue_redraw(&output);
                }
            }
            Action::FocusColumnRight => {
                self.neori.layout.focus_right();
                self.maybe_warp_cursor_to_focus();
                self.neori.layer_shell_on_demand_focus = None;
                // FIXME: granular
                self.neori.queue_redraw_all();
            }
            Action::FocusColumnRightUnderMouse => {
                if let Some((output, ws)) = self.neori.workspace_under_cursor(true) {
                    let ws_id = ws.id();
                    let ws = {
                        let mut workspaces = self.neori.layout.workspaces_mut();
                        workspaces.find(|ws| ws.id() == ws_id).unwrap()
                    };
                    ws.focus_right();
                    self.maybe_warp_cursor_to_focus();
                    self.neori.layer_shell_on_demand_focus = None;
                    self.neori.queue_redraw(&output);
                }
            }
            Action::FocusColumnFirst => {
                self.neori.layout.focus_column_first();
                self.maybe_warp_cursor_to_focus();
                self.neori.layer_shell_on_demand_focus = None;
                // FIXME: granular
                self.neori.queue_redraw_all();
            }
            Action::FocusColumnLast => {
                self.neori.layout.focus_column_last();
                self.maybe_warp_cursor_to_focus();
                self.neori.layer_shell_on_demand_focus = None;
                // FIXME: granular
                self.neori.queue_redraw_all();
            }
            Action::FocusColumnRightOrFirst => {
                self.neori.layout.focus_column_right_or_first();
                self.maybe_warp_cursor_to_focus();
                self.neori.layer_shell_on_demand_focus = None;
                // FIXME: granular
                self.neori.queue_redraw_all();
            }
            Action::FocusColumnLeftOrLast => {
                self.neori.layout.focus_column_left_or_last();
                self.maybe_warp_cursor_to_focus();
                self.neori.layer_shell_on_demand_focus = None;
                // FIXME: granular
                self.neori.queue_redraw_all();
            }
            Action::FocusColumn(index) => {
                self.neori.layout.focus_column(index);
                self.maybe_warp_cursor_to_focus();
                self.neori.layer_shell_on_demand_focus = None;
                // FIXME: granular
                self.neori.queue_redraw_all();
            }
            Action::FocusWindowOrMonitorUp => {
                if let Some(output) = self.neori.output_up() {
                    if self.neori.layout.focus_window_up_or_output(&output)
                        && !self.maybe_warp_cursor_to_focus_centered()
                    {
                        self.move_cursor_to_output(&output);
                    } else {
                        self.maybe_warp_cursor_to_focus();
                    }
                } else {
                    self.neori.layout.focus_up();
                    self.maybe_warp_cursor_to_focus();
                }
                self.neori.layer_shell_on_demand_focus = None;

                // FIXME: granular
                self.neori.queue_redraw_all();
            }
            Action::FocusWindowOrMonitorDown => {
                if let Some(output) = self.neori.output_down() {
                    if self.neori.layout.focus_window_down_or_output(&output)
                        && !self.maybe_warp_cursor_to_focus_centered()
                    {
                        self.move_cursor_to_output(&output);
                    } else {
                        self.maybe_warp_cursor_to_focus();
                    }
                } else {
                    self.neori.layout.focus_down();
                    self.maybe_warp_cursor_to_focus();
                }
                self.neori.layer_shell_on_demand_focus = None;

                // FIXME: granular
                self.neori.queue_redraw_all();
            }
            Action::FocusColumnOrMonitorLeft => {
                if let Some(output) = self.neori.output_left() {
                    if self.neori.layout.focus_column_left_or_output(&output)
                        && !self.maybe_warp_cursor_to_focus_centered()
                    {
                        self.move_cursor_to_output(&output);
                    } else {
                        self.maybe_warp_cursor_to_focus();
                    }
                } else {
                    self.neori.layout.focus_left();
                    self.maybe_warp_cursor_to_focus();
                }
                self.neori.layer_shell_on_demand_focus = None;

                // FIXME: granular
                self.neori.queue_redraw_all();
            }
            Action::FocusColumnOrMonitorRight => {
                if let Some(output) = self.neori.output_right() {
                    if self.neori.layout.focus_column_right_or_output(&output)
                        && !self.maybe_warp_cursor_to_focus_centered()
                    {
                        self.move_cursor_to_output(&output);
                    } else {
                        self.maybe_warp_cursor_to_focus();
                    }
                } else {
                    self.neori.layout.focus_right();
                    self.maybe_warp_cursor_to_focus();
                }
                self.neori.layer_shell_on_demand_focus = None;

                // FIXME: granular
                self.neori.queue_redraw_all();
            }
            Action::FocusWindowDown => {
                self.neori.layout.focus_down();
                self.maybe_warp_cursor_to_focus();
                self.neori.layer_shell_on_demand_focus = None;
                // FIXME: granular
                self.neori.queue_redraw_all();
            }
            Action::FocusWindowUp => {
                self.neori.layout.focus_up();
                self.maybe_warp_cursor_to_focus();
                self.neori.layer_shell_on_demand_focus = None;
                // FIXME: granular
                self.neori.queue_redraw_all();
            }
            Action::FocusWindowDownOrColumnLeft => {
                self.neori.layout.focus_down_or_left();
                self.maybe_warp_cursor_to_focus();
                self.neori.layer_shell_on_demand_focus = None;
                // FIXME: granular
                self.neori.queue_redraw_all();
            }
            Action::FocusWindowDownOrColumnRight => {
                self.neori.layout.focus_down_or_right();
                self.maybe_warp_cursor_to_focus();
                self.neori.layer_shell_on_demand_focus = None;
                // FIXME: granular
                self.neori.queue_redraw_all();
            }
            Action::FocusWindowUpOrColumnLeft => {
                self.neori.layout.focus_up_or_left();
                self.maybe_warp_cursor_to_focus();
                self.neori.layer_shell_on_demand_focus = None;
                // FIXME: granular
                self.neori.queue_redraw_all();
            }
            Action::FocusWindowUpOrColumnRight => {
                self.neori.layout.focus_up_or_right();
                self.maybe_warp_cursor_to_focus();
                self.neori.layer_shell_on_demand_focus = None;
                // FIXME: granular
                self.neori.queue_redraw_all();
            }
            Action::FocusWindowOrWorkspaceDown => {
                self.neori.layout.focus_window_or_workspace_down();
                self.maybe_warp_cursor_to_focus();
                self.neori.layer_shell_on_demand_focus = None;
                // FIXME: granular
                self.neori.queue_redraw_all();
            }
            Action::FocusWindowOrWorkspaceUp => {
                self.neori.layout.focus_window_or_workspace_up();
                self.maybe_warp_cursor_to_focus();
                self.neori.layer_shell_on_demand_focus = None;
                // FIXME: granular
                self.neori.queue_redraw_all();
            }
            Action::FocusWindowTop => {
                self.neori.layout.focus_window_top();
                self.maybe_warp_cursor_to_focus();
                self.neori.layer_shell_on_demand_focus = None;
                // FIXME: granular
                self.neori.queue_redraw_all();
            }
            Action::FocusWindowBottom => {
                self.neori.layout.focus_window_bottom();
                self.maybe_warp_cursor_to_focus();
                self.neori.layer_shell_on_demand_focus = None;
                // FIXME: granular
                self.neori.queue_redraw_all();
            }
            Action::FocusWindowDownOrTop => {
                self.neori.layout.focus_window_down_or_top();
                self.maybe_warp_cursor_to_focus();
                self.neori.layer_shell_on_demand_focus = None;
                // FIXME: granular
                self.neori.queue_redraw_all();
            }
            Action::FocusWindowUpOrBottom => {
                self.neori.layout.focus_window_up_or_bottom();
                self.maybe_warp_cursor_to_focus();
                self.neori.layer_shell_on_demand_focus = None;
                // FIXME: granular
                self.neori.queue_redraw_all();
            }
            Action::MoveWindowToWorkspaceDown(focus) => {
                self.neori.layout.move_to_workspace_down(focus);
                self.maybe_warp_cursor_to_focus();
                // FIXME: granular
                self.neori.queue_redraw_all();
            }
            Action::MoveWindowToWorkspaceUp(focus) => {
                self.neori.layout.move_to_workspace_up(focus);
                self.maybe_warp_cursor_to_focus();
                // FIXME: granular
                self.neori.queue_redraw_all();
            }
            Action::MoveWindowToWorkspace(reference, focus) => {
                if let Some((target_output, index)) =
                    self.neori.find_output_and_workspace_index(reference)
                {
                    let target_was_active =
                        self.neori.layout.active_output().is_some_and(|active| {
                            target_output
                                .as_ref()
                                .is_some_and(|target| target == active)
                        });

                    let activate = if focus {
                        ActivateWindow::Smart
                    } else {
                        ActivateWindow::No
                    };

                    self.neori.layout.move_to_workspace(None, index, activate);

                    if focus {
                        // If the target workspace is on another monitor, move
                        // the cursor there.
                        if !target_was_active {
                            if let Some(output) = target_output {
                                if self.neori.layout.active_output() == Some(&output)
                                    && !self.maybe_warp_cursor_to_focus_centered()
                                {
                                    self.move_cursor_to_output(&output);
                                }
                            }
                        }
                        self.maybe_warp_cursor_to_focus();
                    }

                    // FIXME: granular
                    self.neori.queue_redraw_all();
                }
            }
            Action::MoveWindowToWorkspaceById {
                window_id: id,
                reference,
                focus,
            } => {
                let window = self
                    .neori
                    .layout
                    .windows()
                    .find(|(_, m)| m.id().get() == id);
                let window = window.map(|(_, m)| m.window.clone());
                if let Some(window) = window {
                    if let Some((target_output, index)) =
                        self.neori.find_output_and_workspace_index(reference)
                    {
                        let target_was_active =
                            self.neori.layout.active_output().is_some_and(|active| {
                                target_output
                                    .as_ref()
                                    .is_some_and(|target| target == active)
                            });

                        let activate = if focus {
                            ActivateWindow::Smart
                        } else {
                            ActivateWindow::No
                        };

                        self.neori
                            .layout
                            .move_to_workspace(Some(&window), index, activate);

                        // If the active output changed (window was moved and focused).
                        if !target_was_active && focus {
                            if let Some(output) = target_output {
                                if self.neori.layout.active_output() == Some(&output)
                                    && !self.maybe_warp_cursor_to_focus_centered()
                                {
                                    self.move_cursor_to_output(&output);
                                }
                            }
                        }

                        // If we focused the target window.
                        let new_focus = self.neori.layout.focus();
                        if new_focus.is_some_and(|win| win.window == window) {
                            self.maybe_warp_cursor_to_focus();
                        }

                        // FIXME: granular
                        self.neori.queue_redraw_all();
                    }
                }
            }
            Action::MoveColumnToWorkspaceDown(focus) => {
                self.neori.layout.move_column_to_workspace_down(focus);
                self.maybe_warp_cursor_to_focus();
                // FIXME: granular
                self.neori.queue_redraw_all();
            }
            Action::MoveColumnToWorkspaceUp(focus) => {
                self.neori.layout.move_column_to_workspace_up(focus);
                self.maybe_warp_cursor_to_focus();
                // FIXME: granular
                self.neori.queue_redraw_all();
            }
            Action::MoveColumnToWorkspace(reference, focus) => {
                if let Some((target_output, index)) =
                    self.neori.find_output_and_workspace_index(reference)
                {
                    let target_was_active =
                        self.neori.layout.active_output().is_some_and(|active| {
                            target_output
                                .as_ref()
                                .is_some_and(|target| target == active)
                        });

                    self.neori.layout.move_column_to_workspace(index, focus);

                    if focus {
                        if !target_was_active {
                            if let Some(output) = target_output {
                                if self.neori.layout.active_output() == Some(&output)
                                    && !self.maybe_warp_cursor_to_focus_centered()
                                {
                                    self.move_cursor_to_output(&output);
                                }
                            }
                        }
                        self.maybe_warp_cursor_to_focus();
                    }

                    // FIXME: granular
                    self.neori.queue_redraw_all();
                }
            }
            Action::MoveColumnToIndex(idx) => {
                self.neori.layout.move_column_to_index(idx);
                self.maybe_warp_cursor_to_focus();
                // FIXME: granular
                self.neori.queue_redraw_all();
            }
            Action::FocusWorkspaceDown => {
                let output_before = self.neori.layout.active_output().cloned();
                self.neori.layout.switch_workspace_down();
                self.warp_cursor_if_active_output_changed(output_before);
                self.maybe_warp_cursor_to_focus();
                self.neori.layer_shell_on_demand_focus = None;
                // FIXME: granular
                self.neori.queue_redraw_all();
            }
            Action::FocusWorkspaceDownUnderMouse => {
                if let Some(output) = self.neori.output_under_cursor() {
                    self.neori.layout.switch_workspace_down_on_output(&output);
                    self.maybe_warp_cursor_to_focus();
                    self.neori.layer_shell_on_demand_focus = None;
                    self.neori.queue_redraw(&output);
                }
            }
            Action::FocusWorkspaceUp => {
                let output_before = self.neori.layout.active_output().cloned();
                self.neori.layout.switch_workspace_up();
                self.warp_cursor_if_active_output_changed(output_before);
                self.maybe_warp_cursor_to_focus();
                self.neori.layer_shell_on_demand_focus = None;
                // FIXME: granular
                self.neori.queue_redraw_all();
            }
            Action::FocusWorkspaceUpUnderMouse => {
                if let Some(output) = self.neori.output_under_cursor() {
                    self.neori.layout.switch_workspace_up_on_output(&output);
                    self.maybe_warp_cursor_to_focus();
                    self.neori.layer_shell_on_demand_focus = None;
                    self.neori.queue_redraw(&output);
                }
            }
            Action::FocusWorkspace(reference) => {
                if let Some((_target_output, index)) =
                    self.neori.find_output_and_workspace_index(reference)
                {
                    let output_before = self.neori.layout.active_output().cloned();

                    let config = &self.neori.config;
                    if config.borrow().input.workspace_auto_back_and_forth {
                        self.neori
                            .layout
                            .switch_workspace_auto_back_and_forth(index);
                    } else {
                        self.neori.layout.switch_workspace(index);
                    }

                    // Switching to a workspace that lives on another monitor moves the
                    // keyboard focus there, so bring the cursor along. Compare the active
                    // output before and after the switch so this works for workspaces
                    // referenced by index, name, or id alike.
                    self.warp_cursor_if_active_output_changed(output_before);
                    self.maybe_warp_cursor_to_focus();

                    self.neori.layer_shell_on_demand_focus = None;

                    // FIXME: granular
                    self.neori.queue_redraw_all();
                }
            }
            Action::FocusWorkspacePrevious => {
                let output_before = self.neori.layout.active_output().cloned();
                self.neori.layout.switch_workspace_previous();
                self.warp_cursor_if_active_output_changed(output_before);
                self.maybe_warp_cursor_to_focus();
                self.neori.layer_shell_on_demand_focus = None;
                // FIXME: granular
                self.neori.queue_redraw_all();
            }
            Action::MoveWorkspaceDown => {
                self.neori.layout.move_workspace_down();
                // FIXME: granular
                self.neori.queue_redraw_all();
            }
            Action::MoveWorkspaceUp => {
                self.neori.layout.move_workspace_up();
                // FIXME: granular
                self.neori.queue_redraw_all();
            }
            Action::MoveWorkspaceToIndex(new_idx) => {
                self.neori
                    .layout
                    .move_workspace_to_idx(None, new_idx as u32);
                // FIXME: granular
                self.neori.queue_redraw_all();
            }
            Action::MoveWorkspaceToIndexByRef { new_idx, reference } => {
                if let Some(res) = self.neori.find_output_and_workspace_index(reference) {
                    self.neori
                        .layout
                        .move_workspace_to_idx(Some(res), new_idx as u32);
                    // FIXME: granular
                    self.neori.queue_redraw_all();
                }
            }
            Action::SetWorkspaceName(name) => {
                self.neori.layout.set_workspace_name(name, None);
            }
            Action::SetWorkspaceNameByRef { name, reference } => {
                self.neori.layout.set_workspace_name(name, Some(reference));
            }
            Action::UnsetWorkspaceName => {
                self.neori.layout.unset_workspace_name(None);
            }
            Action::UnsetWorkSpaceNameByRef(reference) => {
                self.neori.layout.unset_workspace_name(Some(reference));
            }
            Action::ConsumeWindowIntoColumn => {
                self.neori.layout.consume_into_column();
                // This does not cause immediate focus or window size change, so warping mouse to
                // focus won't do anything here.
                // FIXME: granular
                self.neori.queue_redraw_all();
            }
            Action::ExpelWindowFromColumn => {
                self.neori.layout.expel_from_column();
                self.maybe_warp_cursor_to_focus();
                // FIXME: granular
                self.neori.queue_redraw_all();
            }
            Action::SwapWindowRight => {
                self.neori
                    .layout
                    .swap_window_in_direction(ScrollDirection::Right);
                self.maybe_warp_cursor_to_focus();
                // FIXME: granular
                self.neori.queue_redraw_all();
            }
            Action::SwapWindowLeft => {
                self.neori
                    .layout
                    .swap_window_in_direction(ScrollDirection::Left);
                self.maybe_warp_cursor_to_focus();
                // FIXME: granular
                self.neori.queue_redraw_all();
            }
            Action::ToggleColumnTabbedDisplay => {
                self.neori.layout.toggle_column_tabbed_display();
                self.maybe_warp_cursor_to_focus();
                // FIXME: granular
                self.neori.queue_redraw_all();
            }
            Action::SetColumnDisplay(display) => {
                self.neori.layout.set_column_display(display);
                self.maybe_warp_cursor_to_focus();
                // FIXME: granular
                self.neori.queue_redraw_all();
            }
            Action::SwitchPresetColumnWidth => {
                self.neori.layout.toggle_width(true);
            }
            Action::SwitchPresetColumnWidthBack => {
                self.neori.layout.toggle_width(false);
            }
            Action::SwitchPresetWindowWidth => {
                self.neori.layout.toggle_window_width(None, true);
            }
            Action::SwitchPresetWindowWidthBack => {
                self.neori.layout.toggle_window_width(None, false);
            }
            Action::SwitchPresetWindowWidthById(id) => {
                let window = self
                    .neori
                    .layout
                    .windows()
                    .find(|(_, m)| m.id().get() == id);
                let window = window.map(|(_, m)| m.window.clone());
                if let Some(window) = window {
                    self.neori.layout.toggle_window_width(Some(&window), true);
                }
            }
            Action::SwitchPresetWindowWidthBackById(id) => {
                let window = self
                    .neori
                    .layout
                    .windows()
                    .find(|(_, m)| m.id().get() == id);
                let window = window.map(|(_, m)| m.window.clone());
                if let Some(window) = window {
                    self.neori.layout.toggle_window_width(Some(&window), false);
                }
            }
            Action::SwitchPresetWindowHeight => {
                self.neori.layout.toggle_window_height(None, true);
            }
            Action::SwitchPresetWindowHeightBack => {
                self.neori.layout.toggle_window_height(None, false);
            }
            Action::SwitchPresetWindowHeightById(id) => {
                let window = self
                    .neori
                    .layout
                    .windows()
                    .find(|(_, m)| m.id().get() == id);
                let window = window.map(|(_, m)| m.window.clone());
                if let Some(window) = window {
                    self.neori.layout.toggle_window_height(Some(&window), true);
                }
            }
            Action::SwitchPresetWindowHeightBackById(id) => {
                let window = self
                    .neori
                    .layout
                    .windows()
                    .find(|(_, m)| m.id().get() == id);
                let window = window.map(|(_, m)| m.window.clone());
                if let Some(window) = window {
                    self.neori.layout.toggle_window_height(Some(&window), false);
                }
            }
            Action::CenterColumn => {
                self.neori.layout.center_column();
                // FIXME: granular
                self.neori.queue_redraw_all();
            }
            Action::CenterWindow => {
                self.neori.layout.center_window(None);
                // FIXME: granular
                self.neori.queue_redraw_all();
            }
            Action::CenterWindowById(id) => {
                let window = self
                    .neori
                    .layout
                    .windows()
                    .find(|(_, m)| m.id().get() == id);
                let window = window.map(|(_, m)| m.window.clone());
                if let Some(window) = window {
                    self.neori.layout.center_window(Some(&window));
                    // FIXME: granular
                    self.neori.queue_redraw_all();
                }
            }
            Action::CenterVisibleColumns => {
                self.neori.layout.center_visible_columns();
                // FIXME: granular
                self.neori.queue_redraw_all();
            }
            Action::MaximizeColumn => {
                self.neori.layout.toggle_full_width();
            }
            Action::MaximizeWindowToEdges => {
                let focus = self.neori.layout.focus().map(|m| m.window.clone());
                if let Some(window) = focus {
                    self.neori.layout.toggle_maximized(&window);
                    // FIXME: granular
                    self.neori.queue_redraw_all();
                }
            }
            Action::MaximizeWindowToEdgesById(id) => {
                let window = self
                    .neori
                    .layout
                    .windows()
                    .find(|(_, m)| m.id().get() == id);
                let window = window.map(|(_, m)| m.window.clone());
                if let Some(window) = window {
                    self.neori.layout.toggle_maximized(&window);
                    // FIXME: granular
                    self.neori.queue_redraw_all();
                }
            }
            Action::FocusMonitorLeft => {
                if let Some(output) = self.neori.output_left() {
                    self.neori.layout.focus_output(&output);
                    if !self.maybe_warp_cursor_to_focus_centered() {
                        self.move_cursor_to_output(&output);
                    }
                    self.neori.layer_shell_on_demand_focus = None;
                }
            }
            Action::FocusMonitorRight => {
                if let Some(output) = self.neori.output_right() {
                    self.neori.layout.focus_output(&output);
                    if !self.maybe_warp_cursor_to_focus_centered() {
                        self.move_cursor_to_output(&output);
                    }
                    self.neori.layer_shell_on_demand_focus = None;
                }
            }
            Action::FocusMonitorDown => {
                if let Some(output) = self.neori.output_down() {
                    self.neori.layout.focus_output(&output);
                    if !self.maybe_warp_cursor_to_focus_centered() {
                        self.move_cursor_to_output(&output);
                    }
                    self.neori.layer_shell_on_demand_focus = None;
                }
            }
            Action::FocusMonitorUp => {
                if let Some(output) = self.neori.output_up() {
                    self.neori.layout.focus_output(&output);
                    if !self.maybe_warp_cursor_to_focus_centered() {
                        self.move_cursor_to_output(&output);
                    }
                    self.neori.layer_shell_on_demand_focus = None;
                }
            }
            Action::FocusMonitorPrevious => {
                if let Some(output) = self.neori.output_previous() {
                    self.neori.layout.focus_output(&output);
                    if !self.maybe_warp_cursor_to_focus_centered() {
                        self.move_cursor_to_output(&output);
                    }
                    self.neori.layer_shell_on_demand_focus = None;
                }
            }
            Action::FocusMonitorNext => {
                if let Some(output) = self.neori.output_next() {
                    self.neori.layout.focus_output(&output);
                    if !self.maybe_warp_cursor_to_focus_centered() {
                        self.move_cursor_to_output(&output);
                    }
                    self.neori.layer_shell_on_demand_focus = None;
                }
            }
            Action::FocusMonitor(output) => {
                if let Some(output) = self.neori.output_by_name_match(&output).cloned() {
                    self.neori.layout.focus_output(&output);
                    if !self.maybe_warp_cursor_to_focus_centered() {
                        self.move_cursor_to_output(&output);
                    }
                    self.neori.layer_shell_on_demand_focus = None;
                }
            }
            Action::MoveWindowToMonitorLeft => {
                if let Some(current_output) = self.neori.screenshot_ui.selection_output() {
                    if let Some(target_output) = self.neori.output_left_of(current_output) {
                        self.move_cursor_to_output(&target_output);
                        self.neori.screenshot_ui.move_to_output(target_output);
                    }
                } else if let Some(output) = self.neori.output_left() {
                    self.neori
                        .layout
                        .move_to_output(None, &output, None, ActivateWindow::Smart);
                    self.neori.layout.focus_output(&output);
                    if !self.maybe_warp_cursor_to_focus_centered() {
                        self.move_cursor_to_output(&output);
                    }
                }
            }
            Action::MoveWindowToMonitorRight => {
                if let Some(current_output) = self.neori.screenshot_ui.selection_output() {
                    if let Some(target_output) = self.neori.output_right_of(current_output) {
                        self.move_cursor_to_output(&target_output);
                        self.neori.screenshot_ui.move_to_output(target_output);
                    }
                } else if let Some(output) = self.neori.output_right() {
                    self.neori
                        .layout
                        .move_to_output(None, &output, None, ActivateWindow::Smart);
                    self.neori.layout.focus_output(&output);
                    if !self.maybe_warp_cursor_to_focus_centered() {
                        self.move_cursor_to_output(&output);
                    }
                }
            }
            Action::MoveWindowToMonitorDown => {
                if let Some(current_output) = self.neori.screenshot_ui.selection_output() {
                    if let Some(target_output) = self.neori.output_down_of(current_output) {
                        self.move_cursor_to_output(&target_output);
                        self.neori.screenshot_ui.move_to_output(target_output);
                    }
                } else if let Some(output) = self.neori.output_down() {
                    self.neori
                        .layout
                        .move_to_output(None, &output, None, ActivateWindow::Smart);
                    self.neori.layout.focus_output(&output);
                    if !self.maybe_warp_cursor_to_focus_centered() {
                        self.move_cursor_to_output(&output);
                    }
                }
            }
            Action::MoveWindowToMonitorUp => {
                if let Some(current_output) = self.neori.screenshot_ui.selection_output() {
                    if let Some(target_output) = self.neori.output_up_of(current_output) {
                        self.move_cursor_to_output(&target_output);
                        self.neori.screenshot_ui.move_to_output(target_output);
                    }
                } else if let Some(output) = self.neori.output_up() {
                    self.neori
                        .layout
                        .move_to_output(None, &output, None, ActivateWindow::Smart);
                    self.neori.layout.focus_output(&output);
                    if !self.maybe_warp_cursor_to_focus_centered() {
                        self.move_cursor_to_output(&output);
                    }
                }
            }
            Action::MoveWindowToMonitorPrevious => {
                if let Some(current_output) = self.neori.screenshot_ui.selection_output() {
                    if let Some(target_output) = self.neori.output_previous_of(current_output) {
                        self.move_cursor_to_output(&target_output);
                        self.neori.screenshot_ui.move_to_output(target_output);
                    }
                } else if let Some(output) = self.neori.output_previous() {
                    self.neori
                        .layout
                        .move_to_output(None, &output, None, ActivateWindow::Smart);
                    self.neori.layout.focus_output(&output);
                    if !self.maybe_warp_cursor_to_focus_centered() {
                        self.move_cursor_to_output(&output);
                    }
                }
            }
            Action::MoveWindowToMonitorNext => {
                if let Some(current_output) = self.neori.screenshot_ui.selection_output() {
                    if let Some(target_output) = self.neori.output_next_of(current_output) {
                        self.move_cursor_to_output(&target_output);
                        self.neori.screenshot_ui.move_to_output(target_output);
                    }
                } else if let Some(output) = self.neori.output_next() {
                    self.neori
                        .layout
                        .move_to_output(None, &output, None, ActivateWindow::Smart);
                    self.neori.layout.focus_output(&output);
                    if !self.maybe_warp_cursor_to_focus_centered() {
                        self.move_cursor_to_output(&output);
                    }
                }
            }
            Action::MoveWindowToMonitor(output) => {
                if let Some(output) = self.neori.output_by_name_match(&output).cloned() {
                    if self.neori.screenshot_ui.is_open() {
                        self.move_cursor_to_output(&output);
                        self.neori.screenshot_ui.move_to_output(output);
                    } else {
                        self.neori.layout.move_to_output(
                            None,
                            &output,
                            None,
                            ActivateWindow::Smart,
                        );
                        self.neori.layout.focus_output(&output);
                        if !self.maybe_warp_cursor_to_focus_centered() {
                            self.move_cursor_to_output(&output);
                        }
                    }
                }
            }
            Action::MoveWindowToMonitorById { id, output } => {
                if let Some(output) = self.neori.output_by_name_match(&output).cloned() {
                    let window = self
                        .neori
                        .layout
                        .windows()
                        .find(|(_, m)| m.id().get() == id);
                    let window = window.map(|(_, m)| m.window.clone());

                    if let Some(window) = window {
                        let target_was_active = self
                            .neori
                            .layout
                            .active_output()
                            .is_some_and(|active| output == *active);

                        self.neori.layout.move_to_output(
                            Some(&window),
                            &output,
                            None,
                            ActivateWindow::Smart,
                        );

                        // If the active output changed (window was moved and focused).
                        #[allow(clippy::collapsible_if)]
                        if !target_was_active && self.neori.layout.active_output() == Some(&output)
                        {
                            if !self.maybe_warp_cursor_to_focus_centered() {
                                self.move_cursor_to_output(&output);
                            }
                        }
                    }
                }
            }
            Action::MoveColumnToMonitorLeft => {
                if let Some(current_output) = self.neori.screenshot_ui.selection_output() {
                    if let Some(target_output) = self.neori.output_left_of(current_output) {
                        self.move_cursor_to_output(&target_output);
                        self.neori.screenshot_ui.move_to_output(target_output);
                    }
                } else if let Some(output) = self.neori.output_left() {
                    self.neori.layout.move_column_to_output(&output, None, true);
                    self.neori.layout.focus_output(&output);
                    if !self.maybe_warp_cursor_to_focus_centered() {
                        self.move_cursor_to_output(&output);
                    }
                }
            }
            Action::MoveColumnToMonitorRight => {
                if let Some(current_output) = self.neori.screenshot_ui.selection_output() {
                    if let Some(target_output) = self.neori.output_right_of(current_output) {
                        self.move_cursor_to_output(&target_output);
                        self.neori.screenshot_ui.move_to_output(target_output);
                    }
                } else if let Some(output) = self.neori.output_right() {
                    self.neori.layout.move_column_to_output(&output, None, true);
                    self.neori.layout.focus_output(&output);
                    if !self.maybe_warp_cursor_to_focus_centered() {
                        self.move_cursor_to_output(&output);
                    }
                }
            }
            Action::MoveColumnToMonitorDown => {
                if let Some(current_output) = self.neori.screenshot_ui.selection_output() {
                    if let Some(target_output) = self.neori.output_down_of(current_output) {
                        self.move_cursor_to_output(&target_output);
                        self.neori.screenshot_ui.move_to_output(target_output);
                    }
                } else if let Some(output) = self.neori.output_down() {
                    self.neori.layout.move_column_to_output(&output, None, true);
                    self.neori.layout.focus_output(&output);
                    if !self.maybe_warp_cursor_to_focus_centered() {
                        self.move_cursor_to_output(&output);
                    }
                }
            }
            Action::MoveColumnToMonitorUp => {
                if let Some(current_output) = self.neori.screenshot_ui.selection_output() {
                    if let Some(target_output) = self.neori.output_up_of(current_output) {
                        self.move_cursor_to_output(&target_output);
                        self.neori.screenshot_ui.move_to_output(target_output);
                    }
                } else if let Some(output) = self.neori.output_up() {
                    self.neori.layout.move_column_to_output(&output, None, true);
                    self.neori.layout.focus_output(&output);
                    if !self.maybe_warp_cursor_to_focus_centered() {
                        self.move_cursor_to_output(&output);
                    }
                }
            }
            Action::MoveColumnToMonitorPrevious => {
                if let Some(current_output) = self.neori.screenshot_ui.selection_output() {
                    if let Some(target_output) = self.neori.output_previous_of(current_output) {
                        self.move_cursor_to_output(&target_output);
                        self.neori.screenshot_ui.move_to_output(target_output);
                    }
                } else if let Some(output) = self.neori.output_previous() {
                    self.neori.layout.move_column_to_output(&output, None, true);
                    self.neori.layout.focus_output(&output);
                    if !self.maybe_warp_cursor_to_focus_centered() {
                        self.move_cursor_to_output(&output);
                    }
                }
            }
            Action::MoveColumnToMonitorNext => {
                if let Some(current_output) = self.neori.screenshot_ui.selection_output() {
                    if let Some(target_output) = self.neori.output_next_of(current_output) {
                        self.move_cursor_to_output(&target_output);
                        self.neori.screenshot_ui.move_to_output(target_output);
                    }
                } else if let Some(output) = self.neori.output_next() {
                    self.neori.layout.move_column_to_output(&output, None, true);
                    self.neori.layout.focus_output(&output);
                    if !self.maybe_warp_cursor_to_focus_centered() {
                        self.move_cursor_to_output(&output);
                    }
                }
            }
            Action::MoveColumnToMonitor(output) => {
                if let Some(output) = self.neori.output_by_name_match(&output).cloned() {
                    if self.neori.screenshot_ui.is_open() {
                        self.move_cursor_to_output(&output);
                        self.neori.screenshot_ui.move_to_output(output);
                    } else {
                        self.neori.layout.move_column_to_output(&output, None, true);
                        self.neori.layout.focus_output(&output);
                        if !self.maybe_warp_cursor_to_focus_centered() {
                            self.move_cursor_to_output(&output);
                        }
                    }
                }
            }
            Action::SetColumnWidth(change) => {
                if self.neori.screenshot_ui.is_open() {
                    self.neori.screenshot_ui.set_width(change);

                    // FIXME: granular
                    self.neori.queue_redraw_all();
                } else {
                    self.neori.layout.set_column_width(change);
                }
            }
            Action::SetWindowWidth(change) => {
                if self.neori.screenshot_ui.is_open() {
                    self.neori.screenshot_ui.set_width(change);

                    // FIXME: granular
                    self.neori.queue_redraw_all();
                } else {
                    self.neori.layout.set_window_width(None, change);
                }
            }
            Action::SetWindowWidthById { id, change } => {
                let window = self
                    .neori
                    .layout
                    .windows()
                    .find(|(_, m)| m.id().get() == id);
                let window = window.map(|(_, m)| m.window.clone());
                if let Some(window) = window {
                    self.neori.layout.set_window_width(Some(&window), change);
                }
            }
            Action::SetWindowHeight(change) => {
                if self.neori.screenshot_ui.is_open() {
                    self.neori.screenshot_ui.set_height(change);

                    // FIXME: granular
                    self.neori.queue_redraw_all();
                } else {
                    self.neori.layout.set_window_height(None, change);
                }
            }
            Action::SetWindowHeightById { id, change } => {
                let window = self
                    .neori
                    .layout
                    .windows()
                    .find(|(_, m)| m.id().get() == id);
                let window = window.map(|(_, m)| m.window.clone());
                if let Some(window) = window {
                    self.neori.layout.set_window_height(Some(&window), change);
                }
            }
            Action::ResetWindowHeight => {
                self.neori.layout.reset_window_height(None);
            }
            Action::ResetWindowHeightById(id) => {
                let window = self
                    .neori
                    .layout
                    .windows()
                    .find(|(_, m)| m.id().get() == id);
                let window = window.map(|(_, m)| m.window.clone());
                if let Some(window) = window {
                    self.neori.layout.reset_window_height(Some(&window));
                }
            }
            Action::ExpandColumnToAvailableWidth => {
                self.neori.layout.expand_column_to_available_width();
            }
            Action::ShowHotkeyOverlay => {
                if self.neori.hotkey_overlay.show() {
                    self.neori.queue_redraw_all();

                    #[cfg(feature = "dbus")]
                    self.neori.a11y_announce_hotkey_overlay();
                }
            }
            Action::MoveWorkspaceToMonitorLeft => {
                if let Some(output) = self.neori.output_left() {
                    self.neori.layout.move_workspace_to_output(&output);
                    if !self.maybe_warp_cursor_to_focus_centered() {
                        self.move_cursor_to_output(&output);
                    }
                }
            }
            Action::MoveWorkspaceToMonitorRight => {
                if let Some(output) = self.neori.output_right() {
                    self.neori.layout.move_workspace_to_output(&output);
                    if !self.maybe_warp_cursor_to_focus_centered() {
                        self.move_cursor_to_output(&output);
                    }
                }
            }
            Action::MoveWorkspaceToMonitorDown => {
                if let Some(output) = self.neori.output_down() {
                    self.neori.layout.move_workspace_to_output(&output);
                    if !self.maybe_warp_cursor_to_focus_centered() {
                        self.move_cursor_to_output(&output);
                    }
                }
            }
            Action::MoveWorkspaceToMonitorUp => {
                if let Some(output) = self.neori.output_up() {
                    self.neori.layout.move_workspace_to_output(&output);
                    if !self.maybe_warp_cursor_to_focus_centered() {
                        self.move_cursor_to_output(&output);
                    }
                }
            }
            Action::MoveWorkspaceToMonitorPrevious => {
                if let Some(output) = self.neori.output_previous() {
                    self.neori.layout.move_workspace_to_output(&output);
                    if !self.maybe_warp_cursor_to_focus_centered() {
                        self.move_cursor_to_output(&output);
                    }
                }
            }
            Action::MoveWorkspaceToMonitorNext => {
                if let Some(output) = self.neori.output_next() {
                    self.neori.layout.move_workspace_to_output(&output);
                    if !self.maybe_warp_cursor_to_focus_centered() {
                        self.move_cursor_to_output(&output);
                    }
                }
            }
            Action::MoveWorkspaceToMonitor(new_output) => {
                if let Some(new_output) = self.neori.output_by_name_match(&new_output).cloned() {
                    if self.neori.layout.move_workspace_to_output(&new_output)
                        && !self.maybe_warp_cursor_to_focus_centered()
                    {
                        self.move_cursor_to_output(&new_output);
                    }
                }
            }
            Action::MoveWorkspaceToMonitorByRef {
                output_name,
                reference,
            } => {
                if let Some((output, old_idx)) =
                    self.neori.find_output_and_workspace_index(reference)
                {
                    if let Some(new_output) = self.neori.output_by_name_match(&output_name).cloned()
                    {
                        if self.neori.layout.move_workspace_to_output_by_id(
                            old_idx,
                            output,
                            &new_output,
                        ) {
                            // Cursor warp already calls `queue_redraw_all`
                            if !self.maybe_warp_cursor_to_focus_centered() {
                                self.move_cursor_to_output(&new_output);
                            }
                        }
                    }
                }
            }
            Action::ToggleWindowFloating => {
                self.neori.layout.toggle_window_floating(None);
                // FIXME: granular
                self.neori.queue_redraw_all();
            }
            Action::ToggleWindowFloatingById(id) => {
                let window = self
                    .neori
                    .layout
                    .windows()
                    .find(|(_, m)| m.id().get() == id);
                let window = window.map(|(_, m)| m.window.clone());
                if let Some(window) = window {
                    self.neori.layout.toggle_window_floating(Some(&window));
                    // FIXME: granular
                    self.neori.queue_redraw_all();
                }
            }
            Action::MoveWindowToFloating => {
                self.neori.layout.set_window_floating(None, true);
                // FIXME: granular
                self.neori.queue_redraw_all();
            }
            Action::MoveWindowToFloatingById(id) => {
                let window = self
                    .neori
                    .layout
                    .windows()
                    .find(|(_, m)| m.id().get() == id);
                let window = window.map(|(_, m)| m.window.clone());
                if let Some(window) = window {
                    self.neori.layout.set_window_floating(Some(&window), true);
                    // FIXME: granular
                    self.neori.queue_redraw_all();
                }
            }
            Action::MoveWindowToTiling => {
                self.neori.layout.set_window_floating(None, false);
                // FIXME: granular
                self.neori.queue_redraw_all();
            }
            Action::MoveWindowToTilingById(id) => {
                let window = self
                    .neori
                    .layout
                    .windows()
                    .find(|(_, m)| m.id().get() == id);
                let window = window.map(|(_, m)| m.window.clone());
                if let Some(window) = window {
                    self.neori.layout.set_window_floating(Some(&window), false);
                    // FIXME: granular
                    self.neori.queue_redraw_all();
                }
            }
            Action::FocusFloating => {
                self.neori.layout.focus_floating();
                self.maybe_warp_cursor_to_focus();
                // FIXME: granular
                self.neori.queue_redraw_all();
            }
            Action::FocusTiling => {
                self.neori.layout.focus_tiling();
                self.maybe_warp_cursor_to_focus();
                // FIXME: granular
                self.neori.queue_redraw_all();
            }
            Action::SwitchFocusBetweenFloatingAndTiling => {
                self.neori.layout.switch_focus_floating_tiling();
                self.maybe_warp_cursor_to_focus();
                // FIXME: granular
                self.neori.queue_redraw_all();
            }
            Action::MoveFloatingWindowById { id, x, y } => {
                let window = if let Some(id) = id {
                    let window = self
                        .neori
                        .layout
                        .windows()
                        .find(|(_, m)| m.id().get() == id);
                    let window = window.map(|(_, m)| m.window.clone());
                    if window.is_none() {
                        return;
                    }
                    window
                } else {
                    None
                };

                self.neori
                    .layout
                    .move_floating_window(window.as_ref(), x, y, true);
                // FIXME: granular
                self.neori.queue_redraw_all();
            }
            Action::ToggleWindowRuleOpacity => {
                let active_window = self
                    .neori
                    .layout
                    .active_workspace_mut()
                    .and_then(|ws| ws.active_window_mut());
                if let Some(window) = active_window {
                    if window.rules().opacity.is_some_and(|o| o != 1.) {
                        window.toggle_ignore_opacity_window_rule();
                        // FIXME: granular
                        self.neori.queue_redraw_all();
                    }
                }
            }
            Action::ToggleWindowRuleOpacityById(id) => {
                let window = self
                    .neori
                    .layout
                    .workspaces_mut()
                    .find_map(|ws| ws.windows_mut().find(|w| w.id().get() == id));
                if let Some(window) = window {
                    if window.rules().opacity.is_some_and(|o| o != 1.) {
                        window.toggle_ignore_opacity_window_rule();
                        // FIXME: granular
                        self.neori.queue_redraw_all();
                    }
                }
            }
            Action::SetDynamicCastWindow => {
                let id = self
                    .neori
                    .layout
                    .active_workspace()
                    .and_then(|ws| ws.active_window())
                    .map(|mapped| mapped.id().get());
                if let Some(id) = id {
                    self.set_dynamic_cast_target(CastTarget::Window { id });
                }
            }
            Action::SetDynamicCastWindowById(id) => {
                let layout = &self.neori.layout;
                if layout.windows().any(|(_, mapped)| mapped.id().get() == id) {
                    self.set_dynamic_cast_target(CastTarget::Window { id });
                }
            }
            Action::SetDynamicCastMonitor(output) => {
                let output = match output {
                    None => self.neori.layout.active_output(),
                    Some(name) => self.neori.output_by_name_match(&name),
                };
                if let Some(output) = output {
                    self.set_dynamic_cast_target(CastTarget::output(output));
                }
            }
            Action::ClearDynamicCastTarget => {
                self.set_dynamic_cast_target(CastTarget::Nothing);
            }
            Action::StopCast(session_id) => {
                self.neori.stop_cast(CastSessionId::from(session_id));
            }
            Action::ToggleOverview => {
                self.neori.layout.toggle_overview();
                self.neori.queue_redraw_all();
            }
            Action::OpenOverview => {
                if self.neori.layout.open_overview() {
                    self.neori.queue_redraw_all();
                }
            }
            Action::CloseOverview => {
                if self.neori.layout.close_overview() {
                    self.neori.queue_redraw_all();
                }
            }
            Action::EnlargeCursor => {
                self.neori.enlarge_cursor();
            }
            Action::SendToScratchpad => {
                self.neori.layout.send_to_scratchpad();
                self.neori.queue_redraw_all();
            }
            Action::RemoveFromScratchpad => {
                self.neori.layout.remove_from_scratchpad();
                self.neori.queue_redraw_all();
            }
            Action::ToggleScratchpad => {
                self.neori.layout.toggle_scratchpad();
                self.neori.queue_redraw_all();
            }
            Action::ToggleWindowUrgent(id) => {
                let window = self
                    .neori
                    .layout
                    .workspaces_mut()
                    .find_map(|ws| ws.windows_mut().find(|w| w.id().get() == id));
                if let Some(window) = window {
                    let urgent = window.is_urgent();
                    window.set_urgent(!urgent);
                }
                self.neori.queue_redraw_all();
            }
            Action::SetWindowUrgent(id) => {
                let window = self
                    .neori
                    .layout
                    .workspaces_mut()
                    .find_map(|ws| ws.windows_mut().find(|w| w.id().get() == id));
                if let Some(window) = window {
                    window.set_urgent(true);
                }
                self.neori.queue_redraw_all();
            }
            Action::UnsetWindowUrgent(id) => {
                let window = self
                    .neori
                    .layout
                    .workspaces_mut()
                    .find_map(|ws| ws.windows_mut().find(|w| w.id().get() == id));
                if let Some(window) = window {
                    window.set_urgent(false);
                }
                self.neori.queue_redraw_all();
            }
            Action::LoadConfigFile(path) => {
                if let Some(watcher) = &self.neori.config_file_watcher {
                    watcher.load_config(path);
                }
            }
            Action::MruConfirm => {
                self.confirm_mru();
            }
            Action::MruCancel => {
                self.neori.cancel_mru();
            }
            Action::MruAdvance {
                direction,
                scope,
                filter,
            } => {
                if self.neori.window_mru_ui.is_open() {
                    self.neori.window_mru_ui.advance(direction, filter);
                    self.neori.queue_redraw_mru_output();
                } else if self.neori.config.borrow().recent_windows.on {
                    self.neori.mru_apply_keyboard_commit();

                    let config = self.neori.config.borrow();
                    let scope = scope.unwrap_or(self.neori.window_mru_ui.scope());

                    let mut wmru = WindowMru::new(&self.neori);
                    if !wmru.is_empty() {
                        wmru.set_scope(scope);
                        if let Some(filter) = filter {
                            wmru.set_filter(filter);
                        }

                        if let Some(output) = self.neori.layout.active_output() {
                            self.neori.window_mru_ui.open(
                                self.neori.clock.clone(),
                                wmru,
                                output.clone(),
                            );

                            // Only select the *next* window if some window (which should be the
                            // first one) is already focused. If nothing is focused, keep the first
                            // window (which is logically the "previously selected" one).
                            let keep_first = direction == MruDirection::Forward
                                && self.neori.layout.focus().is_none();
                            if !keep_first {
                                self.neori.window_mru_ui.advance(direction, None);
                            }

                            drop(config);
                            self.neori.queue_redraw_all();
                        }
                    }
                }
            }
            Action::MruCloseCurrentWindow => {
                if self.neori.window_mru_ui.is_open() {
                    if let Some(id) = self.neori.window_mru_ui.current_window_id() {
                        if let Some(w) = self.neori.find_window_by_id(id) {
                            if let Some(tl) = w.toplevel() {
                                tl.send_close();
                            }
                        }
                    }
                }
            }
            Action::MruFirst => {
                if self.neori.window_mru_ui.is_open() {
                    self.neori.window_mru_ui.first();
                    self.neori.queue_redraw_mru_output();
                }
            }
            Action::MruLast => {
                if self.neori.window_mru_ui.is_open() {
                    self.neori.window_mru_ui.last();
                    self.neori.queue_redraw_mru_output();
                }
            }
            Action::MruSetScope(scope) => {
                if self.neori.window_mru_ui.is_open() {
                    self.neori.window_mru_ui.set_scope(scope);
                    self.neori.queue_redraw_mru_output();
                }
            }
            Action::MruCycleScope => {
                if self.neori.window_mru_ui.is_open() {
                    self.neori.window_mru_ui.cycle_scope();
                    self.neori.queue_redraw_mru_output();
                }
            }
        }
    }

    fn on_pointer_motion<I: InputBackend>(&mut self, event: I::PointerMotionEvent) {
        let was_inside_hot_corner = self.neori.pointer_inside_hot_corner;
        // Any of the early returns here mean that the pointer is not inside the hot corner.
        self.neori.pointer_inside_hot_corner = false;

        // We need an output to be able to move the pointer.
        if self.neori.global_space.outputs().next().is_none() {
            return;
        }

        let serial = SERIAL_COUNTER.next_serial();

        let pointer = self.neori.seat.get_pointer().unwrap();

        let pos = pointer.current_location();

        // We have an output, so we can compute the new location and focus.
        let mut new_pos = pos + event.delta();

        // We received an event for the regular pointer, so show it now.
        self.neori.pointer_visibility = PointerVisibility::Visible;
        self.neori.tablet_cursor_location = None;

        // Detect the shake-to-find gesture.
        let delta = event.delta();
        let clock = self.neori.clock.clone();
        self.neori
            .cursor_shake
            .pointer_motion(event.time_msec(), delta.x, delta.y, &clock);

        // Check if we have an active pointer constraint.
        //
        // FIXME: ideally this should use the pointer focus with up-to-date global location.
        let mut pointer_confined = None;
        if let Some(under) = &self.neori.pointer_contents.surface {
            // No need to check if the pointer focus surface matches, because here we're checking
            // for an already-active constraint, and the constraint is deactivated when the focused
            // surface changes.
            let pos_within_surface = pos - under.1;

            let mut pointer_locked = false;
            with_pointer_constraint(&under.0, &pointer, |constraint| {
                let Some(constraint) = constraint else { return };
                if !constraint.is_active() {
                    return;
                }

                // Constraint does not apply if not within region.
                if let Some(region) = constraint.region() {
                    if !region.contains(pos_within_surface.to_i32_round()) {
                        return;
                    }
                }

                match &*constraint {
                    PointerConstraint::Locked(_locked) => {
                        pointer_locked = true;
                    }
                    PointerConstraint::Confined(confine) => {
                        pointer_confined = Some((under.clone(), confine.region().cloned()));
                    }
                }
            });

            // If the pointer is locked, only send relative motion.
            if pointer_locked {
                pointer.relative_motion(
                    self,
                    Some(under.clone()),
                    &RelativeMotionEvent {
                        delta: event.delta(),
                        delta_unaccel: event.delta_unaccel(),
                        utime: event.time(),
                    },
                );

                pointer.frame(self);

                // I guess a redraw to hide the tablet cursor could be nice? Doesn't matter too
                // much here I think.
                return;
            }
        }

        // Warp pointer across the screen during the spatial movement grabs.
        let spatial_grab = pointer.with_grab(|_, grab| {
            let grab = grab.as_any();
            if let Some(grab) = grab.downcast_ref::<SpatialMovementGrab>() {
                if let Some(output) = grab.view_offset_output() {
                    return Some((output.clone(), true));
                } else if let Some(output) = grab.workspace_switch_output() {
                    return Some((output.clone(), false));
                }
            } else if let Some(grab) = grab.downcast_ref::<MoveGrab>() {
                if let Some(output) = grab.view_offset_output() {
                    return Some((output.clone(), true));
                }
            }
            None
        });
        if let Some((output, horizontal)) = spatial_grab.flatten() {
            if let Some(geo) = self.neori.global_space.output_geometry(&output) {
                let geo = geo.to_f64();
                if horizontal {
                    new_pos.x = (new_pos.x - geo.loc.x).rem_euclid(geo.size.w) + geo.loc.x;
                    new_pos.y = new_pos.y.clamp(geo.loc.y, geo.loc.y + geo.size.h - 1.);
                } else {
                    new_pos.x = new_pos.x.clamp(geo.loc.x, geo.loc.x + geo.size.w - 1.);
                    new_pos.y = (new_pos.y - geo.loc.y).rem_euclid(geo.size.h) + geo.loc.y;
                }
            }
        }

        if self
            .neori
            .global_space
            .output_under(new_pos)
            .next()
            .is_none()
        {
            // We ended up outside the outputs and need to clip the movement.
            if let Some(output) = self.neori.global_space.output_under(pos).next() {
                // The pointer was previously on some output. Clip the movement against its
                // boundaries.
                let geom = self.neori.global_space.output_geometry(output).unwrap();
                new_pos.x = new_pos
                    .x
                    .clamp(geom.loc.x as f64, (geom.loc.x + geom.size.w - 1) as f64);
                new_pos.y = new_pos
                    .y
                    .clamp(geom.loc.y as f64, (geom.loc.y + geom.size.h - 1) as f64);
            } else {
                // The pointer was not on any output in the first place. Find one for it.
                // Let's do the simple thing and just put it on the first output.
                let output = self.neori.global_space.outputs().next().unwrap();
                let geom = self.neori.global_space.output_geometry(output).unwrap();
                new_pos = center(geom).to_f64();
            }
        }

        if let Some(output) = self.neori.screenshot_ui.selection_output() {
            let geom = self.neori.global_space.output_geometry(output).unwrap();
            let point = (new_pos - geom.loc.to_f64())
                .to_physical(output.current_scale().fractional_scale())
                .to_i32_round::<i32>();

            self.neori.screenshot_ui.pointer_motion(point, None);
        }

        if let Some(mru_output) = self.neori.window_mru_ui.output() {
            if let Some((output, pos_within_output)) = self.neori.output_under(new_pos) {
                if mru_output == output {
                    self.neori.window_mru_ui.pointer_motion(pos_within_output);
                }
            }
        }

        let under = self.neori.contents_under(new_pos);

        // Handle confined pointer.
        if let Some((focus_surface, region)) = pointer_confined {
            let mut prevent = false;

            // Prevent the pointer from leaving the focused surface.
            if Some(&focus_surface.0) != under.surface.as_ref().map(|(s, _)| s) {
                prevent = true;
            }

            // Prevent the pointer from leaving the confine region, if any.
            if let Some(region) = region {
                let new_pos_within_surface = new_pos - focus_surface.1;
                if !region.contains(new_pos_within_surface.to_i32_round()) {
                    prevent = true;
                }
            }

            if prevent {
                pointer.relative_motion(
                    self,
                    Some(focus_surface),
                    &RelativeMotionEvent {
                        delta: event.delta(),
                        delta_unaccel: event.delta_unaccel(),
                        utime: event.time(),
                    },
                );

                pointer.frame(self);

                return;
            }
        }

        self.neori.handle_focus_follows_mouse(&under);

        self.neori.pointer_contents.clone_from(&under);

        pointer.motion(
            self,
            under.surface.clone(),
            &MotionEvent {
                location: new_pos,
                serial,
                time: event.time_msec(),
            },
        );

        pointer.relative_motion(
            self,
            under.surface,
            &RelativeMotionEvent {
                delta: event.delta(),
                delta_unaccel: event.delta_unaccel(),
                utime: event.time(),
            },
        );

        pointer.frame(self);

        // contents_under() will return no surface when the hot corner should trigger, so
        // pointer.motion() will set the current focus to None.
        if under.hot_corner && pointer.current_focus().is_none() {
            if !was_inside_hot_corner
                && pointer
                    .with_grab(|_, grab| grab_allows_hot_corner(grab))
                    .unwrap_or(true)
            {
                self.neori.layout.toggle_overview();
            }
            self.neori.pointer_inside_hot_corner = true;
        }

        // Activate a new confinement if necessary.
        self.neori.maybe_activate_pointer_constraint();

        // Inform the layout of an ongoing DnD operation.
        let is_dnd_grab = pointer
            .with_grab(|_, grab| Self::is_dnd_grab(grab.as_any()))
            .unwrap_or(false);
        if is_dnd_grab {
            if let Some((output, pos_within_output)) = self.neori.output_under(new_pos) {
                let output = output.clone();
                self.neori.layout.dnd_update(output, pos_within_output);
            }
        }

        // Notify a11y.
        #[cfg(feature = "dbus")]
        self.a11y_notify_pointer_motion();

        // Redraw to update the cursor position.
        // FIXME: redraw only outputs overlapping the cursor.
        self.neori.queue_redraw_all();
    }

    fn on_pointer_motion_absolute<I: InputBackend>(
        &mut self,
        event: I::PointerMotionAbsoluteEvent,
    ) {
        let was_inside_hot_corner = self.neori.pointer_inside_hot_corner;
        // Any of the early returns here mean that the pointer is not inside the hot corner.
        self.neori.pointer_inside_hot_corner = false;

        let Some(pos) = self.compute_absolute_location(&event, None).or_else(|| {
            self.global_bounding_rectangle().map(|output_geo| {
                event.position_transformed(output_geo.size) + output_geo.loc.to_f64()
            })
        }) else {
            return;
        };

        let serial = SERIAL_COUNTER.next_serial();

        let pointer = self.neori.seat.get_pointer().unwrap();

        if let Some(output) = self.neori.screenshot_ui.selection_output() {
            let geom = self.neori.global_space.output_geometry(output).unwrap();
            let point = (pos - geom.loc.to_f64())
                .to_physical(output.current_scale().fractional_scale())
                .to_i32_round::<i32>();

            self.neori.screenshot_ui.pointer_motion(point, None);
        }

        if let Some(mru_output) = self.neori.window_mru_ui.output() {
            if let Some((output, pos_within_output)) = self.neori.output_under(pos) {
                if mru_output == output {
                    self.neori.window_mru_ui.pointer_motion(pos_within_output);
                }
            }
        }

        let under = self.neori.contents_under(pos);

        self.neori.handle_focus_follows_mouse(&under);

        self.neori.pointer_contents.clone_from(&under);

        pointer.motion(
            self,
            under.surface,
            &MotionEvent {
                location: pos,
                serial,
                time: event.time_msec(),
            },
        );

        pointer.frame(self);

        // contents_under() will return no surface when the hot corner should trigger, so
        // pointer.motion() will set the current focus to None.
        if under.hot_corner && pointer.current_focus().is_none() {
            if !was_inside_hot_corner
                && pointer
                    .with_grab(|_, grab| grab_allows_hot_corner(grab))
                    .unwrap_or(true)
            {
                self.neori.layout.toggle_overview();
            }
            self.neori.pointer_inside_hot_corner = true;
        }

        self.neori.maybe_activate_pointer_constraint();

        // We moved the pointer, show it.
        self.neori.pointer_visibility = PointerVisibility::Visible;

        // We moved the regular pointer, so show it now.
        self.neori.tablet_cursor_location = None;

        // Inform the layout of an ongoing DnD operation.
        let is_dnd_grab = pointer
            .with_grab(|_, grab| Self::is_dnd_grab(grab.as_any()))
            .unwrap_or(false);
        if is_dnd_grab {
            if let Some((output, pos_within_output)) = self.neori.output_under(pos) {
                let output = output.clone();
                self.neori.layout.dnd_update(output, pos_within_output);
            }
        }

        // Notify a11y.
        #[cfg(feature = "dbus")]
        self.a11y_notify_pointer_motion();

        // Redraw to update the cursor position.
        // FIXME: redraw only outputs overlapping the cursor.
        self.neori.queue_redraw_all();
    }

    fn on_pointer_button<I: InputBackend>(&mut self, event: I::PointerButtonEvent) {
        let pointer = self.neori.seat.get_pointer().unwrap();

        let serial = SERIAL_COUNTER.next_serial();

        let button = event.button();

        let button_code = event.button_code();

        let button_state = event.state();

        let mod_key = self.backend.mod_key(&self.neori.config.borrow());

        // Ignore release events for mouse clicks that triggered a bind.
        if self.neori.suppressed_buttons.remove(&button_code) {
            return;
        }

        let mods = self.neori.seat.get_keyboard().unwrap().modifier_state();
        let modifiers = modifiers_from_state(mods);
        let mod_down = modifiers.contains(mod_key.to_modifiers());

        if ButtonState::Pressed == button_state {
            let mut is_mru_open = false;
            if let Some(mru_output) = self.neori.window_mru_ui.output() {
                is_mru_open = true;
                if let Some(MouseButton::Left) = button {
                    let location = pointer.current_location();
                    let (output, pos_within_output) = self.neori.output_under(location).unwrap();
                    if mru_output == output {
                        let id = self.neori.window_mru_ui.pointer_motion(pos_within_output);
                        if id.is_some() {
                            self.confirm_mru();
                        } else {
                            self.neori.cancel_mru();
                        }
                    } else {
                        self.neori.cancel_mru();
                    }

                    self.neori.suppressed_buttons.insert(button_code);
                    return;
                }
            }

            if is_mru_open || self.neori.mods_with_mouse_binds.contains(&modifiers) {
                if let Some(bind) = match button {
                    Some(MouseButton::Left) => Some(Trigger::MouseLeft),
                    Some(MouseButton::Right) => Some(Trigger::MouseRight),
                    Some(MouseButton::Middle) => Some(Trigger::MouseMiddle),
                    Some(MouseButton::Back) => Some(Trigger::MouseBack),
                    Some(MouseButton::Forward) => Some(Trigger::MouseForward),
                    _ => None,
                }
                .and_then(|trigger| {
                    let config = self.neori.config.borrow();
                    let bindings =
                        make_binds_iter(&config, &mut self.neori.window_mru_ui, modifiers);
                    find_configured_bind(bindings, mod_key, trigger, mods)
                })
                .filter(|bind| {
                    !self.neori.screenshot_ui.is_open() || allowed_during_screenshot(&bind.action)
                }) {
                    self.neori.suppressed_buttons.insert(button_code);
                    self.handle_bind(bind.clone());
                    return;
                };
            }

            // We received an event for the regular pointer, so show it now.
            self.neori.pointer_visibility = PointerVisibility::Visible;
            self.neori.tablet_cursor_location = None;

            let is_overview_open = self.neori.layout.is_overview_open();

            if is_overview_open && !pointer.is_grabbed() && button == Some(MouseButton::Right) {
                if let Some((output, ws)) = self.neori.workspace_under_cursor(true) {
                    let ws_id = ws.id();
                    let ws_idx = self.neori.layout.find_workspace_by_id(ws_id).unwrap().0;

                    self.neori.layout.focus_output(&output);

                    let location = pointer.current_location();
                    let start_data = PointerGrabStartData {
                        focus: None,
                        button: button_code,
                        location,
                    };
                    self.neori
                        .layout
                        .view_offset_gesture_begin(&output, Some(ws_idx), false);
                    let grab = SpatialMovementGrab::new(start_data, output, ws_id, true);
                    pointer.set_grab(self, grab, serial, Focus::Clear);
                    self.neori
                        .cursor_manager
                        .set_cursor_image(CursorImageStatus::Named(CursorIcon::AllScroll));

                    // FIXME: granular.
                    self.neori.queue_redraw_all();
                    return;
                }
            }

            if button == Some(MouseButton::Middle) && !pointer.is_grabbed() && mod_down {
                let output_ws = if is_overview_open {
                    self.neori.workspace_under_cursor(true)
                } else {
                    // We don't want to accidentally "catch" the wrong workspace during
                    // animations.
                    self.neori.output_under_cursor().and_then(|output| {
                        let mon = self.neori.layout.monitor_for_output(&output)?;
                        Some((output, mon.active_workspace_ref()))
                    })
                };

                if let Some((output, ws)) = output_ws {
                    let ws_id = ws.id();

                    self.neori.layout.focus_output(&output);

                    let location = pointer.current_location();
                    let start_data = PointerGrabStartData {
                        focus: None,
                        button: button_code,
                        location,
                    };
                    let grab = SpatialMovementGrab::new(start_data, output, ws_id, false);
                    pointer.set_grab(self, grab, serial, Focus::Clear);
                    self.neori
                        .cursor_manager
                        .set_cursor_image(CursorImageStatus::Named(CursorIcon::AllScroll));

                    // FIXME: granular.
                    self.neori.queue_redraw_all();

                    // Don't activate the window under the cursor to avoid unnecessary
                    // scrolling when e.g. Mod+MMB clicking on a partially off-screen window.
                    return;
                }
            }

            if let Some(mapped) = self.neori.window_under_cursor() {
                let window = mapped.window.clone();

                // Check if we need to start an interactive move.
                if button == Some(MouseButton::Left) && !pointer.is_grabbed() {
                    if is_overview_open || mod_down {
                        let location = pointer.current_location();

                        if !is_overview_open {
                            self.neori.layout.activate_window(&window);
                        }

                        let start_data = PointerGrabStartData {
                            focus: None,
                            button: button_code,
                            location,
                        };
                        let start_data = PointerOrTouchStartData::Pointer(start_data);
                        let icon = CursorIcon::Grabbing;
                        if let Some(grab) =
                            MoveGrab::new(self, start_data, window.clone(), false, Some(icon))
                        {
                            pointer.set_grab(self, grab, serial, Focus::Clear);

                            // Set the cursor to Grabbing right away for Mod+LMB since it doesn't
                            // do any other gesture.
                            //
                            // In the overview, we click to activate window and close the overview,
                            // in this case setting the cursor right away would be distracting.
                            if !is_overview_open {
                                self.neori
                                    .cursor_manager
                                    .set_cursor_image(CursorImageStatus::Named(icon));
                            }
                        }
                    }
                }
                // Check if we need to start an interactive resize.
                else if button == Some(MouseButton::Right) && !pointer.is_grabbed() && mod_down {
                    let location = pointer.current_location();
                    let (output, pos_within_output) = self.neori.output_under(location).unwrap();
                    let edges = self
                        .neori
                        .layout
                        .resize_edges_under(output, pos_within_output)
                        .unwrap_or(ResizeEdge::empty());

                    if !edges.is_empty() {
                        // See if we got a double resize-click gesture.
                        // FIXME: deduplicate with resize_request in xdg-shell somehow.
                        let time = get_monotonic_time();
                        let last_cell = mapped.last_interactive_resize_start();
                        let mut last = last_cell.get();
                        last_cell.set(Some((time, edges)));

                        // Floating windows don't have either of the double-resize-click
                        // gestures, so just allow it to resize.
                        if mapped.is_floating() {
                            last = None;
                            last_cell.set(None);
                        }

                        if let Some((last_time, last_edges)) = last {
                            if time.saturating_sub(last_time) <= DOUBLE_CLICK_TIME {
                                // Allow quick resize after a triple click.
                                last_cell.set(None);

                                let intersection = edges.intersection(last_edges);
                                if intersection.intersects(ResizeEdge::LEFT_RIGHT) {
                                    // FIXME: don't activate once we can pass specific windows
                                    // to actions.
                                    self.neori.layout.activate_window(&window);
                                    self.neori.layout.toggle_full_width();
                                }
                                if intersection.intersects(ResizeEdge::TOP_BOTTOM) {
                                    self.neori.layout.activate_window(&window);
                                    self.neori.layout.reset_window_height(Some(&window));
                                }
                                // FIXME: granular.
                                self.neori.queue_redraw_all();
                                return;
                            }
                        }

                        self.neori.layout.activate_window(&window);

                        if self
                            .neori
                            .layout
                            .interactive_resize_begin(window.clone(), edges)
                        {
                            let start_data = PointerGrabStartData {
                                focus: None,
                                button: button_code,
                                location,
                            };
                            let grab = ResizeGrab::new(start_data, window.clone());
                            pointer.set_grab(self, grab, serial, Focus::Clear);
                            self.neori
                                .cursor_manager
                                .set_cursor_image(CursorImageStatus::Named(edges.cursor_icon()));
                        }
                    }
                }

                if !is_overview_open {
                    self.neori.layout.activate_window(&window);
                }

                // FIXME: granular.
                self.neori.queue_redraw_all();
            } else if let Some((output, ws)) = is_overview_open
                .then(|| self.neori.workspace_under_cursor(false))
                .flatten()
            {
                let ws_idx = self.neori.layout.find_workspace_by_id(ws.id()).unwrap().0;

                self.neori.layout.focus_output(&output);
                self.neori.layout.toggle_overview_to_workspace(ws_idx);

                // FIXME: granular.
                self.neori.queue_redraw_all();
            } else if let Some(output) = self.neori.output_under_cursor() {
                self.neori.layout.focus_output(&output);

                // FIXME: granular.
                self.neori.queue_redraw_all();
            }
        };

        self.update_pointer_contents();

        if ButtonState::Pressed == button_state {
            let layer_under = self.neori.pointer_contents.layer.clone();
            self.neori.focus_layer_surface_if_on_demand(layer_under);
        }

        if button == Some(MouseButton::Left) && self.neori.screenshot_ui.is_open() {
            if button_state == ButtonState::Pressed {
                let pos = pointer.current_location();

                // If we'll be moving the existing selection, use the selection output.
                let output = if mod_down {
                    self.neori.screenshot_ui.selection_output()
                } else {
                    self.neori.output_under(pos).map(|(out, _)| out)
                };

                if let Some(output) = output.cloned() {
                    let geom = self.neori.global_space.output_geometry(&output).unwrap();
                    let point = (pos - geom.loc.to_f64())
                        .to_physical(output.current_scale().fractional_scale())
                        .to_i32_round();

                    if self
                        .neori
                        .screenshot_ui
                        .pointer_down(output, point, None, mod_down)
                    {
                        self.neori.queue_redraw_all();
                    }
                }
            } else if let Some(capture) = self.neori.screenshot_ui.pointer_up(None) {
                if capture {
                    self.confirm_screenshot(true);
                } else {
                    self.neori.queue_redraw_all();
                }
            }
        }

        pointer.button(
            self,
            &ButtonEvent {
                button: button_code,
                state: button_state,
                serial,
                time: event.time_msec(),
            },
        );
        pointer.frame(self);
    }

    fn on_pointer_axis<I: InputBackend>(&mut self, event: I::PointerAxisEvent) {
        let pointer = &self.neori.seat.get_pointer().unwrap();

        let source = event.source();

        let mod_key = self.backend.mod_key(&self.neori.config.borrow());

        // We received an event for the regular pointer, so show it now. This is also needed for
        // update_pointer_contents() below to return the real contents, necessary for the pointer
        // axis event to reach the window.
        self.neori.pointer_visibility = PointerVisibility::Visible;
        self.neori.tablet_cursor_location = None;

        let timestamp = Duration::from_micros(event.time());

        let horizontal_amount_v120 = event.amount_v120(Axis::Horizontal);
        let vertical_amount_v120 = event.amount_v120(Axis::Vertical);

        let is_overview_open = self.neori.layout.is_overview_open();

        // We should only handle scrolling in the overview if the pointer is not over a (top or
        // overlay) layer surface.
        let should_handle_in_overview = if is_overview_open {
            // FIXME: ideally this should happen after updating the pointer contents, which happens
            // below. However, our pointer actions are supposed to act on the old surface, before
            // updating the pointer contents.
            pointer
                .current_focus()
                .map(|surface| self.neori.find_root_shell_surface(&surface))
                .is_none_or(|root| {
                    !self
                        .neori
                        .mapped_layer_surfaces
                        .keys()
                        .any(|layer| *layer.wl_surface() == root)
                })
        } else {
            false
        };

        let is_mru_open = self.neori.window_mru_ui.is_open();

        // Handle wheel scroll bindings.
        if source == AxisSource::Wheel {
            // If we have a scroll bind with current modifiers, then accumulate and don't pass to
            // Wayland. If there's no bind, reset the accumulator.
            let mods = self.neori.seat.get_keyboard().unwrap().modifier_state();
            let modifiers = modifiers_from_state(mods);
            let should_handle = should_handle_in_overview
                || is_mru_open
                || self.neori.mods_with_wheel_binds.contains(&modifiers);
            if should_handle {
                let horizontal = horizontal_amount_v120.unwrap_or(0.);
                let ticks = self.neori.horizontal_wheel_tracker.accumulate(horizontal);
                if ticks != 0 {
                    let (bind_left, bind_right) =
                        if should_handle_in_overview && modifiers.is_empty() {
                            let bind_left = Some(Bind {
                                key: Key {
                                    trigger: Trigger::WheelScrollLeft,
                                    modifiers: Modifiers::empty(),
                                },
                                action: Action::FocusColumnLeftUnderMouse,
                                repeat: true,
                                cooldown: None,
                                allow_when_locked: false,
                                allow_inhibiting: false,
                                hotkey_overlay_title: None,
                            });
                            let bind_right = Some(Bind {
                                key: Key {
                                    trigger: Trigger::WheelScrollRight,
                                    modifiers: Modifiers::empty(),
                                },
                                action: Action::FocusColumnRightUnderMouse,
                                repeat: true,
                                cooldown: None,
                                allow_when_locked: false,
                                allow_inhibiting: false,
                                hotkey_overlay_title: None,
                            });
                            (bind_left, bind_right)
                        } else {
                            let config = self.neori.config.borrow();
                            let bindings =
                                make_binds_iter(&config, &mut self.neori.window_mru_ui, modifiers);
                            let bind_left = find_configured_bind(
                                bindings.clone(),
                                mod_key,
                                Trigger::WheelScrollLeft,
                                mods,
                            )
                            .filter(|bind| {
                                !self.neori.screenshot_ui.is_open()
                                    || allowed_during_screenshot(&bind.action)
                            });
                            let bind_right = find_configured_bind(
                                bindings,
                                mod_key,
                                Trigger::WheelScrollRight,
                                mods,
                            )
                            .filter(|bind| {
                                !self.neori.screenshot_ui.is_open()
                                    || allowed_during_screenshot(&bind.action)
                            });
                            (bind_left, bind_right)
                        };

                    if let Some(right) = bind_right {
                        for _ in 0..ticks {
                            self.handle_bind(right.clone());
                        }
                    }
                    if let Some(left) = bind_left {
                        for _ in ticks..0 {
                            self.handle_bind(left.clone());
                        }
                    }
                }

                let vertical = vertical_amount_v120.unwrap_or(0.);
                let ticks = self.neori.vertical_wheel_tracker.accumulate(vertical);
                if ticks != 0 {
                    let (bind_up, bind_down) = if should_handle_in_overview && modifiers.is_empty()
                    {
                        let bind_up = Some(Bind {
                            key: Key {
                                trigger: Trigger::WheelScrollUp,
                                modifiers: Modifiers::empty(),
                            },
                            action: Action::FocusWorkspaceUpUnderMouse,
                            repeat: true,
                            cooldown: Some(Duration::from_millis(50)),
                            allow_when_locked: false,
                            allow_inhibiting: false,
                            hotkey_overlay_title: None,
                        });
                        let bind_down = Some(Bind {
                            key: Key {
                                trigger: Trigger::WheelScrollDown,
                                modifiers: Modifiers::empty(),
                            },
                            action: Action::FocusWorkspaceDownUnderMouse,
                            repeat: true,
                            cooldown: Some(Duration::from_millis(50)),
                            allow_when_locked: false,
                            allow_inhibiting: false,
                            hotkey_overlay_title: None,
                        });
                        (bind_up, bind_down)
                    } else if should_handle_in_overview && modifiers == Modifiers::SHIFT {
                        let bind_up = Some(Bind {
                            key: Key {
                                trigger: Trigger::WheelScrollUp,
                                modifiers: Modifiers::empty(),
                            },
                            action: Action::FocusColumnLeftUnderMouse,
                            repeat: true,
                            cooldown: Some(Duration::from_millis(50)),
                            allow_when_locked: false,
                            allow_inhibiting: false,
                            hotkey_overlay_title: None,
                        });
                        let bind_down = Some(Bind {
                            key: Key {
                                trigger: Trigger::WheelScrollDown,
                                modifiers: Modifiers::empty(),
                            },
                            action: Action::FocusColumnRightUnderMouse,
                            repeat: true,
                            cooldown: Some(Duration::from_millis(50)),
                            allow_when_locked: false,
                            allow_inhibiting: false,
                            hotkey_overlay_title: None,
                        });
                        (bind_up, bind_down)
                    } else {
                        let config = self.neori.config.borrow();
                        let bindings =
                            make_binds_iter(&config, &mut self.neori.window_mru_ui, modifiers);
                        let bind_up = find_configured_bind(
                            bindings.clone(),
                            mod_key,
                            Trigger::WheelScrollUp,
                            mods,
                        )
                        .filter(|bind| {
                            !self.neori.screenshot_ui.is_open()
                                || allowed_during_screenshot(&bind.action)
                        });
                        let bind_down =
                            find_configured_bind(bindings, mod_key, Trigger::WheelScrollDown, mods)
                                .filter(|bind| {
                                    !self.neori.screenshot_ui.is_open()
                                        || allowed_during_screenshot(&bind.action)
                                });
                        (bind_up, bind_down)
                    };

                    if let Some(down) = bind_down {
                        for _ in 0..ticks {
                            self.handle_bind(down.clone());
                        }
                    }
                    if let Some(up) = bind_up {
                        for _ in ticks..0 {
                            self.handle_bind(up.clone());
                        }
                    }
                }

                return;
            } else {
                self.neori.horizontal_wheel_tracker.reset();
                self.neori.vertical_wheel_tracker.reset();
            }
        }

        let horizontal_amount = event.amount(Axis::Horizontal);
        let vertical_amount = event.amount(Axis::Vertical);

        // Handle touchpad and continuous scroll bindings.
        if source == AxisSource::Finger || source == AxisSource::Continuous {
            let mods = self.neori.seat.get_keyboard().unwrap().modifier_state();
            let modifiers = modifiers_from_state(mods);

            let horizontal = horizontal_amount.unwrap_or(0.);
            let vertical = vertical_amount.unwrap_or(0.);

            if should_handle_in_overview && modifiers.is_empty() {
                let mut redraw = false;

                let action = self
                    .neori
                    .overview_scroll_swipe_gesture
                    .update(horizontal, vertical);
                let is_vertical = self.neori.overview_scroll_swipe_gesture.is_vertical();

                if action.end() {
                    if is_vertical {
                        redraw |= self
                            .neori
                            .layout
                            .workspace_switch_gesture_end(Some(true))
                            .is_some();
                    } else {
                        redraw |= self
                            .neori
                            .layout
                            .view_offset_gesture_end(Some(true))
                            .is_some();
                    }
                } else {
                    // Maybe begin, then update.
                    if is_vertical {
                        if action.begin() {
                            if let Some(output) = self.neori.output_under_cursor() {
                                self.neori
                                    .layout
                                    .workspace_switch_gesture_begin(&output, true);
                                redraw = true;
                            }
                        }

                        let res = self
                            .neori
                            .layout
                            .workspace_switch_gesture_update(vertical, timestamp, true);
                        if let Some(Some(_)) = res {
                            redraw = true;
                        }
                    } else {
                        if action.begin() {
                            if let Some((output, ws)) = self.neori.workspace_under_cursor(true) {
                                let ws_id = ws.id();
                                let ws_idx =
                                    self.neori.layout.find_workspace_by_id(ws_id).unwrap().0;

                                self.neori.layout.view_offset_gesture_begin(
                                    &output,
                                    Some(ws_idx),
                                    true,
                                );
                                redraw = true;
                            }
                        }

                        let res = self
                            .neori
                            .layout
                            .view_offset_gesture_update(horizontal, timestamp, true);
                        if let Some(Some(_)) = res {
                            redraw = true;
                        }
                    }
                }

                if redraw {
                    self.neori.queue_redraw_all();
                }

                return;
            } else {
                let mut redraw = false;
                if self.neori.overview_scroll_swipe_gesture.reset() {
                    if self.neori.overview_scroll_swipe_gesture.is_vertical() {
                        redraw |= self
                            .neori
                            .layout
                            .workspace_switch_gesture_end(Some(true))
                            .is_some();
                    } else {
                        redraw |= self
                            .neori
                            .layout
                            .view_offset_gesture_end(Some(true))
                            .is_some();
                    }
                }
                if redraw {
                    self.neori.queue_redraw_all();
                }
            }

            if is_mru_open
                || self
                    .neori
                    .mods_with_finger_scroll_binds
                    .contains(&modifiers)
            {
                let ticks = self
                    .neori
                    .horizontal_finger_scroll_tracker
                    .accumulate(horizontal);
                if ticks != 0 {
                    let config = self.neori.config.borrow();
                    let bindings =
                        make_binds_iter(&config, &mut self.neori.window_mru_ui, modifiers);
                    let bind_left = find_configured_bind(
                        bindings.clone(),
                        mod_key,
                        Trigger::TouchpadScrollLeft,
                        mods,
                    )
                    .filter(|bind| {
                        !self.neori.screenshot_ui.is_open()
                            || allowed_during_screenshot(&bind.action)
                    });
                    let bind_right =
                        find_configured_bind(bindings, mod_key, Trigger::TouchpadScrollRight, mods)
                            .filter(|bind| {
                                !self.neori.screenshot_ui.is_open()
                                    || allowed_during_screenshot(&bind.action)
                            });
                    drop(config);

                    if let Some(right) = bind_right {
                        for _ in 0..ticks {
                            self.handle_bind(right.clone());
                        }
                    }
                    if let Some(left) = bind_left {
                        for _ in ticks..0 {
                            self.handle_bind(left.clone());
                        }
                    }
                }

                let ticks = self
                    .neori
                    .vertical_finger_scroll_tracker
                    .accumulate(vertical);
                if ticks != 0 {
                    let config = self.neori.config.borrow();
                    let bindings =
                        make_binds_iter(&config, &mut self.neori.window_mru_ui, modifiers);
                    let bind_up = find_configured_bind(
                        bindings.clone(),
                        mod_key,
                        Trigger::TouchpadScrollUp,
                        mods,
                    )
                    .filter(|bind| {
                        !self.neori.screenshot_ui.is_open()
                            || allowed_during_screenshot(&bind.action)
                    });
                    let bind_down =
                        find_configured_bind(bindings, mod_key, Trigger::TouchpadScrollDown, mods)
                            .filter(|bind| {
                                !self.neori.screenshot_ui.is_open()
                                    || allowed_during_screenshot(&bind.action)
                            });
                    drop(config);

                    if let Some(down) = bind_down {
                        for _ in 0..ticks {
                            self.handle_bind(down.clone());
                        }
                    }
                    if let Some(up) = bind_up {
                        for _ in ticks..0 {
                            self.handle_bind(up.clone());
                        }
                    }
                }

                return;
            } else {
                self.neori.horizontal_finger_scroll_tracker.reset();
                self.neori.vertical_finger_scroll_tracker.reset();
            }
        }

        self.update_pointer_contents();

        let device_scroll_factor = {
            let config = self.neori.config.borrow();
            match source {
                AxisSource::Wheel => config.input.mouse.scroll_factor,
                AxisSource::Finger => config.input.touchpad.scroll_factor,
                _ => None,
            }
        };

        // Get window-specific scroll factor
        let window_scroll_factor = pointer
            .current_focus()
            .map(|focused| self.neori.find_root_shell_surface(&focused))
            .and_then(|root| self.neori.layout.find_window_and_output(&root).unzip().0)
            .and_then(|window| window.rules().scroll_factor)
            .unwrap_or(1.);

        // Determine final scroll factors based on configuration
        let (horizontal_factor, vertical_factor) = device_scroll_factor
            .map(|x| x.h_v_factors())
            .unwrap_or((1.0, 1.0));
        let (horizontal_factor, vertical_factor) = (
            horizontal_factor * window_scroll_factor,
            vertical_factor * window_scroll_factor,
        );

        let horizontal_amount = horizontal_amount.unwrap_or_else(|| {
            // Winit backend, discrete scrolling.
            horizontal_amount_v120.unwrap_or(0.0) / 120. * 15.
        }) * horizontal_factor;

        let vertical_amount = vertical_amount.unwrap_or_else(|| {
            // Winit backend, discrete scrolling.
            vertical_amount_v120.unwrap_or(0.0) / 120. * 15.
        }) * vertical_factor;

        let horizontal_amount_v120 = horizontal_amount_v120.map(|x| x * horizontal_factor);
        let vertical_amount_v120 = vertical_amount_v120.map(|x| x * vertical_factor);

        let mut frame = AxisFrame::new(event.time_msec()).source(source);
        if horizontal_amount != 0.0 {
            frame = frame
                .relative_direction(Axis::Horizontal, event.relative_direction(Axis::Horizontal));
            frame = frame.value(Axis::Horizontal, horizontal_amount);
            if let Some(v120) = horizontal_amount_v120 {
                frame = frame.v120(Axis::Horizontal, v120 as i32);
            }
        }
        if vertical_amount != 0.0 {
            frame =
                frame.relative_direction(Axis::Vertical, event.relative_direction(Axis::Vertical));
            frame = frame.value(Axis::Vertical, vertical_amount);
            if let Some(v120) = vertical_amount_v120 {
                frame = frame.v120(Axis::Vertical, v120 as i32);
            }
        }

        if source == AxisSource::Finger {
            if event.amount(Axis::Horizontal) == Some(0.0) {
                frame = frame.stop(Axis::Horizontal);
            }
            if event.amount(Axis::Vertical) == Some(0.0) {
                frame = frame.stop(Axis::Vertical);
            }
        }

        pointer.axis(self, frame);
        pointer.frame(self);
    }

    fn on_tablet_tool_axis<I: InputBackend>(&mut self, event: I::TabletToolAxisEvent)
    where
        I::Device: 'static, // Needed for downcasting.
    {
        let Some(pos) = self.compute_tablet_position(&event) else {
            return;
        };

        if let Some(output) = self.neori.screenshot_ui.selection_output() {
            let geom = self.neori.global_space.output_geometry(output).unwrap();
            let point = (pos - geom.loc.to_f64())
                .to_physical(output.current_scale().fractional_scale())
                .to_i32_round::<i32>();

            self.neori.screenshot_ui.pointer_motion(point, None);
        }

        if let Some(mru_output) = self.neori.window_mru_ui.output() {
            if let Some((output, pos_within_output)) = self.neori.output_under(pos) {
                if mru_output == output {
                    self.neori.window_mru_ui.pointer_motion(pos_within_output);
                }
            }
        }

        let under = self.neori.contents_under(pos);

        let tablet_seat = self.neori.seat.tablet_seat();
        let tablet = tablet_seat.get_tablet(&TabletDescriptor::from(&event.device()));
        let tool = tablet_seat.get_tool(&event.tool());
        if let (Some(tablet), Some(tool)) = (tablet, tool) {
            if event.pressure_has_changed() {
                tool.pressure(event.pressure());
            }
            if event.distance_has_changed() {
                tool.distance(event.distance());
            }
            if event.tilt_has_changed() {
                tool.tilt(event.tilt());
            }
            if event.slider_has_changed() {
                tool.slider_position(event.slider_position());
            }
            if event.rotation_has_changed() {
                tool.rotation(event.rotation());
            }
            if event.wheel_has_changed() {
                tool.wheel(event.wheel_delta(), event.wheel_delta_discrete());
            }

            tool.motion(
                pos,
                under.surface,
                &tablet,
                SERIAL_COUNTER.next_serial(),
                event.time_msec(),
            );

            self.neori.pointer_visibility = PointerVisibility::Visible;
            self.neori.tablet_cursor_location = Some(pos);
        }

        // Redraw to update the cursor position.
        // FIXME: redraw only outputs overlapping the cursor.
        self.neori.queue_redraw_all();
    }

    fn on_tablet_tool_tip<I: InputBackend>(&mut self, event: I::TabletToolTipEvent) {
        let tool = self.neori.seat.tablet_seat().get_tool(&event.tool());

        let Some(tool) = tool else {
            return;
        };
        let tip_state = event.tip_state();

        let is_overview_open = self.neori.layout.is_overview_open();

        match tip_state {
            TabletToolTipState::Down => {
                let serial = SERIAL_COUNTER.next_serial();
                tool.tip_down(serial, event.time_msec());

                if let Some(pos) = self.neori.tablet_cursor_location {
                    let under = self.neori.contents_under(pos);

                    if self.neori.screenshot_ui.is_open() {
                        let mod_key = self.backend.mod_key(&self.neori.config.borrow());
                        let mods = self.neori.seat.get_keyboard().unwrap().modifier_state();
                        let modifiers = modifiers_from_state(mods);
                        let mod_down = modifiers.contains(mod_key.to_modifiers());

                        // If we'll be moving the existing selection, use the selection output.
                        let output = if mod_down {
                            self.neori.screenshot_ui.selection_output()
                        } else {
                            under.output.as_ref()
                        };

                        if let Some(output) = output.cloned() {
                            let geom = self.neori.global_space.output_geometry(&output).unwrap();
                            let point = (pos - geom.loc.to_f64())
                                .to_physical(output.current_scale().fractional_scale())
                                .to_i32_round();

                            if self
                                .neori
                                .screenshot_ui
                                .pointer_down(output, point, None, mod_down)
                            {
                                self.neori.queue_redraw_all();
                            }
                        }
                    } else if let Some(mru_output) = self.neori.window_mru_ui.output() {
                        if let Some((output, pos_within_output)) = self.neori.output_under(pos) {
                            if mru_output == output {
                                let id = self.neori.window_mru_ui.pointer_motion(pos_within_output);
                                if id.is_some() {
                                    self.confirm_mru();
                                } else {
                                    self.neori.cancel_mru();
                                }
                            } else {
                                self.neori.cancel_mru();
                            }
                        }
                    } else if let Some((window, _)) = under.window {
                        if let Some(output) = is_overview_open.then_some(under.output).flatten() {
                            let mut workspaces = self.neori.layout.workspaces();
                            if let Some(ws_idx) = workspaces.find_map(|(_, ws_idx, ws)| {
                                ws.windows().any(|w| w.window == window).then_some(ws_idx)
                            }) {
                                drop(workspaces);
                                self.neori.layout.focus_output(&output);
                                self.neori.layout.toggle_overview_to_workspace(ws_idx);
                            }
                        }

                        self.neori.layout.activate_window(&window);

                        // FIXME: granular.
                        self.neori.queue_redraw_all();
                    } else if let Some((output, ws)) = is_overview_open
                        .then(|| self.neori.workspace_under(false, pos))
                        .flatten()
                    {
                        let ws_idx = self.neori.layout.find_workspace_by_id(ws.id()).unwrap().0;

                        self.neori.layout.focus_output(&output);
                        self.neori.layout.toggle_overview_to_workspace(ws_idx);

                        // FIXME: granular.
                        self.neori.queue_redraw_all();
                    } else if let Some(output) = under.output {
                        self.neori.layout.focus_output(&output);

                        // FIXME: granular.
                        self.neori.queue_redraw_all();
                    }
                    self.neori.focus_layer_surface_if_on_demand(under.layer);
                }
            }
            TabletToolTipState::Up => {
                if let Some(capture) = self.neori.screenshot_ui.pointer_up(None) {
                    if capture {
                        self.confirm_screenshot(true);
                    } else {
                        self.neori.queue_redraw_all();
                    }
                }

                tool.tip_up(event.time_msec());
            }
        }
    }

    fn on_tablet_tool_proximity<I: InputBackend>(&mut self, event: I::TabletToolProximityEvent)
    where
        I::Device: 'static, // Needed for downcasting.
    {
        let Some(pos) = self.compute_tablet_position(&event) else {
            return;
        };

        let under = self.neori.contents_under(pos);

        let tablet_seat = self.neori.seat.tablet_seat();
        let display_handle = self.neori.display_handle.clone();
        let tool = tablet_seat.add_tool::<Self>(self, &display_handle, &event.tool());
        let tablet = tablet_seat.get_tablet(&TabletDescriptor::from(&event.device()));
        if let Some(tablet) = tablet {
            match event.state() {
                ProximityState::In => {
                    if let Some(under) = under.surface {
                        tool.proximity_in(
                            pos,
                            under,
                            &tablet,
                            SERIAL_COUNTER.next_serial(),
                            event.time_msec(),
                        );
                    }
                    self.neori.pointer_visibility = PointerVisibility::Visible;
                    self.neori.tablet_cursor_location = Some(pos);
                }
                ProximityState::Out => {
                    tool.proximity_out(event.time_msec());

                    // Move the mouse pointer here to avoid discontinuity.
                    //
                    // Plus, Wayland SDL2 currently warps the pointer into some weird
                    // location on proximity out, so this should help it a little.
                    if let Some(pos) = self.neori.tablet_cursor_location {
                        self.move_cursor(pos);
                    }

                    self.neori.pointer_visibility = PointerVisibility::Visible;
                    self.neori.tablet_cursor_location = None;
                }
            }

            // FIXME: granular.
            self.neori.queue_redraw_all();
        }
    }

    fn on_tablet_tool_button<I: InputBackend>(&mut self, event: I::TabletToolButtonEvent) {
        const BTN_STYLUS3: u32 = 0x149;
        const BTN_STYLUS: u32 = 0x14b;
        const BTN_STYLUS2: u32 = 0x14c;

        let tool = self.neori.seat.tablet_seat().get_tool(&event.tool());

        if let Some(tool) = tool {
            let button = event.button();

            if self.neori.suppressed_buttons.remove(&button) {
                return;
            }

            let trigger = match button {
                BTN_STYLUS => Some(Trigger::TabletStylusButton1),
                BTN_STYLUS2 => Some(Trigger::TabletStylusButton2),
                BTN_STYLUS3 => Some(Trigger::TabletStylusButton3),
                _ => None,
            };

            if let Some(trigger) = trigger {
                if event.button_state() == ButtonState::Pressed {
                    let mod_key = self.backend.mod_key(&self.neori.config.borrow());
                    let mods = self.neori.seat.get_keyboard().unwrap().modifier_state();
                    let modifiers = modifiers_from_state(mods);

                    if self
                        .neori
                        .mods_with_tablet_stylus_binds
                        .contains(&modifiers)
                    {
                        let bind = {
                            let config = self.neori.config.borrow();
                            let bindings = config.binds.0.iter();
                            find_configured_bind(bindings, mod_key, trigger, mods)
                        }
                        .filter(|bind| {
                            !self.neori.screenshot_ui.is_open()
                                || allowed_during_screenshot(&bind.action)
                        });
                        if let Some(bind) = bind {
                            self.neori.suppressed_buttons.insert(button);
                            self.handle_bind(bind.clone());
                            return;
                        }
                    }
                }
            }

            tool.button(
                button,
                event.button_state(),
                SERIAL_COUNTER.next_serial(),
                event.time_msec(),
            );
        }
    }

    fn on_gesture_swipe_begin<I: InputBackend>(&mut self, event: I::GestureSwipeBeginEvent) {
        if self.neori.window_mru_ui.is_open() {
            // Don't start swipe gestures while in the MRU.
            return;
        }

        if event.fingers() == 3 {
            self.neori.gesture_swipe_3f_cumulative = Some((0., 0.));

            // We handled this event.
            return;
        } else if event.fingers() == 4 {
            self.neori.layout.overview_gesture_begin();
            self.neori.queue_redraw_all();

            // We handled this event.
            return;
        }

        let serial = SERIAL_COUNTER.next_serial();
        let pointer = self.neori.seat.get_pointer().unwrap();

        if self.update_pointer_contents() {
            pointer.frame(self);
        }

        pointer.gesture_swipe_begin(
            self,
            &GestureSwipeBeginEvent {
                serial,
                time: event.time_msec(),
                fingers: event.fingers(),
            },
        );
    }

    fn on_gesture_swipe_update<I: InputBackend + 'static>(
        &mut self,
        event: I::GestureSwipeUpdateEvent,
    ) where
        I::Device: 'static,
    {
        let mut delta_x = event.delta_x();
        let mut delta_y = event.delta_y();

        if let Some(libinput_event) =
            (&event as &dyn Any).downcast_ref::<input::event::gesture::GestureSwipeUpdateEvent>()
        {
            delta_x = libinput_event.dx_unaccelerated();
            delta_y = libinput_event.dy_unaccelerated();
        }

        let uninverted_delta_y = delta_y;

        let device = event.device();
        if let Some(device) = (&device as &dyn Any).downcast_ref::<input::Device>() {
            if device.config_scroll_natural_scroll_enabled() {
                delta_x = -delta_x;
                delta_y = -delta_y;
            }
        }

        let is_overview_open = self.neori.layout.is_overview_open();

        if let Some((cx, cy)) = &mut self.neori.gesture_swipe_3f_cumulative {
            *cx += delta_x;
            *cy += delta_y;

            // Check if the gesture moved far enough to decide. Threshold copied from GNOME Shell.
            let (cx, cy) = (*cx, *cy);
            if cx * cx + cy * cy >= 16. * 16. {
                self.neori.gesture_swipe_3f_cumulative = None;

                if let Some(output) = self.neori.output_under_cursor() {
                    if cx.abs() > cy.abs() {
                        let output_ws = if is_overview_open {
                            self.neori.workspace_under_cursor(true)
                        } else {
                            // We don't want to accidentally "catch" the wrong workspace during
                            // animations.
                            self.neori.output_under_cursor().and_then(|output| {
                                let mon = self.neori.layout.monitor_for_output(&output)?;
                                Some((output, mon.active_workspace_ref()))
                            })
                        };

                        if let Some((output, ws)) = output_ws {
                            let ws_idx = self.neori.layout.find_workspace_by_id(ws.id()).unwrap().0;
                            self.neori.layout.view_offset_gesture_begin(
                                &output,
                                Some(ws_idx),
                                true,
                            );
                        }
                    } else {
                        self.neori
                            .layout
                            .workspace_switch_gesture_begin(&output, true);
                    }
                }
            }
        }

        let timestamp = Duration::from_micros(event.time());

        let mut handled = false;
        let res = self
            .neori
            .layout
            .workspace_switch_gesture_update(delta_y, timestamp, true);
        if let Some(output) = res {
            if let Some(output) = output {
                self.neori.queue_redraw(&output);
            }
            handled = true;
        }

        let res = self
            .neori
            .layout
            .view_offset_gesture_update(delta_x, timestamp, true);
        if let Some(output) = res {
            if let Some(output) = output {
                self.neori.queue_redraw(&output);
            }
            handled = true;
        }

        let res = self
            .neori
            .layout
            .overview_gesture_update(-uninverted_delta_y, timestamp);
        if let Some(redraw) = res {
            if redraw {
                self.neori.queue_redraw_all();
            }
            handled = true;
        }

        if handled {
            // We handled this event.
            return;
        }

        let pointer = self.neori.seat.get_pointer().unwrap();

        if self.update_pointer_contents() {
            pointer.frame(self);
        }

        pointer.gesture_swipe_update(
            self,
            &GestureSwipeUpdateEvent {
                time: event.time_msec(),
                delta: event.delta(),
            },
        );
    }

    fn on_gesture_swipe_end<I: InputBackend>(&mut self, event: I::GestureSwipeEndEvent) {
        self.neori.gesture_swipe_3f_cumulative = None;

        let mut handled = false;
        let res = self.neori.layout.workspace_switch_gesture_end(Some(true));
        if let Some(output) = res {
            self.neori.queue_redraw(&output);
            handled = true;
        }

        let res = self.neori.layout.view_offset_gesture_end(Some(true));
        if let Some(output) = res {
            self.neori.queue_redraw(&output);
            handled = true;
        }

        let res = self.neori.layout.overview_gesture_end();
        if res {
            self.neori.queue_redraw_all();
            handled = true;
        }

        if handled {
            // We handled this event.
            return;
        }

        let serial = SERIAL_COUNTER.next_serial();
        let pointer = self.neori.seat.get_pointer().unwrap();

        if self.update_pointer_contents() {
            pointer.frame(self);
        }

        pointer.gesture_swipe_end(
            self,
            &GestureSwipeEndEvent {
                serial,
                time: event.time_msec(),
                cancelled: event.cancelled(),
            },
        );
    }

    fn on_gesture_pinch_begin<I: InputBackend>(&mut self, event: I::GesturePinchBeginEvent) {
        let serial = SERIAL_COUNTER.next_serial();
        let pointer = self.neori.seat.get_pointer().unwrap();

        if self.update_pointer_contents() {
            pointer.frame(self);
        }

        pointer.gesture_pinch_begin(
            self,
            &GesturePinchBeginEvent {
                serial,
                time: event.time_msec(),
                fingers: event.fingers(),
            },
        );
    }

    fn on_gesture_pinch_update<I: InputBackend>(&mut self, event: I::GesturePinchUpdateEvent) {
        let pointer = self.neori.seat.get_pointer().unwrap();

        if self.update_pointer_contents() {
            pointer.frame(self);
        }

        pointer.gesture_pinch_update(
            self,
            &GesturePinchUpdateEvent {
                time: event.time_msec(),
                delta: event.delta(),
                scale: event.scale(),
                rotation: event.rotation(),
            },
        );
    }

    fn on_gesture_pinch_end<I: InputBackend>(&mut self, event: I::GesturePinchEndEvent) {
        let serial = SERIAL_COUNTER.next_serial();
        let pointer = self.neori.seat.get_pointer().unwrap();

        if self.update_pointer_contents() {
            pointer.frame(self);
        }

        pointer.gesture_pinch_end(
            self,
            &GesturePinchEndEvent {
                serial,
                time: event.time_msec(),
                cancelled: event.cancelled(),
            },
        );
    }

    fn on_gesture_hold_begin<I: InputBackend>(&mut self, event: I::GestureHoldBeginEvent) {
        let serial = SERIAL_COUNTER.next_serial();
        let pointer = self.neori.seat.get_pointer().unwrap();

        if self.update_pointer_contents() {
            pointer.frame(self);
        }

        pointer.gesture_hold_begin(
            self,
            &GestureHoldBeginEvent {
                serial,
                time: event.time_msec(),
                fingers: event.fingers(),
            },
        );
    }

    fn on_gesture_hold_end<I: InputBackend>(&mut self, event: I::GestureHoldEndEvent) {
        let serial = SERIAL_COUNTER.next_serial();
        let pointer = self.neori.seat.get_pointer().unwrap();

        if self.update_pointer_contents() {
            pointer.frame(self);
        }

        pointer.gesture_hold_end(
            self,
            &GestureHoldEndEvent {
                serial,
                time: event.time_msec(),
                cancelled: event.cancelled(),
            },
        );
    }

    fn compute_absolute_location<I: InputBackend>(
        &self,
        evt: &impl AbsolutePositionEvent<I>,
        fallback_output: Option<&Output>,
    ) -> Option<Point<f64, Logical>> {
        let output = evt.device().output(self);
        let output = output.filter(|output| self.neori.output_exists(output));
        let output = output.as_ref().or(fallback_output)?;
        let output_geo = self.neori.global_space.output_geometry(output).unwrap();
        let transform = output.current_transform();
        let size = transform.invert().transform_size(output_geo.size);
        Some(
            transform.transform_point_in(evt.position_transformed(size), &size.to_f64())
                + output_geo.loc.to_f64(),
        )
    }

    /// Computes the cursor position for the touch event.
    ///
    /// This function handles the touch output mapping, as well as coordinate transform
    fn compute_touch_location<I: InputBackend>(
        &self,
        evt: &impl AbsolutePositionEvent<I>,
    ) -> Option<Point<f64, Logical>> {
        self.compute_absolute_location(evt, self.neori.output_for_touch())
    }

    fn on_touch_down<I: InputBackend>(&mut self, evt: I::TouchDownEvent) {
        let Some(handle) = self.neori.seat.get_touch() else {
            return;
        };
        let Some(pos) = self.compute_touch_location(&evt) else {
            return;
        };
        let slot = evt.slot();

        let serial = SERIAL_COUNTER.next_serial();

        let under = self.neori.contents_under(pos);

        let mod_key = self.backend.mod_key(&self.neori.config.borrow());
        let mods = self.neori.seat.get_keyboard().unwrap().modifier_state();
        let mods = modifiers_from_state(mods);
        let mod_down = mods.contains(mod_key.to_modifiers());

        if self.neori.screenshot_ui.is_open() {
            // If we'll be moving the existing selection, use the selection output.
            let output = if mod_down {
                self.neori.screenshot_ui.selection_output()
            } else {
                under.output.as_ref()
            };

            if let Some(output) = output.cloned() {
                let geom = self.neori.global_space.output_geometry(&output).unwrap();
                let point = (pos - geom.loc.to_f64())
                    .to_physical(output.current_scale().fractional_scale())
                    .to_i32_round();

                if self
                    .neori
                    .screenshot_ui
                    .pointer_down(output, point, Some(slot), mod_down)
                {
                    self.neori.queue_redraw_all();
                }
            }
        } else if let Some(mru_output) = self.neori.window_mru_ui.output() {
            if let Some((output, pos_within_output)) = self.neori.output_under(pos) {
                if mru_output == output {
                    let id = self.neori.window_mru_ui.pointer_motion(pos_within_output);
                    if id.is_some() {
                        self.confirm_mru();
                    } else {
                        self.neori.cancel_mru();
                    }
                } else {
                    self.neori.cancel_mru();
                }
            }
        } else if !handle.is_grabbed() {
            if self.neori.layout.is_overview_open()
                && !mod_down
                && under.layer.is_none()
                && under.output.is_some()
            {
                let (output, pos_within_output) = self.neori.output_under(pos).unwrap();
                let output = output.clone();

                let mut matched_narrow = true;
                let mut ws = self.neori.workspace_under(false, pos);
                if ws.is_none() {
                    matched_narrow = false;
                    ws = self.neori.workspace_under(true, pos);
                }
                let ws_id = ws.map(|(_, ws)| ws.id());

                let mapped = self.neori.window_under(pos);
                let window = mapped.map(|mapped| mapped.window.clone());

                let start_data = TouchGrabStartData {
                    focus: None,
                    slot,
                    location: pos,
                };
                let start_timestamp = Duration::from_micros(evt.time());
                let grab = TouchOverviewGrab::new(
                    start_data,
                    start_timestamp,
                    output,
                    pos_within_output,
                    ws_id,
                    matched_narrow,
                    window,
                );
                handle.set_grab(self, grab, serial);
            } else if let Some((window, _)) = under.window {
                self.neori.layout.activate_window(&window);

                // Check if we need to start a touch move grab.
                if mod_down {
                    let start_data = TouchGrabStartData {
                        focus: None,
                        slot,
                        location: pos,
                    };
                    let start_data = PointerOrTouchStartData::Touch(start_data);
                    if let Some(grab) = MoveGrab::new(self, start_data, window.clone(), true, None)
                    {
                        handle.set_grab(self, grab, serial);
                    }
                }

                // FIXME: granular.
                self.neori.queue_redraw_all();
            } else if let Some(output) = under.output {
                self.neori.layout.focus_output(&output);

                // FIXME: granular.
                self.neori.queue_redraw_all();
            }
            self.neori.focus_layer_surface_if_on_demand(under.layer);
        };

        handle.down(
            self,
            under.surface,
            &DownEvent {
                slot,
                location: pos,
                serial,
                time: evt.time_msec(),
            },
        );

        // We're using touch, hide the pointer.
        self.neori.pointer_visibility = PointerVisibility::Disabled;
    }
    fn on_touch_up<I: InputBackend>(&mut self, evt: I::TouchUpEvent) {
        let Some(handle) = self.neori.seat.get_touch() else {
            return;
        };
        let slot = evt.slot();

        if let Some(capture) = self.neori.screenshot_ui.pointer_up(Some(slot)) {
            if capture {
                self.confirm_screenshot(true);
            } else {
                self.neori.queue_redraw_all();
            }
        }

        let serial = SERIAL_COUNTER.next_serial();
        handle.up(
            self,
            &UpEvent {
                slot,
                serial,
                time: evt.time_msec(),
            },
        )
    }
    fn on_touch_motion<I: InputBackend>(&mut self, evt: I::TouchMotionEvent) {
        let Some(handle) = self.neori.seat.get_touch() else {
            return;
        };
        let Some(pos) = self.compute_touch_location(&evt) else {
            return;
        };
        let slot = evt.slot();

        if let Some(output) = self.neori.screenshot_ui.selection_output().cloned() {
            let geom = self.neori.global_space.output_geometry(&output).unwrap();
            let point = (pos - geom.loc.to_f64())
                .to_physical(output.current_scale().fractional_scale())
                .to_i32_round::<i32>();

            self.neori.screenshot_ui.pointer_motion(point, Some(slot));
            self.neori.queue_redraw(&output);
        }

        let under = self.neori.contents_under(pos);
        handle.motion(
            self,
            under.surface,
            &TouchMotionEvent {
                slot,
                location: pos,
                time: evt.time_msec(),
            },
        );

        // Inform the layout of an ongoing DnD operation.
        let is_dnd_grab = handle
            .with_grab(|_, grab| Self::is_dnd_grab(grab.as_any()))
            .unwrap_or(false);
        if is_dnd_grab {
            if let Some((output, pos_within_output)) = self.neori.output_under(pos) {
                let output = output.clone();
                self.neori.layout.dnd_update(output, pos_within_output);
            }
        }
    }
    fn on_touch_frame<I: InputBackend>(&mut self, _evt: I::TouchFrameEvent) {
        let Some(handle) = self.neori.seat.get_touch() else {
            return;
        };
        handle.frame(self);
    }
    fn on_touch_cancel<I: InputBackend>(&mut self, _evt: I::TouchCancelEvent) {
        let Some(handle) = self.neori.seat.get_touch() else {
            return;
        };
        handle.cancel(self);
    }

    fn on_switch_toggle<I: InputBackend>(&mut self, evt: I::SwitchToggleEvent) {
        let Some(switch) = evt.switch() else {
            return;
        };

        if switch == Switch::Lid {
            let is_closed = evt.state() == SwitchState::On;
            trace!("lid switch {}", if is_closed { "closed" } else { "opened" });
            self.set_lid_closed(is_closed);
        }

        let action = {
            let bindings = &self.neori.config.borrow().switch_events;
            find_configured_switch_action(bindings, switch, evt.state())
        };

        if let Some(action) = action {
            self.do_action(action, true);
        }
    }

    pub fn is_dnd_grab(grab: &dyn Any) -> bool {
        // Normal DnD
        grab.is::<DnDGrab<Self, WlDataSource, WlSurface>>()
            // Null-source DnD: weston-dnd --self-only
            || grab.is::<DnDGrab<Self, WlSurface, WlSurface>>()
    }

    fn grab_can_be_cancelled_with_esc(grab: &(dyn PointerGrab<State> + 'static)) -> bool {
        let grab = grab.as_any();

        grab.is::<PickWindowGrab>() || grab.is::<PickColorGrab>() || Self::is_dnd_grab(grab)
    }
}

/// Check whether the key should be intercepted and mark intercepted
/// pressed keys as `suppressed`, thus preventing `releases` corresponding
/// to them from being delivered.
#[allow(clippy::too_many_arguments)]
fn should_intercept_key<'a>(
    suppressed_keys: &mut HashSet<Keycode>,
    bindings: impl IntoIterator<Item = &'a Bind>,
    mod_key: ModKey,
    key_code: Keycode,
    modified: Keysym,
    raw: Option<Keysym>,
    pressed: bool,
    mods: ModifiersState,
    screenshot_ui: &ScreenshotUi,
    disable_power_key_handling: bool,
    is_inhibiting_shortcuts: bool,
) -> FilterResult<Option<Bind>> {
    // Actions are only triggered on presses, release of the key
    // shouldn't try to intercept anything unless we have marked
    // the key to suppress.
    if !pressed && !suppressed_keys.contains(&key_code) {
        return FilterResult::Forward;
    }

    let mut final_bind = find_bind(
        bindings,
        mod_key,
        key_code,
        modified,
        raw,
        mods,
        disable_power_key_handling,
    );

    // Allow only a subset of compositor actions while the screenshot UI is open, since the user
    // cannot see the screen.
    if screenshot_ui.is_open() {
        let mut use_screenshot_ui_action = true;

        if let Some(bind) = &final_bind {
            if allowed_during_screenshot(&bind.action) {
                use_screenshot_ui_action = false;
            }
        }

        if use_screenshot_ui_action {
            if let Some(raw) = raw {
                final_bind = screenshot_ui.action(raw, mods).map(|action| Bind {
                    key: Key {
                        trigger: Trigger::Keysym(raw),
                        // Not entirely correct but it doesn't matter in how we currently use it.
                        modifiers: Modifiers::empty(),
                    },
                    action,
                    repeat: true,
                    cooldown: None,
                    allow_when_locked: false,
                    // The screenshot UI owns the focus anyway, so this doesn't really matter.
                    // But logically, nothing can inhibit its actions. Only opening it can be
                    // inhibited.
                    allow_inhibiting: false,
                    hotkey_overlay_title: None,
                });
            }
        }
    }

    match (final_bind, pressed) {
        (Some(bind), true) => {
            if is_inhibiting_shortcuts && bind.allow_inhibiting {
                FilterResult::Forward
            } else {
                suppressed_keys.insert(key_code);
                FilterResult::Intercept(Some(bind))
            }
        }
        (_, false) => {
            // By this point, we know that the key was suppressed on press. Even if we're inhibiting
            // shortcuts, we should still suppress the release.
            // But we don't need to check for shortcuts inhibition here, because
            // if it was inhibited on press (forwarded to the client), it wouldn't be suppressed,
            // so the release would already have been forwarded at the start of this function.
            suppressed_keys.remove(&key_code);
            FilterResult::Intercept(None)
        }
        (None, true) => FilterResult::Forward,
    }
}

fn find_bind<'a>(
    bindings: impl IntoIterator<Item = &'a Bind>,
    mod_key: ModKey,
    key_code: Keycode,
    modified: Keysym,
    raw: Option<Keysym>,
    mods: ModifiersState,
    disable_power_key_handling: bool,
) -> Option<Bind> {
    use keysyms::*;

    // Handle hardcoded binds.
    #[allow(non_upper_case_globals)] // wat
    let hardcoded_action = match modified.raw() {
        modified @ KEY_XF86Switch_VT_1..=KEY_XF86Switch_VT_12 => {
            let vt = (modified - KEY_XF86Switch_VT_1 + 1) as i32;
            Some(Action::ChangeVt(vt))
        }
        KEY_XF86PowerOff if !disable_power_key_handling => Some(Action::Suspend),
        _ => None,
    };

    if let Some(action) = hardcoded_action {
        return Some(Bind {
            key: Key {
                // Not entirely correct but it doesn't matter in how we currently use it.
                trigger: Trigger::Keysym(modified),
                modifiers: Modifiers::empty(),
            },
            action,
            repeat: true,
            cooldown: None,
            allow_when_locked: false,
            // In a worst-case scenario, the user has no way to unlock the compositor and a
            // misbehaving client has a keyboard shortcuts inhibitor, "jailing" the user.
            // The user must always be able to change VTs to recover from such a situation.
            // It also makes no sense to inhibit the default power key handling.
            // Hardcoded binds must never be inhibited.
            allow_inhibiting: false,
            hotkey_overlay_title: None,
        });
    }

    let bindings: Vec<_> = bindings.into_iter().collect();

    // Keycode binds take priority: they match the physical key regardless of the active layout,
    // so a keycode bind must not be shadowed by a keysym bind that happens to collide on the
    // current layout (e.g. Super+Slash by keycode vs Super+Period on the Hebrew layout).
    let evdev = key_code.raw().saturating_sub(8);
    if let Some(bind) = find_configured_bind(
        bindings.iter().copied(),
        mod_key,
        Trigger::Keycode(evdev),
        mods,
    ) {
        return Some(bind);
    }

    // Keysym binds are layout-dependent by design.
    if let Some(raw) = raw {
        if let Some(bind) = find_configured_bind(
            bindings.iter().copied(),
            mod_key,
            Trigger::Keysym(raw),
            mods,
        ) {
            return Some(bind);
        }
    }

    None
}

fn find_configured_bind<'a>(
    bindings: impl IntoIterator<Item = &'a Bind>,
    mod_key: ModKey,
    trigger: Trigger,
    mods: ModifiersState,
) -> Option<Bind> {
    // Handle configured binds.
    let mut modifiers = modifiers_from_state(mods);

    let mod_down = modifiers_from_state(mods).contains(mod_key.to_modifiers());
    if mod_down {
        modifiers |= Modifiers::COMPOSITOR;
    }

    for bind in bindings {
        if bind.key.trigger != trigger {
            continue;
        }

        let mut bind_modifiers = bind.key.modifiers;
        if bind_modifiers.contains(Modifiers::COMPOSITOR) {
            bind_modifiers |= mod_key.to_modifiers();
        } else if bind_modifiers.contains(mod_key.to_modifiers()) {
            bind_modifiers |= Modifiers::COMPOSITOR;
        }

        if bind_modifiers == modifiers {
            return Some(bind.clone());
        }
    }

    None
}

fn find_configured_switch_action(
    bindings: &SwitchBinds,
    switch: Switch,
    state: SwitchState,
) -> Option<Action> {
    let switch_action = match (switch, state) {
        (Switch::Lid, SwitchState::Off) => &bindings.lid_open,
        (Switch::Lid, SwitchState::On) => &bindings.lid_close,
        (Switch::TabletMode, SwitchState::Off) => &bindings.tablet_mode_off,
        (Switch::TabletMode, SwitchState::On) => &bindings.tablet_mode_on,
        _ => unreachable!(),
    };
    switch_action
        .as_ref()
        .map(|switch_action| Action::Spawn(switch_action.spawn.clone()))
}

fn modifiers_from_state(mods: ModifiersState) -> Modifiers {
    let mut modifiers = Modifiers::empty();
    if mods.ctrl {
        modifiers |= Modifiers::CTRL;
    }
    if mods.shift {
        modifiers |= Modifiers::SHIFT;
    }
    if mods.alt {
        modifiers |= Modifiers::ALT;
    }
    if mods.logo {
        modifiers |= Modifiers::SUPER;
    }
    if mods.iso_level3_shift {
        modifiers |= Modifiers::ISO_LEVEL3_SHIFT;
    }
    if mods.iso_level5_shift {
        modifiers |= Modifiers::ISO_LEVEL5_SHIFT;
    }
    modifiers
}

fn should_activate_monitors<I: InputBackend>(event: &InputEvent<I>) -> bool {
    match event {
        InputEvent::Keyboard { event } if event.state() == KeyState::Pressed => true,
        InputEvent::PointerButton { event } if event.state() == ButtonState::Pressed => true,
        InputEvent::PointerMotion { .. }
        | InputEvent::PointerMotionAbsolute { .. }
        | InputEvent::PointerAxis { .. }
        | InputEvent::GestureSwipeBegin { .. }
        | InputEvent::GesturePinchBegin { .. }
        | InputEvent::GestureHoldBegin { .. }
        | InputEvent::TouchDown { .. }
        | InputEvent::TouchMotion { .. }
        | InputEvent::TabletToolAxis { .. }
        | InputEvent::TabletToolProximity { .. }
        | InputEvent::TabletToolTip { .. }
        | InputEvent::TabletToolButton { .. } => true,
        // Ignore events like device additions and removals, key releases, gesture ends.
        _ => false,
    }
}

fn should_hide_hotkey_overlay<I: InputBackend>(event: &InputEvent<I>) -> bool {
    match event {
        InputEvent::Keyboard { event } if event.state() == KeyState::Pressed => true,
        InputEvent::PointerButton { event } if event.state() == ButtonState::Pressed => true,
        InputEvent::PointerAxis { .. }
        | InputEvent::GestureSwipeBegin { .. }
        | InputEvent::GesturePinchBegin { .. }
        | InputEvent::TouchDown { .. }
        | InputEvent::TouchMotion { .. }
        | InputEvent::TabletToolTip { .. }
        | InputEvent::TabletToolButton { .. } => true,
        _ => false,
    }
}

fn should_hide_exit_confirm_dialog<I: InputBackend>(event: &InputEvent<I>) -> bool {
    match event {
        InputEvent::Keyboard { event } if event.state() == KeyState::Pressed => true,
        InputEvent::PointerButton { event } if event.state() == ButtonState::Pressed => true,
        InputEvent::PointerAxis { .. }
        | InputEvent::GestureSwipeBegin { .. }
        | InputEvent::GesturePinchBegin { .. }
        | InputEvent::TouchDown { .. }
        | InputEvent::TouchMotion { .. }
        | InputEvent::TabletToolTip { .. }
        | InputEvent::TabletToolButton { .. } => true,
        _ => false,
    }
}

fn should_notify_activity<I: InputBackend>(event: &InputEvent<I>) -> bool {
    !matches!(
        event,
        InputEvent::DeviceAdded { .. } | InputEvent::DeviceRemoved { .. }
    )
}

fn should_reset_pointer_inactivity_timer<I: InputBackend>(event: &InputEvent<I>) -> bool {
    matches!(
        event,
        InputEvent::PointerAxis { .. }
            | InputEvent::PointerButton { .. }
            | InputEvent::PointerMotion { .. }
            | InputEvent::PointerMotionAbsolute { .. }
            | InputEvent::TabletToolAxis { .. }
            | InputEvent::TabletToolButton { .. }
            | InputEvent::TabletToolProximity { .. }
            | InputEvent::TabletToolTip { .. }
    )
}

fn allowed_when_locked(action: &Action) -> bool {
    matches!(
        action,
        Action::Quit(_)
            | Action::ChangeVt(_)
            | Action::Suspend
            | Action::PowerOffMonitors
            | Action::PowerOnMonitors
            | Action::SwitchLayout(_)
            | Action::ToggleKeyboardShortcutsInhibit
    )
}

fn allowed_during_screenshot(action: &Action) -> bool {
    matches!(
        action,
        Action::Quit(_)
            | Action::ChangeVt(_)
            | Action::Suspend
            | Action::PowerOffMonitors
            | Action::PowerOnMonitors
            // Intended for binds such as volume up/down, lock the screen, etc.
            | Action::Spawn(_)
            | Action::SpawnSh(_)
            // The screenshot UI can handle these.
            | Action::MoveColumnLeft
            | Action::MoveColumnLeftOrToMonitorLeft
            | Action::MoveColumnRight
            | Action::MoveColumnRightOrToMonitorRight
            | Action::MoveWindowUp
            | Action::MoveWindowUpOrToWorkspaceUp
            | Action::MoveWindowDown
            | Action::MoveWindowDownOrToWorkspaceDown
            | Action::MoveColumnToMonitorLeft
            | Action::MoveColumnToMonitorRight
            | Action::MoveColumnToMonitorUp
            | Action::MoveColumnToMonitorDown
            | Action::MoveColumnToMonitorPrevious
            | Action::MoveColumnToMonitorNext
            | Action::MoveColumnToMonitor(_)
            | Action::MoveWindowToMonitorLeft
            | Action::MoveWindowToMonitorRight
            | Action::MoveWindowToMonitorUp
            | Action::MoveWindowToMonitorDown
            | Action::MoveWindowToMonitorPrevious
            | Action::MoveWindowToMonitorNext
            | Action::MoveWindowToMonitor(_)
            | Action::SetWindowWidth(_)
            | Action::SetWindowHeight(_)
            | Action::SetColumnWidth(_)
    )
}

fn hardcoded_overview_bind(raw: Keysym, mods: ModifiersState) -> Option<Bind> {
    let mods = modifiers_from_state(mods);
    if !mods.is_empty() {
        return None;
    }

    let mut repeat = true;
    let action = match raw {
        Keysym::Escape | Keysym::Return => {
            repeat = false;
            Action::ToggleOverview
        }
        Keysym::Left => Action::FocusColumnLeft,
        Keysym::Right => Action::FocusColumnRight,
        Keysym::Up => Action::FocusWindowOrWorkspaceUp,
        Keysym::Down => Action::FocusWindowOrWorkspaceDown,
        _ => {
            return None;
        }
    };

    Some(Bind {
        key: Key {
            trigger: Trigger::Keysym(raw),
            modifiers: Modifiers::empty(),
        },
        action,
        repeat,
        cooldown: None,
        allow_when_locked: false,
        allow_inhibiting: false,
        hotkey_overlay_title: None,
    })
}

pub fn apply_libinput_settings(config: &neori_config::Input, device: &mut input::Device) {
    // According to Mutter code, this setting is specific to touchpads.
    let is_touchpad = device.config_tap_finger_count() > 0;
    if is_touchpad {
        let c = &config.touchpad;
        let _ = device.config_send_events_set_mode(if c.off {
            input::SendEventsMode::DISABLED
        } else if c.disabled_on_external_mouse {
            input::SendEventsMode::DISABLED_ON_EXTERNAL_MOUSE
        } else {
            input::SendEventsMode::ENABLED
        });
        let _ = device.config_tap_set_enabled(c.tap);
        let _ = device.config_dwt_set_enabled(c.dwt);
        let _ = device.config_dwtp_set_enabled(c.dwtp);
        let _ = device.config_tap_set_drag_lock_enabled(if c.drag_lock {
            input::DragLockState::EnabledTimeout
        } else {
            input::DragLockState::Disabled
        });
        let _ = device.config_scroll_set_natural_scroll_enabled(c.natural_scroll);
        let _ = device.config_accel_set_speed(c.accel_speed.0);
        let _ = device.config_left_handed_set(c.left_handed);
        let _ = device.config_middle_emulation_set_enabled(c.middle_emulation);

        if let Some(drag) = c.drag {
            let _ = device.config_tap_set_drag_enabled(drag);
        } else {
            let default = device.config_tap_default_drag_enabled();
            let _ = device.config_tap_set_drag_enabled(default);
        }

        if let Some(accel_profile) = c.accel_profile {
            let _ = device.config_accel_set_profile(accel_profile.into());
        } else if let Some(default) = device.config_accel_default_profile() {
            let _ = device.config_accel_set_profile(default);
        }

        if let Some(method) = c.scroll_method {
            let _ = device.config_scroll_set_method(method.into());

            if method == neori_config::ScrollMethod::OnButtonDown {
                if let Some(button) = c.scroll_button {
                    let _ = device.config_scroll_set_button(button);
                }
                let _ = device.config_scroll_set_button_lock(if c.scroll_button_lock {
                    input::ScrollButtonLockState::Enabled
                } else {
                    input::ScrollButtonLockState::Disabled
                });
            }
        } else if let Some(default) = device.config_scroll_default_method() {
            let _ = device.config_scroll_set_method(default);

            if default == input::ScrollMethod::OnButtonDown {
                if let Some(button) = c.scroll_button {
                    let _ = device.config_scroll_set_button(button);
                }
                let _ = device.config_scroll_set_button_lock(if c.scroll_button_lock {
                    input::ScrollButtonLockState::Enabled
                } else {
                    input::ScrollButtonLockState::Disabled
                });
            }
        }

        if let Some(tap_button_map) = c.tap_button_map {
            let _ = device.config_tap_set_button_map(tap_button_map.into());
        } else if let Some(default) = device.config_tap_default_button_map() {
            let _ = device.config_tap_set_button_map(default);
        }

        if let Some(method) = c.click_method {
            let _ = device.config_click_set_method(method.into());
        } else if let Some(default) = device.config_click_default_method() {
            let _ = device.config_click_set_method(default);
        }
    }

    // This is how Mutter tells apart mice.
    let mut is_trackball = false;
    let mut is_trackpoint = false;
    if let Some(udev_device) = unsafe { device.udev_device() } {
        if udev_device.property_value("ID_INPUT_TRACKBALL").is_some() {
            is_trackball = true;
        }
        if udev_device
            .property_value("ID_INPUT_POINTINGSTICK")
            .is_some()
        {
            is_trackpoint = true;
        }
    }

    let is_mouse = device.has_capability(input::DeviceCapability::Pointer)
        && !is_touchpad
        && !is_trackball
        && !is_trackpoint;
    if is_mouse {
        let c = &config.mouse;
        let _ = device.config_send_events_set_mode(if c.off {
            input::SendEventsMode::DISABLED
        } else {
            input::SendEventsMode::ENABLED
        });
        let _ = device.config_scroll_set_natural_scroll_enabled(c.natural_scroll);
        let _ = device.config_accel_set_speed(c.accel_speed.0);
        let _ = device.config_left_handed_set(c.left_handed);
        let _ = device.config_middle_emulation_set_enabled(c.middle_emulation);

        if let Some(accel_profile) = c.accel_profile {
            let _ = device.config_accel_set_profile(accel_profile.into());
        } else if let Some(default) = device.config_accel_default_profile() {
            let _ = device.config_accel_set_profile(default);
        }

        if let Some(method) = c.scroll_method {
            let _ = device.config_scroll_set_method(method.into());

            if method == neori_config::ScrollMethod::OnButtonDown {
                if let Some(button) = c.scroll_button {
                    let _ = device.config_scroll_set_button(button);
                }
                let _ = device.config_scroll_set_button_lock(if c.scroll_button_lock {
                    input::ScrollButtonLockState::Enabled
                } else {
                    input::ScrollButtonLockState::Disabled
                });
            }
        } else if let Some(default) = device.config_scroll_default_method() {
            let _ = device.config_scroll_set_method(default);

            if default == input::ScrollMethod::OnButtonDown {
                if let Some(button) = c.scroll_button {
                    let _ = device.config_scroll_set_button(button);
                }
                let _ = device.config_scroll_set_button_lock(if c.scroll_button_lock {
                    input::ScrollButtonLockState::Enabled
                } else {
                    input::ScrollButtonLockState::Disabled
                });
            }
        }
    }

    if is_trackball {
        let c = &config.trackball;
        let _ = device.config_send_events_set_mode(if c.off {
            input::SendEventsMode::DISABLED
        } else {
            input::SendEventsMode::ENABLED
        });
        let _ = device.config_scroll_set_natural_scroll_enabled(c.natural_scroll);
        let _ = device.config_accel_set_speed(c.accel_speed.0);
        let _ = device.config_middle_emulation_set_enabled(c.middle_emulation);
        let _ = device.config_left_handed_set(c.left_handed);

        if let Some(accel_profile) = c.accel_profile {
            let _ = device.config_accel_set_profile(accel_profile.into());
        } else if let Some(default) = device.config_accel_default_profile() {
            let _ = device.config_accel_set_profile(default);
        }

        if let Some(method) = c.scroll_method {
            let _ = device.config_scroll_set_method(method.into());

            if method == neori_config::ScrollMethod::OnButtonDown {
                if let Some(button) = c.scroll_button {
                    let _ = device.config_scroll_set_button(button);
                }
                let _ = device.config_scroll_set_button_lock(if c.scroll_button_lock {
                    input::ScrollButtonLockState::Enabled
                } else {
                    input::ScrollButtonLockState::Disabled
                });
            }
        } else if let Some(default) = device.config_scroll_default_method() {
            let _ = device.config_scroll_set_method(default);

            if default == input::ScrollMethod::OnButtonDown {
                if let Some(button) = c.scroll_button {
                    let _ = device.config_scroll_set_button(button);
                }
                let _ = device.config_scroll_set_button_lock(if c.scroll_button_lock {
                    input::ScrollButtonLockState::Enabled
                } else {
                    input::ScrollButtonLockState::Disabled
                });
            }
        }
    }

    if is_trackpoint {
        let c = &config.trackpoint;
        let _ = device.config_send_events_set_mode(if c.off {
            input::SendEventsMode::DISABLED
        } else {
            input::SendEventsMode::ENABLED
        });
        let _ = device.config_scroll_set_natural_scroll_enabled(c.natural_scroll);
        let _ = device.config_accel_set_speed(c.accel_speed.0);
        let _ = device.config_left_handed_set(c.left_handed);
        let _ = device.config_middle_emulation_set_enabled(c.middle_emulation);

        if let Some(accel_profile) = c.accel_profile {
            let _ = device.config_accel_set_profile(accel_profile.into());
        } else if let Some(default) = device.config_accel_default_profile() {
            let _ = device.config_accel_set_profile(default);
        }

        if let Some(method) = c.scroll_method {
            let _ = device.config_scroll_set_method(method.into());

            if method == neori_config::ScrollMethod::OnButtonDown {
                if let Some(button) = c.scroll_button {
                    let _ = device.config_scroll_set_button(button);
                }
                let _ = device.config_scroll_set_button_lock(if c.scroll_button_lock {
                    input::ScrollButtonLockState::Enabled
                } else {
                    input::ScrollButtonLockState::Disabled
                });
            }
        } else if let Some(default) = device.config_scroll_default_method() {
            let _ = device.config_scroll_set_method(default);

            if default == input::ScrollMethod::OnButtonDown {
                if let Some(button) = c.scroll_button {
                    let _ = device.config_scroll_set_button(button);
                }
                let _ = device.config_scroll_set_button_lock(if c.scroll_button_lock {
                    input::ScrollButtonLockState::Enabled
                } else {
                    input::ScrollButtonLockState::Disabled
                });
            }
        }
    }

    let is_tablet = device.has_capability(input::DeviceCapability::TabletTool);
    if is_tablet {
        let c = &config.tablet;
        let _ = device.config_send_events_set_mode(if c.off {
            input::SendEventsMode::DISABLED
        } else {
            input::SendEventsMode::ENABLED
        });

        #[rustfmt::skip]
        const IDENTITY_MATRIX: [f32; 6] = [
            1., 0., 0.,
            0., 1., 0.,
        ];

        let _ = device.config_calibration_set_matrix(
            c.calibration_matrix
                .as_deref()
                .and_then(|m| m.try_into().ok())
                .or(device.config_calibration_default_matrix())
                .unwrap_or(IDENTITY_MATRIX),
        );

        let _ = device.config_left_handed_set(c.left_handed);
    }

    let is_touch = device.has_capability(input::DeviceCapability::Touch);
    if is_touch {
        let c = &config.touch;
        let _ = device.config_send_events_set_mode(if c.off {
            input::SendEventsMode::DISABLED
        } else {
            input::SendEventsMode::ENABLED
        });

        #[rustfmt::skip]
        const IDENTITY_MATRIX: [f32; 6] = [
            1., 0., 0.,
            0., 1., 0.,
        ];

        let _ = device.config_calibration_set_matrix(
            c.calibration_matrix
                .as_deref()
                .and_then(|m| m.try_into().ok())
                .or(device.config_calibration_default_matrix())
                .unwrap_or(IDENTITY_MATRIX),
        );
    }
}

pub fn mods_with_binds(mod_key: ModKey, binds: &Binds, triggers: &[Trigger]) -> HashSet<Modifiers> {
    let mut rv = HashSet::new();
    for bind in &binds.0 {
        if !triggers.contains(&bind.key.trigger) {
            continue;
        }

        let mut mods = bind.key.modifiers;
        if mods.contains(Modifiers::COMPOSITOR) {
            mods.remove(Modifiers::COMPOSITOR);
            mods.insert(mod_key.to_modifiers());
        }

        rv.insert(mods);
    }

    rv
}

pub fn mods_with_mouse_binds(mod_key: ModKey, binds: &Binds) -> HashSet<Modifiers> {
    mods_with_binds(
        mod_key,
        binds,
        &[
            Trigger::MouseLeft,
            Trigger::MouseRight,
            Trigger::MouseMiddle,
            Trigger::MouseBack,
            Trigger::MouseForward,
        ],
    )
}

pub fn mods_with_wheel_binds(mod_key: ModKey, binds: &Binds) -> HashSet<Modifiers> {
    mods_with_binds(
        mod_key,
        binds,
        &[
            Trigger::WheelScrollUp,
            Trigger::WheelScrollDown,
            Trigger::WheelScrollLeft,
            Trigger::WheelScrollRight,
        ],
    )
}

pub fn mods_with_finger_scroll_binds(mod_key: ModKey, binds: &Binds) -> HashSet<Modifiers> {
    mods_with_binds(
        mod_key,
        binds,
        &[
            Trigger::TouchpadScrollUp,
            Trigger::TouchpadScrollDown,
            Trigger::TouchpadScrollLeft,
            Trigger::TouchpadScrollRight,
        ],
    )
}

pub fn mods_with_tablet_stylus_binds(mod_key: ModKey, binds: &Binds) -> HashSet<Modifiers> {
    mods_with_binds(
        mod_key,
        binds,
        &[
            Trigger::TabletStylusButton1,
            Trigger::TabletStylusButton2,
            Trigger::TabletStylusButton3,
        ],
    )
}

fn grab_allows_hot_corner(grab: &(dyn PointerGrab<State> + 'static)) -> bool {
    let grab = grab.as_any();

    // We lean on the blocklist approach here since it's not a terribly big deal if hot corner
    // works where it shouldn't, but it could prevent some workflows if the hot corner doesn't work
    // when it should.
    //
    // Some notable grabs not mentioned here:
    // - DnDGrab allows hot corner to DnD across workspaces.
    // - ClickGrab keeps pointer focus on the window, so the hot corner doesn't trigger.
    // - Touch grabs: touch doesn't trigger the hot corner.
    if grab.is::<ResizeGrab>() || grab.is::<SpatialMovementGrab>() {
        return false;
    }

    if let Some(grab) = grab.downcast_ref::<MoveGrab>() {
        // Window move allows hot corner to DnD across workspaces.
        if !grab.is_move() {
            return false;
        }
    }

    true
}

/// Returns an iterator over bindings.
///
/// Includes dynamically populated bindings like the MRU UI.
fn make_binds_iter<'a>(
    config: &'a Config,
    mru: &'a mut WindowMruUi,
    mods: Modifiers,
) -> impl Iterator<Item = &'a Bind> + Clone {
    // Figure out the binds to use depending on whether the MRU is enabled and/or open.
    let general_binds = (!mru.is_open()).then_some(config.binds.0.iter());
    let general_binds = general_binds.into_iter().flatten();

    let mru_binds =
        (config.recent_windows.on || mru.is_open()).then_some(config.recent_windows.binds.iter());
    let mru_binds = mru_binds.into_iter().flatten();

    let mru_open_binds = mru.is_open().then(|| mru.opened_bindings(mods));
    let mru_open_binds = mru_open_binds.into_iter().flatten();

    // General binds take precedence over the MRU binds.
    general_binds.chain(mru_binds).chain(mru_open_binds)
}

#[cfg(test)]
mod tests {
    use std::cell::Cell;

    use super::*;
    use crate::animation::Clock;

    #[test]
    fn bindings_suppress_keys() {
        let close_keysym = Keysym::q;
        let bindings = Binds(vec![Bind {
            key: Key {
                trigger: Trigger::Keysym(close_keysym),
                modifiers: Modifiers::COMPOSITOR | Modifiers::CTRL,
            },
            action: Action::CloseWindow,
            repeat: true,
            cooldown: None,
            allow_when_locked: false,
            allow_inhibiting: true,
            hotkey_overlay_title: None,
        }]);

        let comp_mod = ModKey::Super;
        let mut suppressed_keys = HashSet::new();

        let screenshot_ui = ScreenshotUi::new(Clock::default(), Default::default());
        let disable_power_key_handling = false;
        let is_inhibiting_shortcuts = Cell::new(false);

        // The key_code we pick is arbitrary, the only thing
        // that matters is that they are different between cases.

        let close_key_code = Keycode::from(close_keysym.raw() + 8u32);
        let close_key_event = |suppr: &mut HashSet<Keycode>, mods: ModifiersState, pressed| {
            should_intercept_key(
                suppr,
                &bindings.0,
                comp_mod,
                close_key_code,
                close_keysym,
                Some(close_keysym),
                pressed,
                mods,
                &screenshot_ui,
                disable_power_key_handling,
                is_inhibiting_shortcuts.get(),
            )
        };

        // Key event with the code which can't trigger any action.
        let none_key_event = |suppr: &mut HashSet<Keycode>, mods: ModifiersState, pressed| {
            should_intercept_key(
                suppr,
                &bindings.0,
                comp_mod,
                Keycode::from(Keysym::l.raw() + 8),
                Keysym::l,
                Some(Keysym::l),
                pressed,
                mods,
                &screenshot_ui,
                disable_power_key_handling,
                is_inhibiting_shortcuts.get(),
            )
        };

        let mut mods = ModifiersState {
            logo: true,
            ctrl: true,
            ..Default::default()
        };

        // Action press/release.

        let filter = close_key_event(&mut suppressed_keys, mods, true);
        assert!(matches!(
            filter,
            FilterResult::Intercept(Some(Bind {
                action: Action::CloseWindow,
                ..
            }))
        ));
        assert!(suppressed_keys.contains(&close_key_code));

        let filter = close_key_event(&mut suppressed_keys, mods, false);
        assert!(matches!(filter, FilterResult::Intercept(None)));
        assert!(suppressed_keys.is_empty());

        // Remove mod to make it for a binding.

        mods.shift = true;
        let filter = close_key_event(&mut suppressed_keys, mods, true);
        assert!(matches!(filter, FilterResult::Forward));

        mods.shift = false;
        let filter = close_key_event(&mut suppressed_keys, mods, false);
        assert!(matches!(filter, FilterResult::Forward));

        // Just none press/release.

        let filter = none_key_event(&mut suppressed_keys, mods, true);
        assert!(matches!(filter, FilterResult::Forward));

        let filter = none_key_event(&mut suppressed_keys, mods, false);
        assert!(matches!(filter, FilterResult::Forward));

        // Press action, press arbitrary, release action, release arbitrary.

        let filter = close_key_event(&mut suppressed_keys, mods, true);
        assert!(matches!(
            filter,
            FilterResult::Intercept(Some(Bind {
                action: Action::CloseWindow,
                ..
            }))
        ));

        let filter = none_key_event(&mut suppressed_keys, mods, true);
        assert!(matches!(filter, FilterResult::Forward));

        let filter = close_key_event(&mut suppressed_keys, mods, false);
        assert!(matches!(filter, FilterResult::Intercept(None)));

        let filter = none_key_event(&mut suppressed_keys, mods, false);
        assert!(matches!(filter, FilterResult::Forward));

        // Trigger and remove all mods.

        let filter = close_key_event(&mut suppressed_keys, mods, true);
        assert!(matches!(
            filter,
            FilterResult::Intercept(Some(Bind {
                action: Action::CloseWindow,
                ..
            }))
        ));

        mods = Default::default();
        let filter = close_key_event(&mut suppressed_keys, mods, false);
        assert!(matches!(filter, FilterResult::Intercept(None)));

        // Ensure that no keys are being suppressed.
        assert!(suppressed_keys.is_empty());

        // Now test shortcut inhibiting.

        // With inhibited shortcuts, we don't intercept our shortcut.
        is_inhibiting_shortcuts.set(true);

        mods = ModifiersState {
            logo: true,
            ctrl: true,
            ..Default::default()
        };

        let filter = close_key_event(&mut suppressed_keys, mods, true);
        assert!(matches!(filter, FilterResult::Forward));
        assert!(suppressed_keys.is_empty());

        let filter = close_key_event(&mut suppressed_keys, mods, false);
        assert!(matches!(filter, FilterResult::Forward));
        assert!(suppressed_keys.is_empty());

        // Toggle it off after pressing the shortcut.
        let filter = close_key_event(&mut suppressed_keys, mods, true);
        assert!(matches!(filter, FilterResult::Forward));
        assert!(suppressed_keys.is_empty());

        is_inhibiting_shortcuts.set(false);

        let filter = close_key_event(&mut suppressed_keys, mods, false);
        assert!(matches!(filter, FilterResult::Forward));
        assert!(suppressed_keys.is_empty());

        // Toggle it on after pressing the shortcut.
        let filter = close_key_event(&mut suppressed_keys, mods, true);
        assert!(matches!(
            filter,
            FilterResult::Intercept(Some(Bind {
                action: Action::CloseWindow,
                ..
            }))
        ));
        assert!(suppressed_keys.contains(&close_key_code));

        is_inhibiting_shortcuts.set(true);

        let filter = close_key_event(&mut suppressed_keys, mods, false);
        assert!(matches!(filter, FilterResult::Intercept(None)));
        assert!(suppressed_keys.is_empty());
    }

    #[test]
    fn comp_mod_handling() {
        let bindings = Binds(vec![
            Bind {
                key: Key {
                    trigger: Trigger::Keysym(Keysym::q),
                    modifiers: Modifiers::COMPOSITOR,
                },
                action: Action::CloseWindow,
                repeat: true,
                cooldown: None,
                allow_when_locked: false,
                allow_inhibiting: true,
                hotkey_overlay_title: None,
            },
            Bind {
                key: Key {
                    trigger: Trigger::Keysym(Keysym::h),
                    modifiers: Modifiers::SUPER,
                },
                action: Action::FocusColumnLeft,
                repeat: true,
                cooldown: None,
                allow_when_locked: false,
                allow_inhibiting: true,
                hotkey_overlay_title: None,
            },
            Bind {
                key: Key {
                    trigger: Trigger::Keysym(Keysym::j),
                    modifiers: Modifiers::empty(),
                },
                action: Action::FocusWindowDown,
                repeat: true,
                cooldown: None,
                allow_when_locked: false,
                allow_inhibiting: true,
                hotkey_overlay_title: None,
            },
            Bind {
                key: Key {
                    trigger: Trigger::Keysym(Keysym::k),
                    modifiers: Modifiers::COMPOSITOR | Modifiers::SUPER,
                },
                action: Action::FocusWindowUp,
                repeat: true,
                cooldown: None,
                allow_when_locked: false,
                allow_inhibiting: true,
                hotkey_overlay_title: None,
            },
            Bind {
                key: Key {
                    trigger: Trigger::Keysym(Keysym::l),
                    modifiers: Modifiers::SUPER | Modifiers::ALT,
                },
                action: Action::FocusColumnRight,
                repeat: true,
                cooldown: None,
                allow_when_locked: false,
                allow_inhibiting: true,
                hotkey_overlay_title: None,
            },
        ]);

        assert_eq!(
            find_configured_bind(
                &bindings.0,
                ModKey::Super,
                Trigger::Keysym(Keysym::q),
                ModifiersState {
                    logo: true,
                    ..Default::default()
                }
            )
            .as_ref(),
            Some(&bindings.0[0])
        );
        assert_eq!(
            find_configured_bind(
                &bindings.0,
                ModKey::Super,
                Trigger::Keysym(Keysym::q),
                ModifiersState::default(),
            ),
            None,
        );

        assert_eq!(
            find_configured_bind(
                &bindings.0,
                ModKey::Super,
                Trigger::Keysym(Keysym::h),
                ModifiersState {
                    logo: true,
                    ..Default::default()
                }
            )
            .as_ref(),
            Some(&bindings.0[1])
        );
        assert_eq!(
            find_configured_bind(
                &bindings.0,
                ModKey::Super,
                Trigger::Keysym(Keysym::h),
                ModifiersState::default(),
            ),
            None,
        );

        assert_eq!(
            find_configured_bind(
                &bindings.0,
                ModKey::Super,
                Trigger::Keysym(Keysym::j),
                ModifiersState {
                    logo: true,
                    ..Default::default()
                }
            ),
            None,
        );
        assert_eq!(
            find_configured_bind(
                &bindings.0,
                ModKey::Super,
                Trigger::Keysym(Keysym::j),
                ModifiersState::default(),
            )
            .as_ref(),
            Some(&bindings.0[2])
        );

        assert_eq!(
            find_configured_bind(
                &bindings.0,
                ModKey::Super,
                Trigger::Keysym(Keysym::k),
                ModifiersState {
                    logo: true,
                    ..Default::default()
                }
            )
            .as_ref(),
            Some(&bindings.0[3])
        );
        assert_eq!(
            find_configured_bind(
                &bindings.0,
                ModKey::Super,
                Trigger::Keysym(Keysym::k),
                ModifiersState::default(),
            ),
            None,
        );

        assert_eq!(
            find_configured_bind(
                &bindings.0,
                ModKey::Super,
                Trigger::Keysym(Keysym::l),
                ModifiersState {
                    logo: true,
                    alt: true,
                    ..Default::default()
                }
            )
            .as_ref(),
            Some(&bindings.0[4])
        );
        assert_eq!(
            find_configured_bind(
                &bindings.0,
                ModKey::Super,
                Trigger::Keysym(Keysym::l),
                ModifiersState {
                    logo: true,
                    ..Default::default()
                },
            ),
            None,
        );
    }

    #[test]
    fn keycode_bind_matches_physical_key_across_layouts() {
        // A Super+Slash bind requested by keycode: the trigger is the *physical* evdev keycode
        // (53), not the keysym.
        let bind = Bind {
            key: Key {
                trigger: Trigger::Keycode(53),
                modifiers: Modifiers::COMPOSITOR,
            },
            action: Action::CloseWindow,
            repeat: true,
            cooldown: None,
            allow_when_locked: false,
            allow_inhibiting: true,
            hotkey_overlay_title: None,
        };
        let bindings = Binds(vec![bind.clone()]);

        let comp_mod = ModKey::Super;
        let super_mods = ModifiersState {
            logo: true,
            ..Default::default()
        };

        // The physical slash key is XKB keycode 61 (evdev 53 + 8).
        let slash_key_code = Keycode::from(53u32 + 8);

        // On the US layout, the raw keysym is `slash`; a keysym bind would match.
        // Simulate the Hebrew layout where the same physical key reports a different keysym.
        let hebrew_sym = Keysym::hebrew_finalkaph;

        // With the US raw keysym, the keycode bind still matches.
        assert_eq!(
            find_bind(
                &bindings.0,
                comp_mod,
                slash_key_code,
                hebrew_sym,
                Some(Keysym::slash),
                super_mods,
                false,
            )
            .as_ref(),
            Some(&bind)
        );

        // With a non-US keysym and no matching keysym bind, the physical key still matches.
        assert_eq!(
            find_bind(
                &bindings.0,
                comp_mod,
                slash_key_code,
                hebrew_sym,
                Some(hebrew_sym),
                super_mods,
                false,
            )
            .as_ref(),
            Some(&bind)
        );
    }

    #[test]
    fn keycode_bind_takes_priority_over_colliding_keysym_bind() {
        // Super+Slash requested by keycode (physical evdev 53), and an unrelated Super+Period
        // keysym bind. On a Hebrew layout the physical slash key reports `period`; the keycode
        // bind must still win so that the physical key behaves the same on every layout.
        let keycode_bind = Bind {
            key: Key {
                trigger: Trigger::Keycode(53),
                modifiers: Modifiers::COMPOSITOR,
            },
            action: Action::CloseWindow,
            repeat: true,
            cooldown: None,
            allow_when_locked: false,
            allow_inhibiting: true,
            hotkey_overlay_title: None,
        };
        let period_bind = Bind {
            key: Key {
                trigger: Trigger::Keysym(Keysym::period),
                modifiers: Modifiers::COMPOSITOR,
            },
            action: Action::Spawn(vec!["dmc".into(), "-n".into()]),
            repeat: true,
            cooldown: None,
            allow_when_locked: false,
            allow_inhibiting: true,
            hotkey_overlay_title: None,
        };
        let bindings = Binds(vec![period_bind.clone(), keycode_bind.clone()]);

        let slash_key_code = Keycode::from(53u32 + 8);
        let found = find_bind(
            &bindings.0,
            ModKey::Super,
            slash_key_code,
            Keysym::period,
            Some(Keysym::period),
            ModifiersState {
                logo: true,
                ..Default::default()
            },
            false,
        );
        // Keycode binds have priority, so the physical key still triggers Super+Slash.
        assert_eq!(found.as_ref(), Some(&keycode_bind));

        // Sanity check: with no keycode bind, the keysym bind still matches.
        let keysym_only = Binds(vec![period_bind.clone()]);
        assert_eq!(
            find_bind(
                &keysym_only.0,
                ModKey::Super,
                slash_key_code,
                Keysym::period,
                Some(Keysym::period),
                ModifiersState {
                    logo: true,
                    ..Default::default()
                },
                false,
            )
            .as_ref(),
            Some(&period_bind)
        );
    }
}
