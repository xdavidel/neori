use neori_ipc::{ColumnDisplay, SizeChange};

use crate::appearance::{
    Border, FocusRing, InsertHint, Shadow, TabIndicator, DEFAULT_BACKGROUND_COLOR,
};
use crate::utils::MergeWith;
use crate::{BorderRule, Color, FloatOrInt, InsertHintPart, ShadowRule, TabIndicatorPart};

#[derive(Debug, Clone, PartialEq)]
pub struct Layout {
    pub focus_ring: FocusRing,
    pub border: Border,
    pub shadow: Shadow,
    pub tab_indicator: TabIndicator,
    pub insert_hint: InsertHint,
    pub preset_column_widths: Vec<PresetSize>,
    pub default_column_width: Option<PresetSize>,
    pub preset_window_heights: Vec<PresetSize>,
    pub center_focused_column: CenterFocusedColumn,
    pub always_center_single_column: bool,
    pub empty_workspace_above_first: bool,
    pub default_column_display: ColumnDisplay,
    pub gaps: f64,
    pub struts: Struts,
    pub background_color: Color,
    pub fullscreen_backdrop_color: Color,
}

impl Default for Layout {
    fn default() -> Self {
        Self {
            focus_ring: FocusRing::default(),
            border: Border::default(),
            shadow: Shadow::default(),
            tab_indicator: TabIndicator::default(),
            insert_hint: InsertHint::default(),
            preset_column_widths: vec![
                PresetSize::Proportion(1. / 3.),
                PresetSize::Proportion(0.5),
                PresetSize::Proportion(2. / 3.),
            ],
            default_column_width: Some(PresetSize::Proportion(0.5)),
            center_focused_column: CenterFocusedColumn::Never,
            always_center_single_column: false,
            empty_workspace_above_first: false,
            default_column_display: ColumnDisplay::Normal,
            gaps: 16.,
            struts: Struts::default(),
            preset_window_heights: vec![
                PresetSize::Proportion(1. / 3.),
                PresetSize::Proportion(0.5),
                PresetSize::Proportion(2. / 3.),
            ],
            background_color: DEFAULT_BACKGROUND_COLOR,
            fullscreen_backdrop_color: Color::from_array_unpremul([0., 0., 0., 1.]),
        }
    }
}

#[allow(dead_code)]
#[derive(Debug, Default, Clone, PartialEq)]
pub struct LayoutPart {
    pub focus_ring: Option<BorderRule>,
    pub border: Option<BorderRule>,
    pub shadow: Option<ShadowRule>,
    pub tab_indicator: Option<TabIndicatorPart>,
    pub insert_hint: Option<InsertHintPart>,
    pub preset_column_widths: Option<Vec<PresetSize>>,
    pub default_column_width: Option<DefaultPresetSize>,
    pub preset_window_heights: Option<Vec<PresetSize>>,
    pub center_focused_column: Option<CenterFocusedColumn>,
    pub always_center_single_column: Option<bool>,
    pub empty_workspace_above_first: Option<bool>,
    pub default_column_display: Option<ColumnDisplay>,
    pub gaps: Option<FloatOrInt<0, 65535>>,
    pub struts: Option<Struts>,
    pub background_color: Option<Color>,
    pub fullscreen_backdrop_color: Option<Color>,
}

impl MergeWith<LayoutPart> for Layout {
    fn merge_with(&mut self, part: &LayoutPart) {
        if let Some(x) = &part.focus_ring {
            self.focus_ring.merge_with(x);
        }
        if let Some(x) = &part.border {
            self.border.merge_with(x);
        }
        if let Some(x) = &part.shadow {
            self.shadow.merge_with(x);
        }
        if let Some(x) = &part.tab_indicator {
            self.tab_indicator.merge_with(x);
        }
        if let Some(x) = &part.insert_hint {
            self.insert_hint.merge_with(x);
        }
        if let Some(x) = &part.preset_column_widths {
            self.preset_column_widths.clone_from(x);
        }
        if let Some(x) = &part.default_column_width {
            self.default_column_width = x.0;
        }
        if let Some(x) = &part.preset_window_heights {
            self.preset_window_heights.clone_from(x);
        }
        if let Some(x) = &part.center_focused_column {
            self.center_focused_column = *x;
        }
        if let Some(x) = &part.always_center_single_column {
            self.always_center_single_column = *x;
        }
        if let Some(x) = &part.empty_workspace_above_first {
            self.empty_workspace_above_first = *x;
        }
        if let Some(x) = &part.default_column_display {
            self.default_column_display = *x;
        }
        if let Some(x) = &part.gaps {
            self.gaps = x.0;
        }
        if let Some(x) = &part.struts {
            self.struts = *x;
        }
        if let Some(x) = &part.background_color {
            self.background_color = *x;
        }
        if let Some(x) = &part.fullscreen_backdrop_color {
            self.fullscreen_backdrop_color = *x;
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq)]
pub enum PresetSize {
    Proportion(f64),
    Fixed(i32),
}

impl From<PresetSize> for SizeChange {
    fn from(value: PresetSize) -> Self {
        match value {
            PresetSize::Proportion(prop) => SizeChange::SetProportion(prop * 100.),
            PresetSize::Fixed(fixed) => SizeChange::SetFixed(fixed),
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq)]
pub struct DefaultPresetSize(pub Option<PresetSize>);

#[derive(Debug, Default, Clone, Copy, PartialEq)]
pub struct Struts {
    pub left: FloatOrInt<-65535, 65535>,
    pub right: FloatOrInt<-65535, 65535>,
    pub top: FloatOrInt<-65535, 65535>,
    pub bottom: FloatOrInt<-65535, 65535>,
}

#[derive(Debug, Default, PartialEq, Eq, Clone, Copy)]
pub enum CenterFocusedColumn {
    #[default]
    Never,
    Always,
    OnOverflow,
}
