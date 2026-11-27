//! Bind actions with arguments: `focus` on the move actions, workspace references and
//! `switch-layout`.

use neori_config::{Action, Config, WorkspaceReference};
use neori_ipc::LayoutSwitchTarget;

/// The actions of the binds of `binds`, in order.
#[track_caller]
fn actions(binds: &str) -> Vec<Action> {
    let config = Config::parse_mem(&format!("neori.strict(true)\n{binds}"))
        .unwrap_or_else(|e| panic!("{e:?}"));
    config
        .binds
        .0
        .iter()
        .map(|b| b.press_action().cloned().unwrap())
        .collect()
}

#[track_caller]
fn error(binds: &str) -> String {
    match Config::parse_mem(&format!("neori.strict(true)\n{binds}")) {
        Ok(_) => panic!("expected an error for {binds}"),
        Err(err) => format!("{err:?}"),
    }
}

#[test]
fn move_actions_take_focus() {
    let got = actions(
        r#"
        neori.bind("Mod+1", { move_column_to_workspace = 2 })
        neori.bind("Mod+2", { move_column_to_workspace = 2, focus = false })
        neori.bind("Mod+3", { move_column_to_workspace = { "web", focus = false } })
        neori.bind("Mod+4", { move_window_to_workspace = 3, focus = false })
        neori.bind("Mod+5", { move_column_left = true, focus = false })
        neori.bind("Mod+6", { move_window_to_workspace_down = true, focus = false })
        neori.bind("Mod+7", "move-column-right")
        "#,
    );
    assert_eq!(
        got,
        [
            Action::MoveColumnToWorkspace {
                reference: WorkspaceReference::Index(2),
                focus: true,
            },
            Action::MoveColumnToWorkspace {
                reference: WorkspaceReference::Index(2),
                focus: false,
            },
            Action::MoveColumnToWorkspace {
                reference: WorkspaceReference::Name("web".to_owned()),
                focus: false,
            },
            Action::MoveWindowToWorkspace {
                window_id: None,
                reference: WorkspaceReference::Index(3),
                focus: false,
            },
            Action::MoveColumnLeft { focus: false },
            Action::MoveWindowToWorkspaceDown { focus: false },
            Action::MoveColumnRight { focus: true },
        ]
    );
}

#[test]
fn workspace_references_are_checked() {
    // These used to become workspace 0, silently.
    assert!(error(r#"neori.bind("Mod+1", { focus_workspace = true })"#)
        .contains("focus_workspace: expected a workspace index or name, got boolean"));
    assert!(
        error(r#"neori.bind("Mod+1", { move_column_to_workspace = { focus = false } })"#)
            .contains("expected a workspace index or name, got nil")
    );
    assert!(error(r#"neori.bind("Mod+1", { focus_workspace = 300 })"#).contains("out of range"));
    assert!(
        error(r#"neori.bind("Mod+1", { move_column_left = true, focus = "no" })"#)
            .contains("focus: expected a boolean, got string")
    );
}

#[test]
fn switch_layout() {
    let got = actions(
        r#"
        neori.bind("Mod+1", "switch-layout next")
        neori.bind("Mod+2", "switch_layout prev")
        neori.bind("Mod+3", "switch-layout 1")
        neori.bind("Mod+4", { switch_layout = "next" })
        neori.bind("Mod+5", { switch_layout = 2 })
        "#,
    );
    assert_eq!(
        got,
        [
            LayoutSwitchTarget::Next,
            LayoutSwitchTarget::Prev,
            LayoutSwitchTarget::Index(1),
            LayoutSwitchTarget::Next,
            LayoutSwitchTarget::Index(2),
        ]
        .map(|layout| Action::SwitchLayout { layout })
    );
    assert!(error(r#"neori.bind("Mod+1", "switch-layout sideways")"#).contains("invalid layout"));
}
