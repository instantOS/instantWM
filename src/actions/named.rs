use crate::actions::ActionInfo;
use crate::client::fullscreen::toggle_fake_fullscreen;
use crate::client::{kill_client, shut_kill, zoom};
use crate::config::ModeConfig;
use crate::contexts::WmCtx;
use crate::floating::scratchpad::DEFAULT_SCRATCHPAD_NAME;
use crate::floating::{
    DEFAULT_EDGE_SCRATCHPAD_NAME, center_window, distribute_clients, edge_scratchpad_create,
    key_move, key_resize, scratchpad_create, scratchpad_hide_name, scratchpad_restore,
    scratchpad_show_name, scratchpad_toggle, set_scratchpad_direction, toggle_floating,
};
use crate::focus::{direction_focus, focus_last_client, focus_stack, focus_stack_neighbor};
use crate::ipc_types::ScratchpadInitialStatus;
use crate::keyboard::alt_tab_key;
use crate::layouts::tree::Side;
use crate::layouts::{
    LayoutCommand, MaximizedStackReorder, begin_tree_placement, center_keyboard_tree_placement,
    cycle_keyboard_tree_placement, cycle_layout_direction, finish_keyboard_tree_placement,
    focus_tree_neighbor, inc_master_count_by, reorder_maximized_stack, reset_active_layout,
    resize_keyboard_tree_placement, resize_tree, resize_tree_smart, set_layout,
    step_keyboard_tree_placement, swap_keyboard_tree_placement, swap_tree_neighbor,
    toggle_floating_presentation, toggle_tiling_maximized,
};
use crate::monitor::{focus_monitor, move_to_monitor_and_follow};
use crate::mouse::draw_window;
use crate::overview::ActionTransition;
use crate::tags::{
    cancel_overview, follow_view, last_view, move_client_follow_view, send_to_monitor, shift_tag,
    shift_view, toggle_overview, win_view,
};
use crate::toggles::{
    toggle_alt_tag, toggle_bar, toggle_hide_tags, toggle_mode, toggle_sticky, unhide_all,
};
use crate::types::{
    EdgeDirection, FocusFollowsMouseMode, HorizontalDirection, MonitorDirection, StackDirection,
    TagMask, TagSelection, ToggleAction, VerticalDirection,
};
use crate::util::spawn;
use std::collections::HashMap;

/// Typed arguments of a named action, parsed once from their textual form
/// (config arrays, `instantwmctl action NAME ARGS...`) and rendered back for
/// display and IPC.
pub(crate) trait ActionArgs: Sized {
    fn parse(args: &[String]) -> Result<Self, String>;
    fn render(&self) -> Vec<String>;
    fn usage() -> String;
}

/// One textual argument value.
trait ArgValue: Sized {
    fn parse_value(value: &str) -> Result<Self, String>;
    fn render_value(&self) -> String;
    fn usage() -> String;
}

macro_rules! value_enum_arg {
    ($($ty:ty),+) => {$(
        impl ArgValue for $ty {
            fn parse_value(value: &str) -> Result<Self, String> {
                <$ty as clap::ValueEnum>::from_str(value, true).map_err(|_| {
                    format!("invalid value '{value}'; expected {}", <Self as ArgValue>::usage())
                })
            }

            fn render_value(&self) -> String {
                clap::ValueEnum::to_possible_value(self)
                    .expect("no skipped variants")
                    .get_name()
                    .to_string()
            }

            fn usage() -> String {
                <$ty as clap::ValueEnum>::value_variants()
                    .iter()
                    .filter_map(clap::ValueEnum::to_possible_value)
                    .map(|value| value.get_name().to_string())
                    .collect::<Vec<_>>()
                    .join("|")
            }
        }
    )+};
}

value_enum_arg!(
    ToggleAction,
    MonitorDirection,
    StackDirection,
    FocusFollowsMouseMode
);

impl ArgValue for String {
    fn parse_value(value: &str) -> Result<Self, String> {
        Ok(value.to_string())
    }

    fn render_value(&self) -> String {
        self.clone()
    }

    fn usage() -> String {
        "NAME".to_string()
    }
}

impl ArgValue for i32 {
    fn parse_value(value: &str) -> Result<Self, String> {
        value
            .parse()
            .map_err(|_| format!("invalid value '{value}'; expected an integer"))
    }

    fn render_value(&self) -> String {
        self.to_string()
    }

    fn usage() -> String {
        "N".to_string()
    }
}

impl ArgValue for u32 {
    fn parse_value(value: &str) -> Result<Self, String> {
        value
            .parse()
            .map_err(|_| format!("invalid value '{value}'; expected a non-negative integer"))
    }

    fn render_value(&self) -> String {
        self.to_string()
    }

    fn usage() -> String {
        "N".to_string()
    }
}

impl ArgValue for LayoutCommand {
    fn parse_value(value: &str) -> Result<Self, String> {
        LayoutCommand::from_name(value).ok_or_else(|| {
            format!(
                "invalid layout '{value}'; expected {}",
                <Self as ArgValue>::usage()
            )
        })
    }

    fn render_value(&self) -> String {
        self.name().to_string()
    }

    fn usage() -> String {
        LayoutCommand::all()
            .iter()
            .map(|layout| layout.name())
            .collect::<Vec<_>>()
            .join("|")
    }
}

macro_rules! single_value_args {
    ($($ty:ty),+) => {$(
        impl ActionArgs for $ty {
            fn parse(args: &[String]) -> Result<Self, String> {
                match args {
                    [value] => Self::parse_value(value),
                    _ => Err(format!("expected 1 argument, got {}", args.len())),
                }
            }

            fn render(&self) -> Vec<String> {
                vec![self.render_value()]
            }

            fn usage() -> String {
                <Self as ArgValue>::usage()
            }
        }
    )+};
}

single_value_args!(
    String,
    u32,
    LayoutCommand,
    MonitorDirection,
    StackDirection,
    FocusFollowsMouseMode
);

impl<T: ArgValue> ActionArgs for Option<T> {
    fn parse(args: &[String]) -> Result<Self, String> {
        match args {
            [] => Ok(None),
            [value] => T::parse_value(value).map(Some),
            _ => Err(format!("expected at most 1 argument, got {}", args.len())),
        }
    }

    fn render(&self) -> Vec<String> {
        self.iter().map(ArgValue::render_value).collect()
    }

    fn usage() -> String {
        format!("[{}]", T::usage())
    }
}

