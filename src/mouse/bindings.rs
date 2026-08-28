//! Configured mouse binding execution.

use crate::actions::execute_button_action;
use crate::contexts::WmCtx;
use crate::types::{ButtonArg, ButtonTarget, InteractionSource, MouseButton, Point, WindowId};

#[derive(Clone, Copy, Debug)]
pub struct ButtonBindingEvent {
    pub target: ButtonTarget,
    pub window: Option<WindowId>,
    pub button: MouseButton,
    pub source: InteractionSource,
    pub root: Point,
    pub clean_state: u32,
    pub time_msec: u32,
}

/// Execute the first configured binding matching this event.
///
/// A physical button chord has exactly one owner. Configuration order is the
/// explicit precedence rule when duplicate targets are present; callers that
/// need several operations should expose a compound action instead of relying
/// on backend-dependent duplicate dispatch.
pub(crate) fn run_first_matching(
    ctx: &mut WmCtx<'_>,
    event: ButtonBindingEvent,
    numlockmask: u32,
) -> bool {
    let buttons = ctx.core().config().bindings.buttons.clone();
    for binding in &buttons {
        if !binding.matches(event.target) || binding.button != event.button {
            continue;
        }
        if crate::util::clean_mask(binding.mask, numlockmask) != event.clean_state {
            continue;
        }

        execute_button_action(
            ctx,
            &binding.action,
            ButtonArg {
                target: event.target,
                window: event.window,
                btn: binding.button,
                source: event.source,
                root: event.root,
                time_msec: event.time_msec,
            },
        );
        return true;
    }
    false
}
