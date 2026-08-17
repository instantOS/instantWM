mod dispatch;
mod named;

pub use dispatch::{execute_button_action, execute_key_action, try_execute_key_action};
pub use named::{NamedAction, get_action_metadata, parse_named_action};

#[derive(Debug, Clone, Copy)]
pub struct ActionMeta {
    pub name: &'static str,
    pub doc: &'static str,
    pub arg_example: Option<&'static str>,
}

#[derive(Debug, Clone)]
pub enum KeyAction {
    /// Execute multiple key actions in order.
    Sequence(Vec<KeyAction>),
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
    /// Begin the bottom-bar gesture.
    ///
    /// The swipe direction is latched once the pointer travels past a
    /// per-monitor threshold, and the matching `left`, `right`, or `up` action
    /// is dispatched exactly once on release. If no swipe latches, a short
    /// press fires `click` and a long press fires `hold` (rebindable in
    /// `get_buttons`).
    BottomBarDrag {
        left: Box<ButtonAction>,
        right: Box<ButtonAction>,
        up: Box<ButtonAction>,
        click: Box<ButtonAction>,
        hold: Box<ButtonAction>,
    },
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

    /// Render this action as a short, human-readable string (e.g.
    /// `spawn ins settings --gui`, `view_tag 4`, `sequence [...]`).
    ///
    /// This is display-only text; it is not meant to round-trip through the
    /// TOML config parser (tag actions and sequences aren't config syntax).
    /// Used by `instantwmctl keybinds` so a help menu can show what each chord
    /// does without re-parsing the internal action tree.
    pub fn describe(&self) -> String {
        match self {
            KeyAction::Named { action, args } => {
                if args.is_empty() {
                    action.name().to_string()
                } else {
                    format!("{} {}", action.name(), args.join(" "))
                }
            }
            KeyAction::Sequence(actions) => {
                let parts: Vec<String> = actions.iter().map(KeyAction::describe).collect();
                format!("sequence [{}]", parts.join(", "))
            }
            KeyAction::ViewTag { tag_idx } => format!("view_tag {}", tag_idx + 1),
            KeyAction::ToggleViewTag { tag_idx } => format!("toggle_view_tag {}", tag_idx + 1),
            KeyAction::SetClientTag { tag_idx } => format!("set_tag {}", tag_idx + 1),
            KeyAction::FollowClientTag { tag_idx } => format!("follow_tag {}", tag_idx + 1),
            KeyAction::ToggleClientTag { tag_idx } => format!("toggle_tag {}", tag_idx + 1),
            KeyAction::SwapTags { tag_idx } => format!("swap_tag {}", tag_idx + 1),
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn key_action_describe_formats_all_variants() {
        assert_eq!(KeyAction::named(NamedAction::FocusNext).describe(), "focus_next");
        assert_eq!(
            KeyAction::named_args(NamedAction::Spawn, &["ins", "settings", "--gui"]).describe(),
            "spawn ins settings --gui"
        );
        assert_eq!(
            KeyAction::Sequence(vec![
                KeyAction::named_args(NamedAction::SetMode, &["default"]),
                KeyAction::named(NamedAction::Quit),
            ])
            .describe(),
            "sequence [set_mode default, quit]"
        );
        assert_eq!(KeyAction::ViewTag { tag_idx: 0 }.describe(), "view_tag 1");
        assert_eq!(
            KeyAction::ToggleViewTag { tag_idx: 1 }.describe(),
            "toggle_view_tag 2"
        );
        assert_eq!(KeyAction::SetClientTag { tag_idx: 2 }.describe(), "set_tag 3");
        assert_eq!(
            KeyAction::FollowClientTag { tag_idx: 3 }.describe(),
            "follow_tag 4"
        );
        assert_eq!(
            KeyAction::ToggleClientTag { tag_idx: 4 }.describe(),
            "toggle_tag 5"
        );
        assert_eq!(KeyAction::SwapTags { tag_idx: 8 }.describe(), "swap_tag 9");
    }
}