/// A command line: program followed by its arguments.
impl ActionArgs for Vec<String> {
    fn parse(args: &[String]) -> Result<Self, String> {
        if args.is_empty() {
            return Err("expected a command".to_string());
        }
        Ok(args.to_vec())
    }

    fn render(&self) -> Vec<String> {
        self.clone()
    }

    fn usage() -> String {
        "COMMAND [ARG ...]".to_string()
    }
}

/// Declares every named action once: its name, overview policy (default:
/// confirm, so a new mutating action never operates on the overview
/// projection), documentation, typed argument and behaviour.
macro_rules! define_named_actions {
    (@transition) => { ActionTransition::Confirm };
    (@transition $transition:ident) => { ActionTransition::$transition };
    (@usage) => { None };
    (@usage $arg:ty) => { Some(<$arg as ActionArgs>::usage()) };
    (@render) => { Vec::new() };
    (@render $value:ident) => { ActionArgs::render($value) };
    (@parse $name:literal, $variant:ident, $args:ident) => {
        if $args.is_empty() {
            Self::$variant
        } else {
            return Err(format!("action '{}' takes no arguments", $name));
        }
    };
    (@parse $name:literal, $variant:ident, $args:ident, $arg:ty) => {
        Self::$variant(
            <$arg as ActionArgs>::parse($args)
                .map_err(|error| format!("action '{}': {error}", $name))?,
        )
    };
    ($(
        $variant:ident $(($arg:ty))? => {
            name: $name:literal,
            $(overview: $transition:ident,)?
            doc: $doc:literal,
            run: |$ctx:ident $(, $value:ident)?| $body:block
        }
    ),+ $(,)?) => {
        #[derive(Debug, Clone, PartialEq)]
        pub enum NamedAction {
            $($variant $(($arg))?,)+
        }

        impl NamedAction {
            pub const fn name(&self) -> &'static str {
                match self {
                    $(Self::$variant { .. } => $name,)+
                }
            }

            /// Parse an action from its name and textual arguments.
            pub fn parse(name: &str, args: &[String]) -> Result<Self, String> {
                Ok(match name {
                    $($name => define_named_actions!(@parse $name, $variant, args $(, $arg)?),)+
                    _ => return Err(format!("unknown action '{name}'")),
                })
            }

            /// The textual arguments [`Self::parse`] accepts for this action.
            pub fn args(&self) -> Vec<String> {
                match self {
                    $(Self::$variant $(($value))? => define_named_actions!(@render $($value)?),)+
                }
            }

            pub(crate) fn overview_transition(&self) -> ActionTransition {
                match self {
                    $(Self::$variant { .. } => define_named_actions!(@transition $($transition)?),)+
                }
            }

            pub(crate) fn execute(&self, ctx: &mut WmCtx<'_>) -> Result<(), String> {
                crate::overview::prepare_named_action(ctx, self);
                match self {
                    $(Self::$variant $(($value))? => {
                        let $ctx = &mut *ctx;
                        $body
                    })+
                }
                Ok(())
            }
        }

        /// Every named action with its documentation, sorted by name.
        pub fn action_infos() -> Vec<ActionInfo> {
            let mut infos = vec![
                $(ActionInfo {
                    name: $name,
                    description: $doc,
                    arg_example: define_named_actions!(@usage $($arg)?),
                }),+
            ];
            infos.sort_by_key(|info| info.name);
            infos
        }
    };
}

impl NamedAction {
    /// Human-readable form, e.g. `spawn ins settings --gui`.
    pub fn describe(&self) -> String {
        std::iter::once(self.name().to_string())
            .chain(self.args())
            .collect::<Vec<_>>()
            .join(" ")
    }
}

impl From<HorizontalDirection> for Side {
    fn from(direction: HorizontalDirection) -> Self {
        match direction {
            HorizontalDirection::Left => Side::Left,
            HorizontalDirection::Right => Side::Right,
        }
    }
}

impl From<VerticalDirection> for Side {
    fn from(direction: VerticalDirection) -> Self {
        match direction {
            VerticalDirection::Up => Side::Top,
            VerticalDirection::Down => Side::Bottom,
        }
    }
}

fn validate_mode_name(
    configured_modes: &HashMap<String, ModeConfig>,
    name: &str,
) -> Result<(), String> {
    if name == crate::core_state::TREE_PLACEMENT_MODE_NAME {
        return Err("mode 'placement' can only be entered by begin_tree_placement".to_string());
    }
    if configured_modes.contains_key(name)
        || matches!(
            crate::core_state::ActiveWmMode::from_name(name),
            crate::core_state::ActiveWmMode::Default | crate::core_state::ActiveWmMode::Overview
        )
    {
        Ok(())
    } else {
        Err(format!("mode '{name}' not found"))
    }
}

fn focus_horizontal(ctx: &mut WmCtx<'_>, direction: HorizontalDirection) {
    if ctx.core().model().is_overview_active() {
        crate::overview::focus_direction(ctx, direction.into());
        return;
    }
    if ctx
        .core()
        .model()
        .expect_selected_monitor()
        .is_maximized_layout()
    {
        if !focus_stack_neighbor(ctx, direction.into()) {
            crate::animation::scroll_view_with_slide(ctx, direction);
        }
        return;
    }

    if !focus_tree_neighbor(ctx, direction.into()) && !direction_focus(ctx, direction.into()) {
        crate::animation::scroll_view_with_slide(ctx, direction);
    }
}

fn focus_vertical(ctx: &mut WmCtx<'_>, direction: VerticalDirection) {
    if ctx.core().model().is_overview_active() {
        crate::overview::focus_direction(ctx, direction.into());
        return;
    }
    let maximized = ctx
        .core()
        .model()
        .expect_selected_monitor()
        .is_maximized_layout();
    if maximized
        || (!focus_tree_neighbor(ctx, direction.into()) && !direction_focus(ctx, direction.into()))
    {
        focus_stack(ctx, direction.into());
    }
}

fn move_horizontal(ctx: &mut WmCtx<'_>, direction: HorizontalDirection) {
    match reorder_maximized_stack(ctx, direction.into()) {
        MaximizedStackReorder::Reordered | MaximizedStackReorder::ReconcileRequired => return,
        MaximizedStackReorder::Boundary => {
            let _ = move_client_follow_view(ctx, direction);
            return;
        }
        MaximizedStackReorder::NotApplicable => {}
    }

    if swap_tree_neighbor(ctx, direction.into()) {
        return;
    }
    let Some(win) = ctx.core().model().selected_win() else {
        return;
    };
    if !key_move(ctx, win, direction.into()) {
        let _ = move_client_follow_view(ctx, direction);
    }
}

