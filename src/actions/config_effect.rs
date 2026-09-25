use crate::contexts::WmCtx;

/// Apply the follow-up work a runtime-config edit requires.
///
/// Shared by named actions and IPC config commands so every runtime edit
/// performs the same follow-up work for its [`crate::config::runtime::ConfigEffect`].
pub(crate) fn apply_config_effect(
    ctx: &mut WmCtx<'_>,
    effect: crate::config::runtime::ConfigEffect,
) {
    use crate::config::runtime::{ConfigEffect, sync_bar_config_to_monitors};
    match effect {
        ConfigEffect::None => {}
        ConfigEffect::Bar | ConfigEffect::BarVisibility => {
            sync_bar_config_to_monitors(
                ctx.core_mut().state_mut(),
                effect == ConfigEffect::BarVisibility,
            );
            ctx.reinit_bar_resources();
            ctx.request_bar_update();
            crate::layouts::manager::arrange(ctx, None);
        }
        ConfigEffect::Rearrange => {
            ctx.request_bar_update();
            crate::layouts::manager::arrange(ctx, None);
        }
        ConfigEffect::Recolor => {
            ctx.reinit_bar_resources();
            ctx.request_bar_update();
            crate::layouts::manager::arrange(ctx, None);
        }
        // `request_bar_update` is the backend-agnostic "mark dirty".
        ConfigEffect::BarUpdate => ctx.request_bar_update(),
        ConfigEffect::Input => ctx.core_mut().queue_input_config_apply(),
        ConfigEffect::Monitors => ctx.core_mut().queue_monitor_config_apply(),
        ConfigEffect::Cursor => {
            ctx.core_mut().queue_cursor_config_apply();
            ctx.request_bar_update();
        }
    }
}
