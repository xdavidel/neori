use crate::LayoutPart;

#[derive(Debug, Clone, PartialEq)]
pub struct Workspace {
    pub name: WorkspaceName,
    /// Output names to open this workspace on, in preference order.
    pub open_on_output: Vec<String>,
    pub layout: Option<WorkspaceLayoutPart>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct WorkspaceName(pub String);

#[derive(Debug, Clone, PartialEq)]
pub struct WorkspaceLayoutPart(pub LayoutPart);