fn move_vertical(ctx: &mut WmCtx<'_>, direction: VerticalDirection) {
    if !matches!(
        reorder_maximized_stack(ctx, direction.into()),
        MaximizedStackReorder::NotApplicable
    ) {
        return;
    }

    if !swap_tree_neighbor(ctx, direction.into())
        && let Some(win) = ctx.core().model().selected_win()
    {
        key_move(ctx, win, direction.into());
    }
}

fn key_resize_or_tree(ctx: &mut WmCtx<'_>, side: Side, direction: crate::types::Direction) {
    if !resize_tree(ctx, side)
        && let Some(win) = ctx.core().model().selected_win()
    {
        key_resize(ctx, win, direction);
    }
}

/// Logical pixels added per unconfigured gap key press. Bindings can pass an
/// explicit integer argument to use a different step.
const DEFAULT_GAP_STEP: i32 = 2;

/// Move both tiling gaps by `delta` logical pixels and re-arrange.
///
/// Inner and outer gaps move together because users think of "the gap size"
/// as one knob; per-axis values stay reachable through config and IPC. Both
/// clamp at zero: placement treats zero gaps as disabled, so decreasing at
/// the floor simply keeps gapless tiling instead of inverting windows.
fn adjust_gaps(ctx: &mut WmCtx<'_>, delta: i32) {
    let layout = &mut ctx.core_mut().config_mut().layout;
    layout.inner_gap = layout.inner_gap.saturating_add(delta).max(0);
    layout.outer_gap = layout.outer_gap.saturating_add(delta).max(0);
    crate::layouts::manager::arrange(ctx, None);
}

fn with_selected_win(ctx: &mut WmCtx<'_>, f: impl FnOnce(&mut WmCtx<'_>, crate::types::WindowId)) {
    if let Some(win) = ctx.core().model().selected_win() {
        f(ctx, win);
    }
}

fn edge_scratchpad_set_direction(ctx: &mut WmCtx, dir: EdgeDirection) {
    if let Some(win) = ctx
        .core()
        .model()
        .scratchpad_find(DEFAULT_EDGE_SCRATCHPAD_NAME)
    {
        set_scratchpad_direction(ctx, win, dir);
    }
}

