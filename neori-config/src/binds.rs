use std::str::FromStr;
use std::time::Duration;

use bitflags::bitflags;
use miette::miette;
use neori_ipc::{
    ColumnDisplay, LayoutSwitchTarget, PositionChange, SizeChange, WorkspaceReferenceArg,
};
use smithay::input::keyboard::keysyms::KEY_NoSymbol;
use smithay::input::keyboard::xkb::{
    keysym_from_name, keysym_get_name, KEYSYM_CASE_INSENSITIVE, KEYSYM_NO_FLAGS,
};
use smithay::input::keyboard::Keysym;

use crate::misc::ShakeToFindPart;
use crate::recent_windows::{MruDirection, MruFilter, MruScope};
use crate::utils::Flag;

#[derive(Debug, Default, PartialEq)]
pub struct Binds(pub Vec<Bind>);

#[derive(Debug, Clone, PartialEq)]
pub struct Bind {
    pub key: Key,
    pub action: Action,
    pub repeat: bool,
    pub cooldown: Option<Duration>,
    pub allow_when_locked: bool,
    pub allow_inhibiting: bool,
    pub hotkey_overlay_title: Option<Option<String>>,
}

#[derive(Debug, PartialEq, Eq, Clone, Copy, Hash)]
pub struct Key {
    pub trigger: Trigger,
    pub modifiers: Modifiers,
}

impl Key {
    /// Rewrites a keysym trigger into the equivalent evdev keycode so that the binding
    /// keeps matching the same physical key when the keyboard layout changes.
    ///
    /// The keysym name is looked up in the `us` layout, so this works for any key that
    /// exists on a standard US keyboard. Non-keysym triggers are returned unchanged.
    pub fn into_keycode(self) -> Result<Self, miette::Error> {
        let name = match self.trigger {
            Trigger::Keysym(keysym) => keysym_get_name(keysym),
            other => {
                return Ok(Self {
                    trigger: other,
                    modifiers: self.modifiers,
                });
            }
        };

        match self.try_into_keycode() {
            Some(key) => Ok(key),
            None => Err(miette!(
                "cannot use keycode for '{name}': no key with this name in the US layout"
            )),
        }
    }

    /// Like [`Key::into_keycode`], but returns `None` when the keysym has no key in the US
    /// layout (e.g. XF86 media keys) instead of erroring. Non-keysym triggers convert to
    /// themselves.
    pub fn try_into_keycode(self) -> Option<Self> {
        let trigger = match self.trigger {
            Trigger::Keysym(keysym) => {
                let name = keysym_get_name(keysym);
                Trigger::Keycode(crate::keycode::evdev_from_keysym_name(&name)?)
            }
            other => other,
        };

        Some(Self {
            trigger,
            modifiers: self.modifiers,
        })
    }
}

#[derive(Debug, PartialEq, Eq, Clone, Copy, Hash)]
pub enum Trigger {
    Keysym(Keysym),
    /// A Linux evdev keycode. Unlike a keysym this does not change with the active
    /// keyboard layout.
    Keycode(u32),
    MouseLeft,
    MouseRight,
    MouseMiddle,
    MouseBack,
    MouseForward,
    WheelScrollDown,
    WheelScrollUp,
    WheelScrollLeft,
    WheelScrollRight,
    TouchpadScrollDown,
    TouchpadScrollUp,
    TouchpadScrollLeft,
    TouchpadScrollRight,
    TabletStylusButton1,
    TabletStylusButton2,
    TabletStylusButton3,
}

bitflags! {
    #[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
    pub struct Modifiers : u8 {
        const CTRL = 1;
        const SHIFT = 1 << 1;
        const ALT = 1 << 2;
        const SUPER = 1 << 3;
        const ISO_LEVEL3_SHIFT = 1 << 4;
        const ISO_LEVEL5_SHIFT = 1 << 5;
        const COMPOSITOR = 1 << 6;
    }
}

#[derive(Debug, Default, Clone, PartialEq)]
pub struct SwitchBinds {
    pub lid_open: Option<SwitchAction>,
    pub lid_close: Option<SwitchAction>,
    pub tablet_mode_on: Option<SwitchAction>,
    pub tablet_mode_off: Option<SwitchAction>,
}

#[derive(Debug, Clone, PartialEq)]
pub struct SwitchAction {
    pub spawn: Vec<String>,
}

