mod dispatch;
mod named;

pub use dispatch::{execute_button_action, execute_key_action};
pub use named::{NamedAction, get_action_metadata, parse_named_action};

#[derive(Debug, Clone, Copy)]
pub struct ActionMeta {
    pub name: &'static str,
    pub doc: &'static str,
    pub arg_example: Option<&'static str>,
}

#[derive(Debug, Clone)]
pub enum KeyAction {
    Named {
        action: NamedAction,
        args: Vec<String>,
    },
    ViewTag {
        tag_idx: usize,
    },
    ToggleViewTag {
        tag_idx: usize,
    },
    SetClientTag {
        tag_idx: usize,
    },
    FollowClientTag {
        tag_idx: usize,
    },
    ToggleClientTag {
        tag_idx: usize,
    },
    SwapTags {
        tag_idx: usize,
    },
}

#[derive(Debug, Clone)]
pub enum ButtonAction {
    Named {
        action: NamedAction,
        args: Vec<String>,
    },
    WindowTitleMouseHandler,
    CloseClickedTitleWindow,
    DragTagBegin,
    ToggleClickedViewTag,
    SetSelectedClientClickedTag,
    ToggleSelectedClientClickedTag,
    ClientMoveDrag,
    ResizeSelectedAspect,
    KillSelectedClient,
    ToggleLockSelectedClient,
    SidebarGestureBegin,
    ReorderSelected {
        direction: crate::types::StackDirection,
    },
    ScaleSelected {
        percent: i32,
    },
    HideEdgeScratchpad,
    ShowEdgeScratchpad,
    ToggleFloatingSelected,
    ResizeMouseFromCursor,
}

pub fn argv(args: &[&str]) -> Vec<String> {
    args.iter().map(|s| (*s).to_string()).collect()
}

impl KeyAction {
    pub fn named(action: NamedAction) -> Self {
        Self::Named {
            action,
            args: Vec::new(),
        }
    }

    pub fn named_args(action: NamedAction, args: &[&str]) -> Self {
        Self::Named {
            action,
            args: argv(args),
        }
    }
}

impl ButtonAction {
    pub fn named(action: NamedAction) -> Self {
        Self::Named {
            action,
            args: Vec::new(),
        }
    }

    pub fn named_args(action: NamedAction, args: &[&str]) -> Self {
        Self::Named {
            action,
            args: argv(args),
        }
    }
}