define_named_actions!(
    Zoom => { name: "zoom", doc: "zoom client into master area", run: |ctx| { zoom(ctx); } },
    Kill => { name: "kill", doc: "close focused window gracefully", run: |ctx| { with_selected_win(ctx, kill_client); } },
    ShutKill => { name: "shut_kill", doc: "force kill focused window", run: |ctx| { shut_kill(ctx); } },
    Quit => { name: "quit", doc: "quit instantwm", run: |ctx| { ctx.core_mut().quit(); } },
    FocusNext => { name: "focus_next", overview: Preserve, doc: "focus next window in stack", run: |ctx| { focus_stack(ctx, StackDirection::Next); } },
    FocusPrev => { name: "focus_prev", overview: Preserve, doc: "focus previous window in stack", run: |ctx| { focus_stack(ctx, StackDirection::Previous); } },
    FocusLast => { name: "focus_last", overview: Cancel, doc: "focus last focused window", run: |ctx| { focus_last_client(ctx); } },
    FocusUp => { name: "focus_up", overview: Preserve, doc: "focus above; cycle backward in bar order when no window is above", run: |ctx| { focus_vertical(ctx, VerticalDirection::Up); } },
    FocusDown => { name: "focus_down", overview: Preserve, doc: "focus below; cycle forward in bar order when no window is below", run: |ctx| { focus_vertical(ctx, VerticalDirection::Down); } },
    FocusLeft => { name: "focus_left", overview: Preserve, doc: "focus left, or move backward through bar order in maximized presentation; switch tags at the boundary", run: |ctx| { focus_horizontal(ctx, HorizontalDirection::Left); } },
    FocusRight => { name: "focus_right", overview: Preserve, doc: "focus right, or move forward through bar order in maximized presentation; switch tags at the boundary", run: |ctx| { focus_horizontal(ctx, HorizontalDirection::Right); } },
    DownKey => { name: "down_key", overview: Preserve, doc: "alt-tab forward", run: |ctx| { alt_tab_key(ctx, VerticalDirection::Down); } },
    UpKey => { name: "up_key", overview: Preserve, doc: "alt-tab backward", run: |ctx| { alt_tab_key(ctx, VerticalDirection::Up); } },
    LayoutFloat => { name: "layout_float", doc: "toggle floating layout presentation without changing per-window floating state", run: |ctx| { toggle_floating_presentation(ctx); } },
    ToggleTilingMaximized => { name: "toggle_tiling_maximized", doc: "toggle maximized-stack presentation, or restore manual tiling from floating layout", run: |ctx| { toggle_tiling_maximized(ctx); } },
    CycleLayoutNext => { name: "cycle_layout_next", doc: "cycle to next layout", run: |ctx| { cycle_layout_direction(ctx, true); } },
    CycleLayoutPrev => { name: "cycle_layout_prev", doc: "cycle to previous layout", run: |ctx| { cycle_layout_direction(ctx, false); } },
    IncMasterCount(Option<i32>) => { name: "inc_master_count", doc: "change the master window count (default +1)", run: |ctx, delta| { inc_master_count_by(ctx, delta.unwrap_or(1)); } },
    IncGaps(Option<i32>) => { name: "inc_gaps", doc: "increase tiled inner and outer gaps", run: |ctx, delta| { adjust_gaps(ctx, delta.unwrap_or(DEFAULT_GAP_STEP)); } },
    DecGaps(Option<i32>) => { name: "dec_gaps", doc: "decrease tiled inner and outer gaps", run: |ctx, delta| { adjust_gaps(ctx, -delta.unwrap_or(DEFAULT_GAP_STEP)); } },
    CenterWindow => { name: "center_window", doc: "center focused window", run: |ctx| { with_selected_win(ctx, center_window); } },
    DistributeClients => { name: "distribute_clients", doc: "distribute windows evenly", run: |ctx| { distribute_clients(ctx); } },
    KeyResizeUp => { name: "key_resize_up", doc: "grow a tiled window vertically or resize a floating window", run: |ctx| { key_resize_or_tree(ctx, Side::Top, VerticalDirection::Up.into()); } },
    KeyResizeDown => { name: "key_resize_down", doc: "shrink a tiled window vertically or resize a floating window", run: |ctx| { key_resize_or_tree(ctx, Side::Bottom, VerticalDirection::Down.into()); } },
    KeyResizeLeft => { name: "key_resize_left", doc: "shrink a tiled window horizontally or resize a floating window", run: |ctx| { key_resize_or_tree(ctx, Side::Left, HorizontalDirection::Left.into()); } },
    KeyResizeRight => { name: "key_resize_right", doc: "grow a tiled window horizontally or resize a floating window", run: |ctx| { key_resize_or_tree(ctx, Side::Right, HorizontalDirection::Right.into()); } },
    KeyMoveUp => { name: "key_move_up", doc: "move toward the previous maximized title, swap a tiled window upward, or move a floating window", run: |ctx| { move_vertical(ctx, VerticalDirection::Up); } },
    KeyMoveDown => { name: "key_move_down", doc: "move toward the next maximized title, swap a tiled window downward, or move a floating window", run: |ctx| { move_vertical(ctx, VerticalDirection::Down); } },
    KeyMoveLeft => { name: "key_move_left", doc: "move toward the previous maximized title or move left, carrying the window to the adjacent tag at the boundary", run: |ctx| { move_horizontal(ctx, HorizontalDirection::Left); } },
    KeyMoveRight => { name: "key_move_right", doc: "move toward the next maximized title or move right, carrying the window to the adjacent tag at the boundary", run: |ctx| { move_horizontal(ctx, HorizontalDirection::Right); } },
    TreeGrow => { name: "tree_grow", doc: "grow the focused window along its most local split", run: |ctx| { resize_tree_smart(ctx, true); } },
    TreeShrink => { name: "tree_shrink", doc: "shrink the focused window along its most local split", run: |ctx| { resize_tree_smart(ctx, false); } },
    PushUp => { name: "push_up", doc: "swap a tiled window upward (legacy action)", run: |ctx| { swap_tree_neighbor(ctx, Side::Top); } },
    PushDown => { name: "push_down", doc: "swap a tiled window downward (legacy action)", run: |ctx| { swap_tree_neighbor(ctx, Side::Bottom); } },
    LastView => { name: "last_view", overview: Cancel, doc: "view previously viewed tags", run: |ctx| { last_view(ctx); } },
    FollowView => { name: "follow_view", doc: "follow client to its tags", run: |ctx| { follow_view(ctx); } },
    WinView => { name: "win_view", doc: "view tags of focused client", run: |ctx| { win_view(ctx); } },
    ScrollLeft => { name: "scroll_left", overview: Cancel, doc: "scroll tags left", run: |ctx| { crate::animation::scroll_view_with_slide(ctx, HorizontalDirection::Left); } },
    ScrollRight => { name: "scroll_right", overview: Cancel, doc: "scroll tags right", run: |ctx| { crate::animation::scroll_view_with_slide(ctx, HorizontalDirection::Right); } },
    MoveClientLeft => { name: "move_client_left", doc: "move client to tag on left", run: |ctx| { move_client_follow_view(ctx, HorizontalDirection::Left); } },
    MoveClientRight => { name: "move_client_right", doc: "move client to tag on right", run: |ctx| { move_client_follow_view(ctx, HorizontalDirection::Right); } },
    ShiftTagLeft => { name: "shift_tag_left", doc: "shift client to tag on left", run: |ctx| { shift_tag(ctx, HorizontalDirection::Left); } },
    ShiftTagRight => { name: "shift_tag_right", doc: "shift client to tag on right", run: |ctx| { shift_tag(ctx, HorizontalDirection::Right); } },
    ShiftViewLeft => { name: "shift_view_left", overview: Cancel, doc: "shift view to tag on left", run: |ctx| { shift_view(ctx, HorizontalDirection::Left); } },
    ShiftViewRight => { name: "shift_view_right", overview: Cancel, doc: "shift view to tag on right", run: |ctx| { shift_view(ctx, HorizontalDirection::Right); } },
    ViewAll => { name: "view_all", overview: Cancel, doc: "view all tags", run: |ctx| { crate::tags::view::view_selection(ctx, TagSelection::All); } },
    TagAll => { name: "tag_all", doc: "tag client with all tags", run: |ctx| { with_selected_win(ctx, |ctx, win| crate::tags::client_tags::set_client_tag(ctx, win, TagMask::ALL_BITS)); } },
    ToggleOverview => { name: "toggle_overview", overview: Preserve, doc: "toggle overview mode", run: |ctx| { toggle_overview(ctx, TagMask::ALL_BITS); } },
    CancelOverview => { name: "cancel_overview", overview: Preserve, doc: "leave overview and restore previous view", run: |ctx| { cancel_overview(ctx, TagMask::ALL_BITS); } },
    EdgeScratchpadToggle => { name: "edge_scratchpad_toggle", overview: Preserve, doc: "toggle the default edge scratchpad", run: |ctx| { scratchpad_toggle(ctx, Some(DEFAULT_EDGE_SCRATCHPAD_NAME)); } },
    EdgeScratchpadCreate => { name: "edge_scratchpad_create", doc: "toggle the default edge scratchpad (create from the focused window, or restore if it exists)", run: |ctx| { edge_scratchpad_create(ctx); } },
    EdgeScratchpadShow => { name: "edge_scratchpad_show", overview: Preserve, doc: "show the default edge scratchpad", run: |ctx| { let _ = scratchpad_show_name(ctx, DEFAULT_EDGE_SCRATCHPAD_NAME); } },
    EdgeScratchpadHide => { name: "edge_scratchpad_hide", overview: Preserve, doc: "hide the default edge scratchpad", run: |ctx| { scratchpad_hide_name(ctx, DEFAULT_EDGE_SCRATCHPAD_NAME); } },
    EdgeScratchpadDirectionUp => { name: "edge_scratchpad_direction_up", overview: Preserve, doc: "set default edge scratchpad direction to top", run: |ctx| { edge_scratchpad_set_direction(ctx, EdgeDirection::Top); } },
    EdgeScratchpadDirectionDown => { name: "edge_scratchpad_direction_down", overview: Preserve, doc: "set default edge scratchpad direction to bottom", run: |ctx| { edge_scratchpad_set_direction(ctx, EdgeDirection::Bottom); } },
    EdgeScratchpadDirectionLeft => { name: "edge_scratchpad_direction_left", overview: Preserve, doc: "set default edge scratchpad direction to left", run: |ctx| { edge_scratchpad_set_direction(ctx, EdgeDirection::Left); } },
    EdgeScratchpadDirectionRight => { name: "edge_scratchpad_direction_right", overview: Preserve, doc: "set default edge scratchpad direction to right", run: |ctx| { edge_scratchpad_set_direction(ctx, EdgeDirection::Right); } },
    ScratchpadToggle => {
        name: "scratchpad_toggle",
        doc: "toggle scratchpad, creating it from current window if it doesn't exist",
        run: |ctx| {
            if ctx.core().model().scratchpad_find(DEFAULT_SCRATCHPAD_NAME).is_some() {
                scratchpad_toggle(ctx, Some(DEFAULT_SCRATCHPAD_NAME));
            } else {
                let _ = scratchpad_create(ctx, DEFAULT_SCRATCHPAD_NAME, None, None, ScratchpadInitialStatus::Shown);
            }
        }
    },
    ScratchpadRestore => { name: "scratchpad_restore", doc: "restore the focused scratchpad as an ordinary window", run: |ctx| { let _ = scratchpad_restore(ctx, None, None); } },
    ToggleBar => { name: "toggle_bar", overview: Preserve, doc: "toggle status bar", run: |ctx| { toggle_bar(ctx); } },
    ToggleBottomBar(Option<ToggleAction>) => { name: "toggle_bottom_bar", overview: Preserve, doc: "toggle or set bottom bar visibility", run: |ctx, action| {
        let mut shown = ctx.core().model().expect_selected_monitor().shows_bottom_bar();
        action.unwrap_or_default().apply(&mut shown);
        crate::toggles::set_bottom_bar_shown(ctx, shown);
    } },
    ToggleFloating => { name: "toggle_floating", doc: "toggle focused window between tiled and floating", run: |ctx| { toggle_floating(ctx); } },
    ToggleSticky => { name: "toggle_sticky", doc: "toggle sticky (visible on all tags)", run: |ctx| { with_selected_win(ctx, toggle_sticky); } },
    ToggleAltTag(Option<ToggleAction>) => { name: "toggle_alt_tag", overview: Preserve, doc: "toggle or set alt-tag mode", run: |ctx, action| { toggle_alt_tag(ctx, action.unwrap_or_default()); } },
    ToggleAnimated(Option<ToggleAction>) => { name: "toggle_animated", overview: Preserve, doc: "toggle or set window animations", run: |ctx, action| { let action = action.unwrap_or_default(); let mut enabled = ctx.core().config().animations.enabled; action.apply(&mut enabled); ctx.core_mut().state_mut().config.animations.enabled = enabled; } },
    ToggleHideTags(Option<ToggleAction>) => { name: "toggle_hide_tags", overview: Preserve, doc: "toggle or set hiding empty tags in the bar", run: |ctx, action| { toggle_hide_tags(ctx, action.unwrap_or_default()); } },
    ToggleFocusFollowsFloatMouse(Option<ToggleAction>) => { name: "toggle_focus_follows_float_mouse", overview: Preserve, doc: "toggle or set focus-follows-mouse for floating windows", run: |ctx, action| { let action = action.unwrap_or_default(); let mut enabled = ctx.core().config().window.focus_follows_float_mouse; action.apply(&mut enabled); ctx.core_mut().state_mut().config.window.focus_follows_float_mouse = enabled; } },
    SetFocusFollowsMouse(FocusFollowsMouseMode) => { name: "set_focus_follows_mouse", overview: Preserve, doc: "set focus-follows-mouse behavior", run: |ctx, mode| { let mode = *mode; ctx.core_mut().state_mut().config.window.focus_follows_mouse = mode; } },
    ModeToggle(String) => { name: "mode_toggle", doc: "toggle a mode (enter if not active, else return to default)", run: |ctx, mode| { validate_mode_name(&ctx.core().config().bindings.modes, mode)?; toggle_mode(ctx, mode); } },
    UnhideAll => { name: "unhide_all", doc: "show all hidden windows", run: |ctx| { unhide_all(ctx); } },
    Hide => { name: "hide", doc: "minimize focused window or hide the visible scratchpad", run: |ctx| { with_selected_win(ctx, crate::client::hide_for_user); } },
    ToggleFakeFullscreen => { name: "toggle_fake_fullscreen", doc: "toggle fake fullscreen", run: |ctx| { toggle_fake_fullscreen(ctx); } },
    DrawWindow => { name: "draw_window", doc: "start dragging/resizing window", run: |ctx| { draw_window(ctx); } },
    BeginTreePlacement => { name: "begin_tree_placement", doc: "place the focused tiled window within its layout tree", run: |ctx| { let _ = begin_tree_placement(ctx); } },
    PlacementLeft => { name: "placement_left", doc: "select the placement target to the left", run: |ctx| { step_keyboard_tree_placement(ctx, Side::Left); } },
    PlacementRight => { name: "placement_right", doc: "select the placement target to the right", run: |ctx| { step_keyboard_tree_placement(ctx, Side::Right); } },
    PlacementUp => { name: "placement_up", doc: "select the placement target above", run: |ctx| { step_keyboard_tree_placement(ctx, Side::Top); } },
    PlacementDown => { name: "placement_down", doc: "select the placement target below", run: |ctx| { step_keyboard_tree_placement(ctx, Side::Bottom); } },
    PlacementSwapLeft => { name: "placement_swap_left", doc: "swap the armed window with its left neighbour", run: |ctx| { swap_keyboard_tree_placement(ctx, Side::Left); } },
    PlacementSwapRight => { name: "placement_swap_right", doc: "swap the armed window with its right neighbour", run: |ctx| { swap_keyboard_tree_placement(ctx, Side::Right); } },
    PlacementSwapUp => { name: "placement_swap_up", doc: "swap the armed window with its upper neighbour", run: |ctx| { swap_keyboard_tree_placement(ctx, Side::Top); } },
    PlacementSwapDown => { name: "placement_swap_down", doc: "swap the armed window with its lower neighbour", run: |ctx| { swap_keyboard_tree_placement(ctx, Side::Bottom); } },
    PlacementResizeLeft => { name: "placement_resize_left", doc: "resize the armed window at its left edge", run: |ctx| { resize_keyboard_tree_placement(ctx, Side::Left); } },
    PlacementResizeRight => { name: "placement_resize_right", doc: "resize the armed window at its right edge", run: |ctx| { resize_keyboard_tree_placement(ctx, Side::Right); } },
    PlacementResizeUp => { name: "placement_resize_up", doc: "resize the armed window at its upper edge", run: |ctx| { resize_keyboard_tree_placement(ctx, Side::Top); } },
    PlacementResizeDown => { name: "placement_resize_down", doc: "resize the armed window at its lower edge", run: |ctx| { resize_keyboard_tree_placement(ctx, Side::Bottom); } },
    PlacementNext => { name: "placement_next", doc: "select the next placement target", run: |ctx| { cycle_keyboard_tree_placement(ctx, false); } },
    PlacementPrevious => { name: "placement_previous", doc: "select the previous placement target", run: |ctx| { cycle_keyboard_tree_placement(ctx, true); } },
    PlacementCenter => { name: "placement_center", doc: "select the center replacement target", run: |ctx| { center_keyboard_tree_placement(ctx); } },
    PlacementApply => { name: "placement_apply", doc: "apply the pending tree placement", run: |ctx| { finish_keyboard_tree_placement(ctx, true); } },
    PlacementCancel => { name: "placement_cancel", doc: "cancel the pending tree placement", run: |ctx| { finish_keyboard_tree_placement(ctx, false); } },
    NextKeyboardLayout => { name: "next_keyboard_layout", overview: Preserve, doc: "cycle to next keyboard layout", run: |ctx| { let _ = crate::keyboard_layout::cycle_keyboard_layout(ctx, StackDirection::Next); } },
    PrevKeyboardLayout => { name: "prev_keyboard_layout", overview: Preserve, doc: "cycle to previous keyboard layout", run: |ctx| { let _ = crate::keyboard_layout::cycle_keyboard_layout(ctx, StackDirection::Previous); } },
    KeyboardLayout(String) => { name: "keyboard_layout", overview: Preserve, doc: "set keyboard layout, e.g. us(intl)", run: |ctx, layout| { crate::keyboard_layout::set_keyboard_layout_by_name(ctx, layout); } },
    SetMode(String) => { name: "set_mode", doc: "set WM mode (sway-like modes)", run: |ctx, mode| { validate_mode_name(&ctx.core().config().bindings.modes, mode)?; ctx.set_current_mode(mode.clone()); } },
    Spawn(Vec<String>) => { name: "spawn", overview: Preserve, doc: "spawn a command without shell expansion", run: |ctx, argv| { spawn(ctx, argv)?; } },
    SetLayout(LayoutCommand) => { name: "set_layout", doc: "set layout", run: |ctx, layout| { set_layout(ctx, *layout); } },
    ResetLayout => { name: "reset_layout", doc: "reset the active layout to stock geometry", run: |ctx| { reset_active_layout(ctx); } },
    FocusStack(StackDirection) => { name: "focus_stack", overview: Preserve, doc: "focus stack direction", run: |ctx, direction| { focus_stack(ctx, *direction); } },
    ViewTag(u32) => { name: "view_tag", overview: Cancel, doc: "view a tag by its 1-based number", run: |ctx, number| {
        let number = *number as usize;
        if number == 0 || number > ctx.core().model().tags.num_tags {
            return Err(format!("tag number {number} is out of range"));
        }
        let mask = TagMask::from_index(number - 1).ok_or_else(|| format!("tag number {number} is out of range"))?;
        crate::tags::view::view_tags(ctx, mask);
    } },
    WarpFocus => { name: "warp_focus", overview: Preserve, doc: "warp the pointer to the focused window", run: |ctx| { crate::mouse::warp::warp_to_focus(ctx); } },
    FocusMon(MonitorDirection) => { name: "focus_mon", overview: Cancel, doc: "focus another monitor, warping the pointer to it", run: |ctx, direction| { focus_monitor(ctx, *direction); } },
    SendMon(MonitorDirection) => { name: "send_mon", doc: "move the focused client to another monitor without following", run: |ctx, direction| { send_to_monitor(ctx, *direction); } },
    FollowMon(MonitorDirection) => { name: "follow_mon", doc: "move the focused client to another monitor and follow", run: |ctx, direction| { move_to_monitor_and_follow(ctx, *direction); } },
    SetBorder(Option<u32>) => { name: "set_border", doc: "set the focused window border width", run: |ctx, width| {
        let width = match width {
            Some(width) => i32::try_from(*width).map_err(|_| format!("border width {width} is too large"))?,
            None => crate::config::mod_consts::BORDER_PX,
        };
        with_selected_win(ctx, |ctx, win| ctx.set_border(win, width));
    } }
);