// Remember to add new actions to the CLI enum too.
#[derive(Debug, Clone, PartialEq)]
pub enum Action {
    Quit(bool),
    ChangeVt(i32),
    Suspend,
    PowerOffMonitors,
    PowerOnMonitors,
    ToggleDebugTint,
    DebugToggleOpaqueRegions,
    DebugToggleDamage,
    Spawn(Vec<String>),
    SpawnSh(String),
    DoScreenTransition(Option<u16>),
    ConfirmScreenshot {
        write_to_disk: bool,
    },
    CancelScreenshot,
    ScreenshotTogglePointer,
    Screenshot(bool, Option<String>),
    ScreenshotScreen(bool, bool, Option<String>),
    ScreenshotWindow(bool, bool, Option<String>),
    ScreenshotWindowById {
        id: u64,
        write_to_disk: bool,
        show_pointer: bool,
        path: Option<String>,
    },
    ToggleKeyboardShortcutsInhibit,
    CloseWindow,
    CloseWindowById(u64),
    FullscreenWindow,
    FullscreenWindowById(u64),
    ToggleWindowedFullscreen,
    ToggleWindowedFullscreenById(u64),
    FocusWindow(u64),
    FocusWindowInColumn(u8),
    FocusWindowPrevious,
    FocusColumnLeft,
    FocusColumnLeftUnderMouse,
    FocusColumnRight,
    FocusColumnRightUnderMouse,
    FocusColumnFirst,
    FocusColumnLast,
    FocusColumnRightOrFirst,
    FocusColumnLeftOrLast,
    FocusColumn(usize),
    FocusWindowOrMonitorUp,
    FocusWindowOrMonitorDown,
    FocusColumnOrMonitorLeft,
    FocusColumnOrMonitorRight,
    FocusWindowDown,
    FocusWindowUp,
    FocusWindowDownOrColumnLeft,
    FocusWindowDownOrColumnRight,
    FocusWindowUpOrColumnLeft,
    FocusWindowUpOrColumnRight,
    FocusWindowOrWorkspaceDown,
    FocusWindowOrWorkspaceUp,
    FocusWindowTop,
    FocusWindowBottom,
    FocusWindowDownOrTop,
    FocusWindowUpOrBottom,
    MoveColumnLeft,
    MoveColumnRight,
    MoveColumnToFirst,
    MoveColumnToLast,
    MoveColumnLeftOrToMonitorLeft,
    MoveColumnRightOrToMonitorRight,
    MoveColumnToIndex(usize),
    MoveWindowDown,
    MoveWindowUp,
    MoveWindowDownOrToWorkspaceDown,
    MoveWindowUpOrToWorkspaceUp,
    ConsumeOrExpelWindowLeft,
    ConsumeOrExpelWindowLeftById(u64),
    ConsumeOrExpelWindowRight,
    ConsumeOrExpelWindowRightById(u64),
    ConsumeWindowIntoColumn,
    ExpelWindowFromColumn,
    SwapWindowLeft,
    SwapWindowRight,
    ToggleColumnTabbedDisplay,
    SetColumnDisplay(ColumnDisplay),
    CenterColumn,
    CenterWindow,
    CenterWindowById(u64),
    CenterVisibleColumns,
    FocusWorkspaceDown,
    FocusWorkspaceDownUnderMouse,
    FocusWorkspaceUp,
    FocusWorkspaceUpUnderMouse,
    FocusWorkspace(WorkspaceReference),
    FocusWorkspacePrevious,
    MoveWindowToWorkspaceDown(bool),
    MoveWindowToWorkspaceUp(bool),
    MoveWindowToWorkspace(WorkspaceReference, bool),
    MoveWindowToWorkspaceById {
        window_id: u64,
        reference: WorkspaceReference,
        focus: bool,
    },
    MoveColumnToWorkspaceDown(bool),
    MoveColumnToWorkspaceUp(bool),
    MoveColumnToWorkspace(WorkspaceReference, bool),
    MoveWorkspaceDown,
    MoveWorkspaceUp,
    MoveWorkspaceToIndex(usize),
    MoveWorkspaceToIndexByRef {
        new_idx: usize,
        reference: WorkspaceReference,
    },
    MoveWorkspaceToMonitorByRef {
        output_name: String,
        reference: WorkspaceReference,
    },
    MoveWorkspaceToMonitor(String),
    SetWorkspaceName(String),
    SetWorkspaceNameByRef {
        name: String,
        reference: WorkspaceReference,
    },
    UnsetWorkspaceName,
    UnsetWorkSpaceNameByRef(WorkspaceReference),
    FocusMonitorLeft,
    FocusMonitorRight,
    FocusMonitorDown,
    FocusMonitorUp,
    FocusMonitorPrevious,
    FocusMonitorNext,
    FocusMonitor(String),
    MoveWindowToMonitorLeft,
    MoveWindowToMonitorRight,
    MoveWindowToMonitorDown,
    MoveWindowToMonitorUp,
    MoveWindowToMonitorPrevious,
    MoveWindowToMonitorNext,
    MoveWindowToMonitor(String),
    MoveWindowToMonitorById {
        id: u64,
        output: String,
    },
    MoveColumnToMonitorLeft,
    MoveColumnToMonitorRight,
    MoveColumnToMonitorDown,
    MoveColumnToMonitorUp,
    MoveColumnToMonitorPrevious,
    MoveColumnToMonitorNext,
    MoveColumnToMonitor(String),
    SetWindowWidth(SizeChange),
    SetWindowWidthById {
        id: u64,
        change: SizeChange,
    },
    SetWindowHeight(SizeChange),
    SetWindowHeightById {
        id: u64,
        change: SizeChange,
    },
    ResetWindowHeight,
    ResetWindowHeightById(u64),
    SwitchPresetColumnWidth,
    SwitchPresetColumnWidthBack,
    SwitchPresetWindowWidth,
    SwitchPresetWindowWidthBack,
    SwitchPresetWindowWidthById(u64),
    SwitchPresetWindowWidthBackById(u64),
    SwitchPresetWindowHeight,
    SwitchPresetWindowHeightBack,
    SwitchPresetWindowHeightById(u64),
    SwitchPresetWindowHeightBackById(u64),
    MaximizeColumn,
    MaximizeWindowToEdges,
    MaximizeWindowToEdgesById(u64),
    SetColumnWidth(SizeChange),
    ExpandColumnToAvailableWidth,
    SwitchLayout(LayoutSwitchTarget),
    ShowHotkeyOverlay,
    MoveWorkspaceToMonitorLeft,
    MoveWorkspaceToMonitorRight,
    MoveWorkspaceToMonitorDown,
    MoveWorkspaceToMonitorUp,
    MoveWorkspaceToMonitorPrevious,
    MoveWorkspaceToMonitorNext,
    ToggleWindowFloating,
    ToggleWindowFloatingById(u64),
    MoveWindowToFloating,
    MoveWindowToFloatingById(u64),
    MoveWindowToTiling,
    MoveWindowToTilingById(u64),
    FocusFloating,
    FocusTiling,
    SwitchFocusBetweenFloatingAndTiling,
    MoveFloatingWindowById {
        id: Option<u64>,
        x: PositionChange,
        y: PositionChange,
    },
    ToggleWindowRuleOpacity,
    ToggleWindowRuleOpacityById(u64),
    SetDynamicCastWindow,
    SetDynamicCastWindowById(u64),
    SetDynamicCastMonitor(Option<String>),
    ClearDynamicCastTarget,
    StopCast(u64),
    ToggleOverview,
    OpenOverview,
    CloseOverview,
    EnlargeCursor,
    SetShakeToFind(ShakeToFindPart),
    ToggleWindowUrgent(u64),
    SetWindowUrgent(u64),
    UnsetWindowUrgent(u64),
    SendToScratchpad,
    RemoveFromScratchpad,
    ToggleScratchpad,
    LoadConfigFile(Option<String>),
    MruAdvance {
        direction: MruDirection,
        scope: Option<MruScope>,
        filter: Option<MruFilter>,
    },
    MruConfirm,
    MruCancel,
    MruCloseCurrentWindow,
    MruFirst,
    MruLast,
    MruSetScope(MruScope),
    MruCycleScope,
}

