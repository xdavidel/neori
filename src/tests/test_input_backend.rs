use smithay::backend::input::{
    Axis, AxisRelativeDirection, AxisSource, ButtonState, Device, DeviceCapability, Event,
    GestureBeginEvent, GestureEndEvent, GestureHoldBeginEvent, GestureHoldEndEvent,
    GesturePinchBeginEvent, GesturePinchEndEvent, GesturePinchUpdateEvent, GestureSwipeBeginEvent,
    GestureSwipeEndEvent, GestureSwipeUpdateEvent, InputBackend, InputTime, KeyState,
    KeyboardKeyEvent, Keycode, PointerAxisEvent, PointerButtonEvent, UnusedEvent,
};
use smithay::output::Output;

use crate::input::backend_ext::NeoriInputDevice;

pub struct TestInputBackend;

#[derive(PartialEq, Eq, Hash)]
pub struct TestInputDevice;

pub struct TestKeyboardKeyEvent {
    pub time: InputTime,
    pub code: Keycode,
    pub state: KeyState,
    pub count: u32,
}

impl InputBackend for TestInputBackend {
    type Device = TestInputDevice;

    type KeyboardKeyEvent = TestKeyboardKeyEvent;
    type PointerAxisEvent = TestPointerAxisEvent;
    type PointerButtonEvent = TestPointerButtonEvent;
    type PointerMotionEvent = UnusedEvent;
    type PointerMotionAbsoluteEvent = UnusedEvent;

    type GestureSwipeBeginEvent = TestGestureEvent;
    type GestureSwipeUpdateEvent = TestGestureEvent;
    type GestureSwipeEndEvent = TestGestureEvent;
    type GesturePinchBeginEvent = TestGestureEvent;
    type GesturePinchUpdateEvent = TestGestureEvent;
    type GesturePinchEndEvent = TestGestureEvent;
    type GestureHoldBeginEvent = TestGestureEvent;
    type GestureHoldEndEvent = TestGestureEvent;

    type TouchDownEvent = UnusedEvent;
    type TouchUpEvent = UnusedEvent;
    type TouchMotionEvent = UnusedEvent;
    type TouchCancelEvent = UnusedEvent;
    type TouchFrameEvent = UnusedEvent;
    type TabletToolAxisEvent = UnusedEvent;
    type TabletToolProximityEvent = UnusedEvent;
    type TabletToolTipEvent = UnusedEvent;
    type TabletToolButtonEvent = UnusedEvent;

    type SwitchToggleEvent = UnusedEvent;

    type SpecialEvent = UnusedEvent;
}

impl Device for TestInputDevice {
    fn id(&self) -> String {
        String::from("test")
    }

    fn name(&self) -> String {
        String::from("test input device")
    }

    fn has_capability(&self, capability: DeviceCapability) -> bool {
        matches!(capability, DeviceCapability::Keyboard)
    }

    fn usb_id(&self) -> Option<(u32, u32)> {
        None
    }

    fn syspath(&self) -> Option<std::path::PathBuf> {
        None
    }
}

impl NeoriInputDevice for TestInputDevice {
    fn output(&self, _state: &crate::neori::State) -> Option<Output> {
        None
    }
}

impl Event<TestInputBackend> for TestKeyboardKeyEvent {
    fn time(&self) -> InputTime {
        self.time
    }

    fn device(&self) -> <TestInputBackend as InputBackend>::Device {
        TestInputDevice
    }
}

impl KeyboardKeyEvent<TestInputBackend> for TestKeyboardKeyEvent {
    fn key_code(&self) -> Keycode {
        self.code
    }

    fn state(&self) -> KeyState {
        self.state
    }

    fn count(&self) -> u32 {
        self.count
    }
}

pub struct TestPointerButtonEvent {
    pub code: u32,
    pub state: ButtonState,
}

impl Event<TestInputBackend> for TestPointerButtonEvent {
    fn time(&self) -> InputTime {
        InputTime::now()
    }

    fn device(&self) -> TestInputDevice {
        TestInputDevice
    }
}

impl PointerButtonEvent<TestInputBackend> for TestPointerButtonEvent {
    fn button_code(&self) -> u32 {
        self.code
    }

    fn state(&self) -> ButtonState {
        self.state
    }
}

/// A wheel scroll by `v120` steps on each axis.
pub struct TestPointerAxisEvent {
    pub horizontal_v120: f64,
    pub vertical_v120: f64,
}

impl Event<TestInputBackend> for TestPointerAxisEvent {
    fn time(&self) -> InputTime {
        InputTime::now()
    }

    fn device(&self) -> TestInputDevice {
        TestInputDevice
    }
}

impl PointerAxisEvent<TestInputBackend> for TestPointerAxisEvent {
    fn amount(&self, axis: Axis) -> Option<f64> {
        let v120 = self.amount_v120(axis)?;
        Some(v120 / 120. * 15.)
    }

    fn amount_v120(&self, axis: Axis) -> Option<f64> {
        Some(match axis {
            Axis::Horizontal => self.horizontal_v120,
            Axis::Vertical => self.vertical_v120,
        })
    }

    fn source(&self) -> AxisSource {
        AxisSource::Wheel
    }

    fn relative_direction(&self, _axis: Axis) -> AxisRelativeDirection {
        AxisRelativeDirection::Identical
    }
}

/// Any touchpad gesture event: each kind reads the fields it has.
#[derive(Default)]
pub struct TestGestureEvent {
    pub fingers: u32,
    pub dx: f64,
    pub dy: f64,
    pub scale: f64,
    pub rotation: f64,
    pub cancelled: bool,
}

impl Event<TestInputBackend> for TestGestureEvent {
    fn time(&self) -> InputTime {
        InputTime::now()
    }

    fn device(&self) -> TestInputDevice {
        TestInputDevice
    }
}

impl GestureBeginEvent<TestInputBackend> for TestGestureEvent {
    fn fingers(&self) -> u32 {
        self.fingers
    }
}

impl GestureEndEvent<TestInputBackend> for TestGestureEvent {
    fn cancelled(&self) -> bool {
        self.cancelled
    }
}

impl GestureSwipeBeginEvent<TestInputBackend> for TestGestureEvent {}
impl GestureSwipeEndEvent<TestInputBackend> for TestGestureEvent {}
impl GesturePinchBeginEvent<TestInputBackend> for TestGestureEvent {}
impl GesturePinchEndEvent<TestInputBackend> for TestGestureEvent {}
impl GestureHoldBeginEvent<TestInputBackend> for TestGestureEvent {}
impl GestureHoldEndEvent<TestInputBackend> for TestGestureEvent {}

impl GestureSwipeUpdateEvent<TestInputBackend> for TestGestureEvent {
    fn delta_x(&self) -> f64 {
        self.dx
    }

    fn delta_y(&self) -> f64 {
        self.dy
    }
}

impl GesturePinchUpdateEvent<TestInputBackend> for TestGestureEvent {
    fn delta_x(&self) -> f64 {
        self.dx
    }

    fn delta_y(&self) -> f64 {
        self.dy
    }

    fn scale(&self) -> f64 {
        self.scale
    }

    fn rotation(&self) -> f64 {
        self.rotation
    }
}