#[cfg(test)]
mod tests {
    use super::{NamedAction, focus_vertical, move_horizontal, move_vertical};
    use crate::backend::Backend;
    use crate::backend::wayland::WaylandBackend;
    use crate::layouts::tree::Preset;

    use crate::layouts::{LayoutCommand, PresentationMode};
    use crate::types::{
        Client, ClientMode, HorizontalDirection, Monitor, Rect, StackDirection, TagMask,
        ToggleAction, VerticalDirection, WindowId,
    };
    use crate::wm::Wm;

    fn maximized_tiled_wm(windows: &[WindowId], selected: WindowId) -> Wm {
        let mut wm = Wm::new(Backend::new_wayland(WaylandBackend::new()));
        wm.core.model.tags.num_tags = 3;
        let tag = TagMask::single(1).unwrap();
        let monitor_id = wm.core.model.monitors.push(Monitor {
            monitor_rect: Rect::new(0, 0, 1200, 800),
            available_rect: Rect::new(0, 0, 1200, 800),
            ..Monitor::default()
        });
        wm.core.model.monitors.set_selected(monitor_id);
        for &win in windows {
            wm.core.model.insert_client(Client {
                win,
                monitor_id,
                tags: tag,
                mode: ClientMode::tiled(),
                ..Client::default()
            });
        }
        let monitor = wm.core.model.monitor_mut(monitor_id).unwrap();
        monitor.set_selected_tags(tag);
        monitor.clients = windows.to_vec();
        monitor.selected = Some(selected);
        monitor
            .per_tag_state()
            .layout_tree
            .apply_preset(Preset::Grid, windows, 1);
        monitor.per_tag_state().presentation = PresentationMode::Maximized;
        wm
    }