impl From<neori_ipc::Action> for Action {
    fn from(value: neori_ipc::Action) -> Self {
        match value {
            neori_ipc::Action::Quit { skip_confirmation } => Self::Quit(skip_confirmation),
            neori_ipc::Action::PowerOffMonitors {} => Self::PowerOffMonitors,
            neori_ipc::Action::PowerOnMonitors {} => Self::PowerOnMonitors,
            neori_ipc::Action::Spawn { command } => Self::Spawn(command),
            neori_ipc::Action::SpawnSh { command } => Self::SpawnSh(command),
            neori_ipc::Action::DoScreenTransition { delay_ms } => {
                Self::DoScreenTransition(delay_ms)
            }
            neori_ipc::Action::Screenshot { show_pointer, path } => {
                Self::Screenshot(show_pointer, path)
            }
            neori_ipc::Action::ScreenshotScreen {
                write_to_disk,
                show_pointer,
                path,
            } => Self::ScreenshotScreen(write_to_disk, show_pointer, path),
            neori_ipc::Action::ScreenshotWindow {
                id: None,
                write_to_disk,
                show_pointer,
                path,
            } => Self::ScreenshotWindow(write_to_disk, show_pointer, path),
            neori_ipc::Action::ScreenshotWindow {
                id: Some(id),
                write_to_disk,
                show_pointer,
                path,
            } => Self::ScreenshotWindowById {
                id,
                write_to_disk,
                show_pointer,
                path,
            },
            neori_ipc::Action::ToggleKeyboardShortcutsInhibit {} => {
                Self::ToggleKeyboardShortcutsInhibit
            }
            neori_ipc::Action::CloseWindow { id: None } => Self::CloseWindow,
            neori_ipc::Action::CloseWindow { id: Some(id) } => Self::CloseWindowById(id),
            neori_ipc::Action::FullscreenWindow { id: None } => Self::FullscreenWindow,
            neori_ipc::Action::FullscreenWindow { id: Some(id) } => Self::FullscreenWindowById(id),
            neori_ipc::Action::ToggleWindowedFullscreen { id: None } => {
                Self::ToggleWindowedFullscreen
            }
            neori_ipc::Action::ToggleWindowedFullscreen { id: Some(id) } => {
                Self::ToggleWindowedFullscreenById(id)
            }
            neori_ipc::Action::FocusWindow { id } => Self::FocusWindow(id),
            neori_ipc::Action::FocusWindowInColumn { index } => Self::FocusWindowInColumn(index),
            neori_ipc::Action::FocusWindowPrevious {} => Self::FocusWindowPrevious,
            neori_ipc::Action::FocusColumnLeft {} => Self::FocusColumnLeft,
            neori_ipc::Action::FocusColumnRight {} => Self::FocusColumnRight,
            neori_ipc::Action::FocusColumnFirst {} => Self::FocusColumnFirst,
            neori_ipc::Action::FocusColumnLast {} => Self::FocusColumnLast,
            neori_ipc::Action::FocusColumnRightOrFirst {} => Self::FocusColumnRightOrFirst,
            neori_ipc::Action::FocusColumnLeftOrLast {} => Self::FocusColumnLeftOrLast,
            neori_ipc::Action::FocusColumn { index } => Self::FocusColumn(index),
            neori_ipc::Action::FocusWindowOrMonitorUp {} => Self::FocusWindowOrMonitorUp,
            neori_ipc::Action::FocusWindowOrMonitorDown {} => Self::FocusWindowOrMonitorDown,
            neori_ipc::Action::FocusColumnOrMonitorLeft {} => Self::FocusColumnOrMonitorLeft,
            neori_ipc::Action::FocusColumnOrMonitorRight {} => Self::FocusColumnOrMonitorRight,
            neori_ipc::Action::FocusWindowDown {} => Self::FocusWindowDown,
            neori_ipc::Action::FocusWindowUp {} => Self::FocusWindowUp,
            neori_ipc::Action::FocusWindowDownOrColumnLeft {} => Self::FocusWindowDownOrColumnLeft,
            neori_ipc::Action::FocusWindowDownOrColumnRight {} => {
                Self::FocusWindowDownOrColumnRight
            }
            neori_ipc::Action::FocusWindowUpOrColumnLeft {} => Self::FocusWindowUpOrColumnLeft,
            neori_ipc::Action::FocusWindowUpOrColumnRight {} => Self::FocusWindowUpOrColumnRight,
            neori_ipc::Action::FocusWindowOrWorkspaceDown {} => Self::FocusWindowOrWorkspaceDown,
            neori_ipc::Action::FocusWindowOrWorkspaceUp {} => Self::FocusWindowOrWorkspaceUp,
            neori_ipc::Action::FocusWindowTop {} => Self::FocusWindowTop,
            neori_ipc::Action::FocusWindowBottom {} => Self::FocusWindowBottom,
            neori_ipc::Action::FocusWindowDownOrTop {} => Self::FocusWindowDownOrTop,
            neori_ipc::Action::FocusWindowUpOrBottom {} => Self::FocusWindowUpOrBottom,
            neori_ipc::Action::MoveColumnLeft {} => Self::MoveColumnLeft,
            neori_ipc::Action::MoveColumnRight {} => Self::MoveColumnRight,
            neori_ipc::Action::MoveColumnToFirst {} => Self::MoveColumnToFirst,
            neori_ipc::Action::MoveColumnToLast {} => Self::MoveColumnToLast,
            neori_ipc::Action::MoveColumnToIndex { index } => Self::MoveColumnToIndex(index),
            neori_ipc::Action::MoveColumnLeftOrToMonitorLeft {} => {
                Self::MoveColumnLeftOrToMonitorLeft
            }
            neori_ipc::Action::MoveColumnRightOrToMonitorRight {} => {
                Self::MoveColumnRightOrToMonitorRight
            }
            neori_ipc::Action::MoveWindowDown {} => Self::MoveWindowDown,
            neori_ipc::Action::MoveWindowUp {} => Self::MoveWindowUp,
            neori_ipc::Action::MoveWindowDownOrToWorkspaceDown {} => {
                Self::MoveWindowDownOrToWorkspaceDown
            }
            neori_ipc::Action::MoveWindowUpOrToWorkspaceUp {} => Self::MoveWindowUpOrToWorkspaceUp,
            neori_ipc::Action::ConsumeOrExpelWindowLeft { id: None } => {
                Self::ConsumeOrExpelWindowLeft
            }
            neori_ipc::Action::ConsumeOrExpelWindowLeft { id: Some(id) } => {
                Self::ConsumeOrExpelWindowLeftById(id)
            }
            neori_ipc::Action::ConsumeOrExpelWindowRight { id: None } => {
                Self::ConsumeOrExpelWindowRight
            }
            neori_ipc::Action::ConsumeOrExpelWindowRight { id: Some(id) } => {
                Self::ConsumeOrExpelWindowRightById(id)
            }
            neori_ipc::Action::ConsumeWindowIntoColumn {} => Self::ConsumeWindowIntoColumn,
            neori_ipc::Action::ExpelWindowFromColumn {} => Self::ExpelWindowFromColumn,
            neori_ipc::Action::SwapWindowRight {} => Self::SwapWindowRight,
            neori_ipc::Action::SwapWindowLeft {} => Self::SwapWindowLeft,
            neori_ipc::Action::ToggleColumnTabbedDisplay {} => Self::ToggleColumnTabbedDisplay,
            neori_ipc::Action::SetColumnDisplay { display } => Self::SetColumnDisplay(display),
            neori_ipc::Action::CenterColumn {} => Self::CenterColumn,
            neori_ipc::Action::CenterWindow { id: None } => Self::CenterWindow,
            neori_ipc::Action::CenterWindow { id: Some(id) } => Self::CenterWindowById(id),
            neori_ipc::Action::CenterVisibleColumns {} => Self::CenterVisibleColumns,
            neori_ipc::Action::FocusWorkspaceDown {} => Self::FocusWorkspaceDown,
            neori_ipc::Action::FocusWorkspaceUp {} => Self::FocusWorkspaceUp,
            neori_ipc::Action::FocusWorkspace { reference } => {
                Self::FocusWorkspace(WorkspaceReference::from(reference))
            }
            neori_ipc::Action::FocusWorkspacePrevious {} => Self::FocusWorkspacePrevious,
            neori_ipc::Action::MoveWindowToWorkspaceDown { focus } => {
                Self::MoveWindowToWorkspaceDown(focus)
            }
            neori_ipc::Action::MoveWindowToWorkspaceUp { focus } => {
                Self::MoveWindowToWorkspaceUp(focus)
            }
            neori_ipc::Action::MoveWindowToWorkspace {
                window_id: None,
                reference,
                focus,
            } => Self::MoveWindowToWorkspace(WorkspaceReference::from(reference), focus),
            neori_ipc::Action::MoveWindowToWorkspace {
                window_id: Some(window_id),
                reference,
                focus,
            } => Self::MoveWindowToWorkspaceById {
                window_id,
                reference: WorkspaceReference::from(reference),
                focus,
            },
            neori_ipc::Action::MoveColumnToWorkspaceDown { focus } => {
                Self::MoveColumnToWorkspaceDown(focus)
            }
            neori_ipc::Action::MoveColumnToWorkspaceUp { focus } => {
                Self::MoveColumnToWorkspaceUp(focus)
            }
            neori_ipc::Action::MoveColumnToWorkspace { reference, focus } => {
                Self::MoveColumnToWorkspace(WorkspaceReference::from(reference), focus)
            }
            neori_ipc::Action::MoveWorkspaceDown {} => Self::MoveWorkspaceDown,
            neori_ipc::Action::MoveWorkspaceUp {} => Self::MoveWorkspaceUp,
            neori_ipc::Action::SetWorkspaceName {
                name,
                workspace: None,
            } => Self::SetWorkspaceName(name),
            neori_ipc::Action::SetWorkspaceName {
                name,
                workspace: Some(reference),
            } => Self::SetWorkspaceNameByRef {
                name,
                reference: WorkspaceReference::from(reference),
            },
            neori_ipc::Action::UnsetWorkspaceName { reference: None } => Self::UnsetWorkspaceName,
            neori_ipc::Action::UnsetWorkspaceName {
                reference: Some(reference),
            } => Self::UnsetWorkSpaceNameByRef(WorkspaceReference::from(reference)),
            neori_ipc::Action::FocusMonitorLeft {} => Self::FocusMonitorLeft,
            neori_ipc::Action::FocusMonitorRight {} => Self::FocusMonitorRight,
            neori_ipc::Action::FocusMonitorDown {} => Self::FocusMonitorDown,
            neori_ipc::Action::FocusMonitorUp {} => Self::FocusMonitorUp,
            neori_ipc::Action::FocusMonitorPrevious {} => Self::FocusMonitorPrevious,
            neori_ipc::Action::FocusMonitorNext {} => Self::FocusMonitorNext,
            neori_ipc::Action::FocusMonitor { output } => Self::FocusMonitor(output),
            neori_ipc::Action::MoveWindowToMonitorLeft {} => Self::MoveWindowToMonitorLeft,
            neori_ipc::Action::MoveWindowToMonitorRight {} => Self::MoveWindowToMonitorRight,
            neori_ipc::Action::MoveWindowToMonitorDown {} => Self::MoveWindowToMonitorDown,
            neori_ipc::Action::MoveWindowToMonitorUp {} => Self::MoveWindowToMonitorUp,
            neori_ipc::Action::MoveWindowToMonitorPrevious {} => Self::MoveWindowToMonitorPrevious,
            neori_ipc::Action::MoveWindowToMonitorNext {} => Self::MoveWindowToMonitorNext,
            neori_ipc::Action::MoveWindowToMonitor { id: None, output } => {
                Self::MoveWindowToMonitor(output)
            }
            neori_ipc::Action::MoveWindowToMonitor {
                id: Some(id),
                output,
            } => Self::MoveWindowToMonitorById { id, output },
            neori_ipc::Action::MoveColumnToMonitorLeft {} => Self::MoveColumnToMonitorLeft,
            neori_ipc::Action::MoveColumnToMonitorRight {} => Self::MoveColumnToMonitorRight,
            neori_ipc::Action::MoveColumnToMonitorDown {} => Self::MoveColumnToMonitorDown,
            neori_ipc::Action::MoveColumnToMonitorUp {} => Self::MoveColumnToMonitorUp,
            neori_ipc::Action::MoveColumnToMonitorPrevious {} => Self::MoveColumnToMonitorPrevious,
            neori_ipc::Action::MoveColumnToMonitorNext {} => Self::MoveColumnToMonitorNext,
            neori_ipc::Action::MoveColumnToMonitor { output } => Self::MoveColumnToMonitor(output),
            neori_ipc::Action::SetWindowWidth { id: None, change } => Self::SetWindowWidth(change),
            neori_ipc::Action::SetWindowWidth {
                id: Some(id),
                change,
            } => Self::SetWindowWidthById { id, change },
            neori_ipc::Action::SetWindowHeight { id: None, change } => {
                Self::SetWindowHeight(change)
            }
            neori_ipc::Action::SetWindowHeight {
                id: Some(id),
                change,
            } => Self::SetWindowHeightById { id, change },
            neori_ipc::Action::ResetWindowHeight { id: None } => Self::ResetWindowHeight,
            neori_ipc::Action::ResetWindowHeight { id: Some(id) } => {
                Self::ResetWindowHeightById(id)
            }
            neori_ipc::Action::SwitchPresetColumnWidth {} => Self::SwitchPresetColumnWidth,
            neori_ipc::Action::SwitchPresetColumnWidthBack {} => Self::SwitchPresetColumnWidthBack,
            neori_ipc::Action::SwitchPresetWindowWidth { id: None } => {
                Self::SwitchPresetWindowWidth
            }
            neori_ipc::Action::SwitchPresetWindowWidthBack { id: None } => {
                Self::SwitchPresetWindowWidthBack
            }
            neori_ipc::Action::SwitchPresetWindowWidth { id: Some(id) } => {
                Self::SwitchPresetWindowWidthById(id)
            }
            neori_ipc::Action::SwitchPresetWindowWidthBack { id: Some(id) } => {
                Self::SwitchPresetWindowWidthBackById(id)
            }
            neori_ipc::Action::SwitchPresetWindowHeight { id: None } => {
                Self::SwitchPresetWindowHeight
            }
            neori_ipc::Action::SwitchPresetWindowHeightBack { id: None } => {
                Self::SwitchPresetWindowHeightBack
            }
            neori_ipc::Action::SwitchPresetWindowHeight { id: Some(id) } => {
                Self::SwitchPresetWindowHeightById(id)
            }
            neori_ipc::Action::SwitchPresetWindowHeightBack { id: Some(id) } => {
                Self::SwitchPresetWindowHeightBackById(id)
            }
            neori_ipc::Action::MaximizeColumn {} => Self::MaximizeColumn,
            neori_ipc::Action::MaximizeWindowToEdges { id: None } => Self::MaximizeWindowToEdges,
            neori_ipc::Action::MaximizeWindowToEdges { id: Some(id) } => {
                Self::MaximizeWindowToEdgesById(id)
            }
            neori_ipc::Action::SetColumnWidth { change } => Self::SetColumnWidth(change),
            neori_ipc::Action::ExpandColumnToAvailableWidth {} => {
                Self::ExpandColumnToAvailableWidth
            }
            neori_ipc::Action::SwitchLayout { layout } => Self::SwitchLayout(layout),
            neori_ipc::Action::ShowHotkeyOverlay {} => Self::ShowHotkeyOverlay,
            neori_ipc::Action::MoveWorkspaceToMonitorLeft {} => Self::MoveWorkspaceToMonitorLeft,
            neori_ipc::Action::MoveWorkspaceToMonitorRight {} => Self::MoveWorkspaceToMonitorRight,
            neori_ipc::Action::MoveWorkspaceToMonitorDown {} => Self::MoveWorkspaceToMonitorDown,
            neori_ipc::Action::MoveWorkspaceToMonitorUp {} => Self::MoveWorkspaceToMonitorUp,
            neori_ipc::Action::MoveWorkspaceToMonitorPrevious {} => {
                Self::MoveWorkspaceToMonitorPrevious
            }
            neori_ipc::Action::MoveWorkspaceToIndex {
                index,
                reference: Some(reference),
            } => Self::MoveWorkspaceToIndexByRef {
                new_idx: index,
                reference: WorkspaceReference::from(reference),
            },
            neori_ipc::Action::MoveWorkspaceToIndex {
                index,
                reference: None,
            } => Self::MoveWorkspaceToIndex(index),
            neori_ipc::Action::MoveWorkspaceToMonitor {
                output,
                reference: Some(reference),
            } => Self::MoveWorkspaceToMonitorByRef {
                output_name: output,
                reference: WorkspaceReference::from(reference),
            },
            neori_ipc::Action::MoveWorkspaceToMonitor {
                output,
                reference: None,
            } => Self::MoveWorkspaceToMonitor(output),
            neori_ipc::Action::MoveWorkspaceToMonitorNext {} => Self::MoveWorkspaceToMonitorNext,
            neori_ipc::Action::ToggleDebugTint {} => Self::ToggleDebugTint,
            neori_ipc::Action::DebugToggleOpaqueRegions {} => Self::DebugToggleOpaqueRegions,
            neori_ipc::Action::DebugToggleDamage {} => Self::DebugToggleDamage,
            neori_ipc::Action::ToggleWindowFloating { id: None } => Self::ToggleWindowFloating,
            neori_ipc::Action::ToggleWindowFloating { id: Some(id) } => {
                Self::ToggleWindowFloatingById(id)
            }
            neori_ipc::Action::MoveWindowToFloating { id: None } => Self::MoveWindowToFloating,
            neori_ipc::Action::MoveWindowToFloating { id: Some(id) } => {
                Self::MoveWindowToFloatingById(id)
            }
            neori_ipc::Action::MoveWindowToTiling { id: None } => Self::MoveWindowToTiling,
            neori_ipc::Action::MoveWindowToTiling { id: Some(id) } => {
                Self::MoveWindowToTilingById(id)
            }
            neori_ipc::Action::FocusFloating {} => Self::FocusFloating,
            neori_ipc::Action::FocusTiling {} => Self::FocusTiling,
            neori_ipc::Action::SwitchFocusBetweenFloatingAndTiling {} => {
                Self::SwitchFocusBetweenFloatingAndTiling
            }
            neori_ipc::Action::MoveFloatingWindow { id, x, y } => {
                Self::MoveFloatingWindowById { id, x, y }
            }
            neori_ipc::Action::ToggleWindowRuleOpacity { id: None } => {
                Self::ToggleWindowRuleOpacity
            }
            neori_ipc::Action::ToggleWindowRuleOpacity { id: Some(id) } => {
                Self::ToggleWindowRuleOpacityById(id)
            }
            neori_ipc::Action::SetDynamicCastWindow { id: None } => Self::SetDynamicCastWindow,
            neori_ipc::Action::SetDynamicCastWindow { id: Some(id) } => {
                Self::SetDynamicCastWindowById(id)
            }
            neori_ipc::Action::SetDynamicCastMonitor { output } => {
                Self::SetDynamicCastMonitor(output)
            }
            neori_ipc::Action::ClearDynamicCastTarget {} => Self::ClearDynamicCastTarget,
            neori_ipc::Action::StopCast { session_id } => Self::StopCast(session_id),
            neori_ipc::Action::ToggleOverview {} => Self::ToggleOverview,
            neori_ipc::Action::OpenOverview {} => Self::OpenOverview,
            neori_ipc::Action::CloseOverview {} => Self::CloseOverview,
            neori_ipc::Action::EnlargeCursor {} => Self::EnlargeCursor,
            neori_ipc::Action::SetShakeToFind { settings } => {
                Self::SetShakeToFind(ShakeToFindPart {
                    enable: settings.enable.map(Flag),
                    size_multiplier: settings.size_multiplier,
                    size: settings.size,
                    timeout_ms: settings.timeout_ms,
                    animation_ms: settings.animation_ms,
                    window_ms: settings.window_ms,
                    min_reversals: settings.min_reversals,
                    min_step: settings.min_step,
                    reversal_angle_deg: settings.reversal_angle_deg,
                })
            }
            neori_ipc::Action::SendToScratchpad {} => Self::SendToScratchpad,
            neori_ipc::Action::RemoveFromScratchpad {} => Self::RemoveFromScratchpad,
            neori_ipc::Action::ToggleScratchpad {} => Self::ToggleScratchpad,
            neori_ipc::Action::ToggleWindowUrgent { id } => Self::ToggleWindowUrgent(id),
            neori_ipc::Action::SetWindowUrgent { id } => Self::SetWindowUrgent(id),
            neori_ipc::Action::UnsetWindowUrgent { id } => Self::UnsetWindowUrgent(id),
            neori_ipc::Action::LoadConfigFile { path } => Self::LoadConfigFile(path),
        }
    }
}

