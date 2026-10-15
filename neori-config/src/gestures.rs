use crate::FloatOrInt;

#[derive(Debug, Default, Clone, Copy, PartialEq)]
pub struct Gestures {
    pub dnd_edge_view_scroll: DndEdgeViewScroll,
    pub dnd_edge_workspace_switch: DndEdgeWorkspaceSwitch,
    pub hot_corners: HotCorners,
}

#[allow(dead_code)]
#[derive(Debug, Default, Clone, Copy, PartialEq)]
pub struct GesturesPart {
    pub dnd_edge_view_scroll: Option<DndEdgeViewScrollPart>,
    pub dnd_edge_workspace_switch: Option<DndEdgeWorkspaceSwitchPart>,
    pub hot_corners: Option<HotCorners>,
}

#[derive(Debug, Clone, Copy, PartialEq)]
pub struct DndEdgeViewScroll {
    pub trigger_width: f64,
    pub delay_ms: u16,
    pub max_speed: f64,
}

impl Default for DndEdgeViewScroll {
    fn default() -> Self {
        Self {
            trigger_width: 30.,
            delay_ms: 100,
            max_speed: 1500.,
        }
    }
}

#[allow(dead_code)]
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct DndEdgeViewScrollPart {
    pub trigger_width: Option<FloatOrInt<0, 65535>>,
    pub delay_ms: Option<u16>,
    pub max_speed: Option<FloatOrInt<0, 1_000_000>>,
}

#[derive(Debug, Clone, Copy, PartialEq)]
pub struct DndEdgeWorkspaceSwitch {
    pub trigger_height: f64,
    pub delay_ms: u16,
    pub max_speed: f64,
}

impl Default for DndEdgeWorkspaceSwitch {
    fn default() -> Self {
        Self {
            trigger_height: 50.,
            delay_ms: 100,
            max_speed: 1500.,
        }
    }
}

#[allow(dead_code)]
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct DndEdgeWorkspaceSwitchPart {
    pub trigger_height: Option<FloatOrInt<0, 65535>>,
    pub delay_ms: Option<u16>,
    pub max_speed: Option<FloatOrInt<0, 1_000_000>>,
}

#[derive(Debug, Default, Clone, Copy, PartialEq)]
pub struct HotCorners {
    pub off: bool,
    pub top_left: bool,
    pub top_right: bool,
    pub bottom_left: bool,
    pub bottom_right: bool,
}