    #[test]
    fn layout_command_from_name_accepts_only_canonical_names() {
        assert_eq!(LayoutCommand::from_name("tile"), Some(LayoutCommand::Tile));
        assert_eq!(
            LayoutCommand::from_name("floating"),
            Some(LayoutCommand::Floating)
        );
        assert_eq!(
            LayoutCommand::from_name("maximized"),
            Some(LayoutCommand::Maximized)
        );
        assert_eq!(
            LayoutCommand::from_name("bottom-stack"),
            Some(LayoutCommand::BottomStack)
        );
        assert_eq!(LayoutCommand::from_name("bad"), None);
    }

    #[test]
    fn toggle_animated_flips_the_config_animation_switch() {
        let mut wm = maximized_tiled_wm(&[WindowId(1)], WindowId(1));
        assert!(wm.core.config.animations.enabled);

        NamedAction::ToggleAnimated(None)
            .execute(&mut wm.ctx())
            .unwrap();
        assert!(!wm.core.config.animations.enabled);

        NamedAction::ToggleAnimated(Some(ToggleAction::SetTrue))
            .execute(&mut wm.ctx())
            .unwrap();
        assert!(wm.core.config.animations.enabled);

        NamedAction::ToggleAnimated(Some(ToggleAction::SetFalse))
            .execute(&mut wm.ctx())
            .unwrap();
        assert!(!wm.core.config.animations.enabled);
    }