#[derive(Debug, PartialEq, Eq, Clone)]
pub enum WorkspaceReference {
    Id(u64),
    Index(u8),
    Name(String),
}

impl From<WorkspaceReferenceArg> for WorkspaceReference {
    fn from(reference: WorkspaceReferenceArg) -> WorkspaceReference {
        match reference {
            WorkspaceReferenceArg::Id(id) => Self::Id(id),
            WorkspaceReferenceArg::Index(i) => Self::Index(i),
            WorkspaceReferenceArg::Name(n) => Self::Name(n),
        }
    }
}

impl FromStr for Key {
    type Err = miette::Error;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        let mut modifiers = Modifiers::empty();

        let mut split = s.split('+');
        let key = split.next_back().unwrap();

        for part in split {
            let part = part.trim();
            if part.eq_ignore_ascii_case("mod") {
                modifiers |= Modifiers::COMPOSITOR
            } else if part.eq_ignore_ascii_case("ctrl") || part.eq_ignore_ascii_case("control") {
                modifiers |= Modifiers::CTRL;
            } else if part.eq_ignore_ascii_case("shift") {
                modifiers |= Modifiers::SHIFT;
            } else if part.eq_ignore_ascii_case("alt") {
                modifiers |= Modifiers::ALT;
            } else if part.eq_ignore_ascii_case("super") || part.eq_ignore_ascii_case("win") {
                modifiers |= Modifiers::SUPER;
            } else if part.eq_ignore_ascii_case("iso_level3_shift")
                || part.eq_ignore_ascii_case("mod5")
            {
                modifiers |= Modifiers::ISO_LEVEL3_SHIFT;
            } else if part.eq_ignore_ascii_case("iso_level5_shift")
                || part.eq_ignore_ascii_case("mod3")
            {
                modifiers |= Modifiers::ISO_LEVEL5_SHIFT;
            } else {
                return Err(miette!("invalid modifier: {part}"));
            }
        }

