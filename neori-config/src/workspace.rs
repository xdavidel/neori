use neori_config_derive::LuaTable;

use crate::LayoutPart;

#[derive(Debug, Default, Clone, PartialEq, LuaTable)]
pub struct Workspace {
    pub name: WorkspaceName,
    /// Output names to open this workspace on, in preference order.
    pub open_on_output: Vec<String>,
    pub layout: Option<WorkspaceLayoutPart>,
}

#[derive(Debug, Default, Clone, PartialEq, Eq)]
pub struct WorkspaceName(pub String);

#[derive(Debug, Clone, PartialEq)]
pub struct WorkspaceLayoutPart(pub LayoutPart);