    fn parse(name: &str, args: &[&str]) -> Result<NamedAction, String> {
        let args: Vec<String> = args.iter().map(|arg| arg.to_string()).collect();
        NamedAction::parse(name, &args)
    }

    #[test]
    fn actions_parse_typed_arguments_once() {
        assert_eq!(
            parse("edge_scratchpad_direction_left", &[]),
            Ok(NamedAction::EdgeScratchpadDirectionLeft)
        );
        assert_eq!(
            parse("focus_stack", &["backward"]),
            Ok(NamedAction::FocusStack(StackDirection::Previous))
        );
        assert_eq!(
            parse("toggle_alt_tag", &[]),
            Ok(NamedAction::ToggleAltTag(None))
        );
        assert_eq!(
            parse("toggle_alt_tag", &["on"]),
            Ok(NamedAction::ToggleAltTag(Some(ToggleAction::SetTrue)))
        );
        assert_eq!(
            parse("set_layout", &["bottom-stack"]),
            Ok(NamedAction::SetLayout(LayoutCommand::BottomStack))
        );

        for (name, args) in [
            ("toggle_alt_tag", &["sometimes"][..]),
            ("focus_next", &["unexpected"]),
            ("set_layout", &[]),
            ("set_layout", &["not-a-layout"]),
            ("set_border", &["-1"]),
            ("inc_gaps", &["x"]),
            ("dec_gaps", &["2", "3"]),
            ("spawn", &[]),
            ("none", &[]),
        ] {
            assert!(parse(name, args).is_err(), "{name} {args:?} should fail");
        }
    }

    #[test]
    fn rendered_arguments_parse_back_to_the_same_action() {
        for action in [
            NamedAction::Spawn(vec!["printf".into(), "hello world".into()]),
            NamedAction::FocusMon(crate::types::MonitorDirection::Prev),
            NamedAction::ToggleAnimated(Some(ToggleAction::SetFalse)),
            NamedAction::IncMasterCount(Some(-1)),
            NamedAction::SetBorder(None),
        ] {
            assert_eq!(
                NamedAction::parse(action.name(), &action.args()),
                Ok(action)
            );
        }
    }

    #[test]
    fn gap_actions_move_both_gaps_and_clamp_at_zero() {
        let mut wm = Wm::new(Backend::new_wayland(WaylandBackend::new()));
        wm.core.config.layout.inner_gap = 4;
        wm.core.config.layout.outer_gap = 8;

        NamedAction::IncGaps(Some(3))
            .execute(&mut wm.ctx())
            .unwrap();
        assert_eq!(wm.core.config.layout.inner_gap, 7);
        assert_eq!(wm.core.config.layout.outer_gap, 11);

        // The default step applies when no argument is passed.
        NamedAction::DecGaps(None).execute(&mut wm.ctx()).unwrap();
        assert_eq!(wm.core.config.layout.inner_gap, 5);
        assert_eq!(wm.core.config.layout.outer_gap, 9);

        // Decreasing past the floor clamps instead of disabling windows into
        // negative gaps.
        for _ in 0..10 {
            NamedAction::DecGaps(None).execute(&mut wm.ctx()).unwrap();
        }
        assert_eq!(wm.core.config.layout.inner_gap, 0);
        assert_eq!(wm.core.config.layout.outer_gap, 0);
    }

    #[test]
    fn toggle_actions_can_set_idempotently() {
        let mut wm = Wm::new(Backend::new_wayland(WaylandBackend::new()));
        let action = NamedAction::ToggleAltTag(Some(ToggleAction::SetTrue));
        action.execute(&mut wm.ctx()).unwrap();
        action.execute(&mut wm.ctx()).unwrap();
        assert!(wm.core.model.tags.show_alternative_names);
    }

    #[test]
    fn quit_action_uses_the_normal_wm_shutdown_flag() {
        let mut wm = Wm::new(Backend::new_wayland(WaylandBackend::new()));
        NamedAction::Quit.execute(&mut wm.ctx()).unwrap();
        assert!(!wm.running);
    }

    #[test]
    fn action_dispatch_rejects_unknown_and_interaction_owned_modes() {
        let mut wm = Wm::new(Backend::new_wayland(WaylandBackend::new()));

        let unknown = NamedAction::SetMode("does-not-exist".to_string())
            .execute(&mut wm.ctx())
            .unwrap_err();
        assert!(unknown.contains("not found"));

        let placement =
            NamedAction::SetMode(crate::core_state::TREE_PLACEMENT_MODE_NAME.to_string())
                .execute(&mut wm.ctx())
                .unwrap_err();
        assert!(placement.contains("begin_tree_placement"));
    }

    #[test]
    fn horizontal_window_move_crosses_tags_only_at_the_tree_edge() {
        let mut wm = Wm::new(Backend::new_wayland(WaylandBackend::new()));
        wm.core.model.tags.num_tags = 3;
        let tag1 = TagMask::single(1).unwrap();
        let tag2 = TagMask::single(2).unwrap();
        let monitor_id = wm.core.model.monitors.push(Monitor {
            monitor_rect: Rect::new(0, 0, 1200, 800),
            available_rect: Rect::new(0, 0, 1200, 800),
            ..Monitor::default()
        });
        wm.core.model.monitors.set_selected(monitor_id);

        let left = WindowId(1);
        let right = WindowId(2);
        for win in [left, right] {
            wm.core.model.insert_client(Client {
                win,
                monitor_id,
                tags: tag1,
                mode: ClientMode::tiled(),
                ..Client::default()
            });
        }
        let monitor = wm.core.model.monitor_mut(monitor_id).unwrap();
        monitor.set_selected_tags(tag1);
        monitor.clients = vec![left, right];
        monitor.selected = Some(left);
        monitor
            .per_tag_state()
            .layout_tree
            .apply_preset(Preset::MasterStack, &[left, right], 1);

        move_horizontal(&mut wm.ctx(), HorizontalDirection::Right);

        // The first press has a visual neighbour, so it only swaps the tree.
        assert_eq!(wm.core.model.client(left).unwrap().tags, tag1);
        assert_eq!(
            wm.core.model.expect_selected_monitor().selected_tags(),
            tag1
        );

        move_horizontal(&mut wm.ctx(), HorizontalDirection::Right);

        // The same client is now at the right edge, so the next press carries
        // it into the adjacent tag and follows it there.
        assert_eq!(wm.core.model.client(left).unwrap().tags, tag2);
        assert_eq!(
            wm.core.model.expect_selected_monitor().selected_tags(),
            tag2
        );
        assert_eq!(wm.core.model.selected_win(), Some(left));
    }