        let trigger = if key.eq_ignore_ascii_case("MouseLeft") {
            Trigger::MouseLeft
        } else if key.eq_ignore_ascii_case("MouseRight") {
            Trigger::MouseRight
        } else if key.eq_ignore_ascii_case("MouseMiddle") {
            Trigger::MouseMiddle
        } else if key.eq_ignore_ascii_case("MouseBack") {
            Trigger::MouseBack
        } else if key.eq_ignore_ascii_case("MouseForward") {
            Trigger::MouseForward
        } else if key.eq_ignore_ascii_case("WheelScrollDown") {
            Trigger::WheelScrollDown
        } else if key.eq_ignore_ascii_case("WheelScrollUp") {
            Trigger::WheelScrollUp
        } else if key.eq_ignore_ascii_case("WheelScrollLeft") {
            Trigger::WheelScrollLeft
        } else if key.eq_ignore_ascii_case("WheelScrollRight") {
            Trigger::WheelScrollRight
        } else if key.eq_ignore_ascii_case("TouchpadScrollDown") {
            Trigger::TouchpadScrollDown
        } else if key.eq_ignore_ascii_case("TouchpadScrollUp") {
            Trigger::TouchpadScrollUp
        } else if key.eq_ignore_ascii_case("TouchpadScrollLeft") {
            Trigger::TouchpadScrollLeft
        } else if key.eq_ignore_ascii_case("TouchpadScrollRight") {
            Trigger::TouchpadScrollRight
        } else if key.eq_ignore_ascii_case("TabletStylusButton1") {
            Trigger::TabletStylusButton1
        } else if key.eq_ignore_ascii_case("TabletStylusButton2") {
            Trigger::TabletStylusButton2
        } else if key.eq_ignore_ascii_case("TabletStylusButton3") {
            Trigger::TabletStylusButton3
        } else {
            let mut keysym = keysym_from_name(key, KEYSYM_CASE_INSENSITIVE);
            if keysym == Keysym::XF86_Screensaver {
                keysym = keysym_from_name(key, KEYSYM_NO_FLAGS);
                if keysym.raw() == KEY_NoSymbol {
                    keysym = Keysym::XF86_ScreenSaver;
                }
            }
            if keysym.raw() == KEY_NoSymbol {
                return Err(miette!("invalid key: {key}"));
            }
            Trigger::Keysym(keysym)
        };

