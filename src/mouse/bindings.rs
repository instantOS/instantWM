//! Configured mouse binding execution.

use crate::actions::execute_button_action;
use crate::contexts::WmCtx;
use crate::types::{
    ButtonArg, ButtonTarget, InteractionSource, ModMask, MouseButton, Point, WindowId,
};

#[derive(Clone, Copy, Debug)]
pub struct ButtonBindingEvent {
    pub target: ButtonTarget,
    pub window: Option<WindowId>,
    pub button: MouseButton,
    pub source: InteractionSource,
    pub root: Point,
    pub clean_state: ModMask,
    pub time_msec: u32,
}

/// Dispatch the first configured binding matching this event.
///
/// Named for what it does rather than for the loop that finds the match: it both
/// selects the owning binding and runs it, and it pairs with
/// `execute_button_action`, which it calls.
///
/// Returns whether a binding owned the event. A physical button chord has
/// exactly one owner: configuration order is the explicit precedence rule when
/// duplicate targets are present, and callers that need several operations
/// should expose a compound action instead of relying on backend-dependent
/// duplicate dispatch.
pub(crate) fn dispatch_button_binding(
    ctx: &mut WmCtx<'_>,
    event: ButtonBindingEvent,
    numlockmask: ModMask,
) -> bool {
    // Find the owning binding first, then execute it. Holding the borrow across
    // `execute_button_action` would alias `ctx`, and cloning the whole table per
    // click to dodge that is what this replaces; only the winning action is
    // cloned.
    let matched = ctx
        .core()
        .config()
        .bindings
        .buttons
        .iter()
        .find(|binding| {
            binding.matches(event.target)
                && binding.button == event.button
                && binding.mask.cleaned(numlockmask) == event.clean_state
        })
        .map(|binding| (binding.action.clone(), binding.button));

    let Some((action, button)) = matched else {
        return false;
    };

    execute_button_action(
        ctx,
        &action,
        ButtonArg {
            target: event.target,
            window: event.window,
            btn: button,
            source: event.source,
            root: event.root,
            time_msec: event.time_msec,
        },
    );
    true
}