    #[test]
    fn maximized_window_move_reorders_adjacent_titles_not_hidden_visual_neighbors() {
        let windows = [WindowId(1), WindowId(2), WindowId(3), WindowId(4)];
        let selected = WindowId(4);
        let mut wm = maximized_tiled_wm(&windows, selected);

        // In this grid, window 4's hidden visual neighbour to the left is
        // window 2. The exposed maximized order instead places window 3
        // immediately before it.
        assert_eq!(
            wm.core
                .model
                .expect_selected_monitor()
                .per_tag()
                .unwrap()
                .layout_tree
                .visual_neighbor(selected, crate::layouts::tree::Side::Left),
            Some(WindowId(2))
        );

        move_horizontal(&mut wm.ctx(), HorizontalDirection::Left);

        let monitor = wm.core.model.expect_selected_monitor();
        assert_eq!(
            monitor.per_tag().unwrap().layout_tree.leaves(),
            vec![WindowId(1), WindowId(2), WindowId(4), WindowId(3)]
        );
        assert_eq!(
            monitor.bar_client_order(&wm.core.model.clients),
            vec![WindowId(1), WindowId(2), WindowId(4), WindowId(3)]
        );
        assert_eq!(monitor.selected, Some(selected));
    }

    #[test]
    fn maximized_horizontal_move_crosses_tags_at_title_strip_boundary() {
        let windows = [WindowId(1), WindowId(2), WindowId(3)];
        let selected = WindowId(3);
        let mut wm = maximized_tiled_wm(&windows, selected);
        let tag2 = TagMask::single(2).unwrap();

        move_horizontal(&mut wm.ctx(), HorizontalDirection::Right);

        assert_eq!(wm.core.model.client(selected).unwrap().tags, tag2);
        assert_eq!(
            wm.core.model.expect_selected_monitor().selected_tags(),
            tag2
        );
        assert_eq!(wm.core.model.selected_win(), Some(selected));
    }

    #[test]
    fn maximized_vertical_move_stops_at_title_strip_boundary() {
        let windows = [WindowId(1), WindowId(2), WindowId(3)];
        let selected = WindowId(3);
        let mut wm = maximized_tiled_wm(&windows, selected);
        let tag1 = TagMask::single(1).unwrap();

        move_vertical(&mut wm.ctx(), VerticalDirection::Up);
        assert_eq!(
            wm.core
                .model
                .expect_selected_monitor()
                .per_tag()
                .unwrap()
                .layout_tree
                .leaves(),
            vec![WindowId(1), WindowId(3), WindowId(2)]
        );

        move_vertical(&mut wm.ctx(), VerticalDirection::Down);
        assert_eq!(
            wm.core
                .model
                .expect_selected_monitor()
                .per_tag()
                .unwrap()
                .layout_tree
                .leaves(),
            windows
        );

        move_vertical(&mut wm.ctx(), VerticalDirection::Down);

        let monitor = wm.core.model.expect_selected_monitor();
        assert_eq!(monitor.per_tag().unwrap().layout_tree.leaves(), windows);
        assert_eq!(monitor.selected_tags(), tag1);
        assert_eq!(monitor.selected, Some(selected));
    }

    #[test]
    fn maximized_move_does_not_treat_pending_tree_reconciliation_as_a_boundary() {
        let windows = [WindowId(1), WindowId(2), WindowId(3)];
        let selected = WindowId(3);
        let mut wm = maximized_tiled_wm(&windows, selected);
        let tag1 = TagMask::single(1).unwrap();
        assert!(
            wm.core
                .model
                .expect_selected_monitor_mut()
                .per_tag_state()
                .layout_tree
                .remove(selected)
        );

        // Title order defensively appends a newly managed tiled client before
        // the next arrange reconciles its leaf. Moving left during that window
        // must not fall through to an adjacent-tag transfer.
        move_horizontal(&mut wm.ctx(), HorizontalDirection::Left);

        assert_eq!(wm.core.model.client(selected).unwrap().tags, tag1);
        assert_eq!(
            wm.core.model.expect_selected_monitor().selected_tags(),
            tag1
        );
        assert_eq!(wm.core.model.selected_win(), Some(selected));
    }

    #[test]
    fn vertical_focus_falls_back_to_cycling_in_bar_order() {
        let mut wm = Wm::new(Backend::new_wayland(WaylandBackend::new()));
        let tag = TagMask::single(1).unwrap();
        let monitor_id = wm.core.model.monitors.push(Monitor {
            monitor_rect: Rect::new(0, 0, 1200, 800),
            available_rect: Rect::new(0, 0, 1200, 800),
            ..Monitor::default()
        });
        wm.core.model.monitors.set_selected(monitor_id);

        let left = WindowId(1);
        let middle = WindowId(2);
        let right = WindowId(3);
        for win in [left, middle, right] {
            wm.core.model.insert_client(Client {
                win,
                monitor_id,
                tags: tag,
                mode: ClientMode::tiled(),
                ..Client::default()
            });
        }
        let monitor = wm.core.model.monitor_mut(monitor_id).unwrap();
        monitor.set_selected_tags(tag);
        monitor.clients = vec![left, middle, right];
        monitor.selected = Some(middle);
        monitor.per_tag_state().layout_tree.apply_preset(
            Preset::BottomStack,
            &[left, middle, right],
            0,
        );

        focus_vertical(&mut wm.ctx(), VerticalDirection::Down);
        assert_eq!(wm.core.model.selected_win(), Some(right));

        focus_vertical(&mut wm.ctx(), VerticalDirection::Down);
        assert_eq!(wm.core.model.selected_win(), Some(left));

        focus_vertical(&mut wm.ctx(), VerticalDirection::Up);
        assert_eq!(wm.core.model.selected_win(), Some(right));
    }
}