        Ok(Key { trigger, modifiers })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parse_xf86_screensaver() {
        assert_eq!(
            "XF86ScreenSaver".parse::<Key>().unwrap(),
            Key {
                trigger: Trigger::Keysym(Keysym::XF86_ScreenSaver),
                modifiers: Modifiers::empty(),
            },
        );
        assert_eq!(
            "XF86Screensaver".parse::<Key>().unwrap(),
            Key {
                trigger: Trigger::Keysym(Keysym::XF86_Screensaver),
                modifiers: Modifiers::empty(),
            }
        );
        assert_eq!(
            "xf86screensaver".parse::<Key>().unwrap(),
            Key {
                trigger: Trigger::Keysym(Keysym::XF86_ScreenSaver),
                modifiers: Modifiers::empty(),
            }
        );
    }

    #[test]
    fn parse_iso_level_shifts() {
        assert_eq!(
            "ISO_Level3_Shift+A".parse::<Key>().unwrap(),
            Key {
                trigger: Trigger::Keysym(Keysym::a),
                modifiers: Modifiers::ISO_LEVEL3_SHIFT
            },
        );
        assert_eq!(
            "Mod5+A".parse::<Key>().unwrap(),
            Key {
                trigger: Trigger::Keysym(Keysym::a),
                modifiers: Modifiers::ISO_LEVEL3_SHIFT
            },
        );

        assert_eq!(
            "ISO_Level5_Shift+A".parse::<Key>().unwrap(),
            Key {
                trigger: Trigger::Keysym(Keysym::a),
                modifiers: Modifiers::ISO_LEVEL5_SHIFT
            },
        );
        assert_eq!(
            "Mod3+A".parse::<Key>().unwrap(),
            Key {
                trigger: Trigger::Keysym(Keysym::a),
                modifiers: Modifiers::ISO_LEVEL5_SHIFT
            },
        );
    }
}
