use neori_config_derive::LuaTable;

use crate::appearance::{BackgroundEffectRule, BlockOutFrom, CornerRadius, ShadowRule};
use crate::utils::RegexEq;
use crate::window_rule::PopupsRule;

#[derive(Debug, Default, Clone, PartialEq, LuaTable)]
pub struct LayerRule {
    #[lua(alias = "match")]
    pub matches: Vec<Match>,
    #[lua(alias = "exclude")]
    pub excludes: Vec<Match>,
    pub opacity: Option<f32>,
    pub block_out_from: Option<BlockOutFrom>,
    pub shadow: ShadowRule,
    pub geometry_corner_radius: Option<CornerRadius>,
    pub place_within_backdrop: Option<bool>,
    pub focus_on_type_in_overview: Option<bool>,
    pub baba_is_float: Option<bool>,
    pub background_effect: BackgroundEffectRule,
    pub popups: PopupsRule,
}

#[derive(Debug, Default, Clone, PartialEq, LuaTable)]
#[lua(class = "LayerMatch")]
pub struct Match {
    pub namespace: Option<RegexEq>,
    pub at_startup: Option<bool>,
    pub layer: Option<neori_ipc::Layer>,
}
