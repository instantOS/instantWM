//! Configured mouse binding execution.

use crate::actions::execute_button_action;
use crate::contexts::WmCtx;
use crate::types::{ButtonArg, ButtonTarget, MouseButton, WindowId};

#[derive(Clone, Copy, Debug)]
pub struct ButtonBindingEvent {
    pub target: ButtonTarget,
    pub window: Option<WindowId>,
    pub button: MouseButton,
    pub root_x: i32,
    pub root_y: i32,
    pub clean_state: u32,
}

pub fn run_all(ctx: &mut WmCtx<'_>, event: ButtonBindingEvent, numlockmask: u32) {
    let _ = run_matching(ctx, event, numlockmask, MatchPolicy::All);
}

pub fn consume_one(ctx: &mut WmCtx<'_>, event: ButtonBindingEvent, numlockmask: u32) -> bool {
    run_matching(ctx, event, numlockmask, MatchPolicy::First)
}

#[derive(Clone, Copy)]
enum MatchPolicy {
    All,
    First,
}

fn run_matching(
    ctx: &mut WmCtx<'_>,
    event: ButtonBindingEvent,
    numlockmask: u32,
    policy: MatchPolicy,
) -> bool {
    let mut matched = false;
    let buttons = ctx.core().globals().cfg.buttons.clone();
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
                rx: event.root_x,
                ry: event.root_y,
            },
        );
        matched = true;

        if matches!(policy, MatchPolicy::First) {
            return true;
        }
    }
    matched
}
