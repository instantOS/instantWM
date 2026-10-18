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
        ConfigEffect::BarPolicy {
            clear_visibility_overrides,
        } => {
            sync_bar_config_to_monitors(&mut ctx.core_mut().state, clear_visibility_overrides);
            crate::monitor::resync_monitor_ui_metrics(&mut ctx.core_mut().state);
            ctx.core_mut()
                .bar
                .invalidate(crate::bar::BarChange::Presentation);
            ctx.core_mut().queue_layout_for_all_monitors_urgent();
        }
        ConfigEffect::Layout => {
            ctx.core_mut().bar.invalidate_content();
            ctx.core_mut().queue_layout_for_all_monitors_urgent();
        }
        ConfigEffect::Fonts => {
            crate::monitor::resync_monitor_ui_metrics(&mut ctx.core_mut().state);
            ctx.core_mut()
                .bar
                .invalidate(crate::bar::BarChange::Appearance);
            ctx.core_mut().queue_layout_for_all_monitors_urgent();
        }
        ConfigEffect::Bar(change) => ctx.core_mut().bar.invalidate(change),
        ConfigEffect::Input => ctx.core_mut().work.queue_input_config_apply(),
        ConfigEffect::Monitors => ctx.core_mut().work.queue_monitor_config_apply(),
        ConfigEffect::Cursor => {
            ctx.core_mut().work.queue_cursor_config_apply();
            ctx.core_mut().bar.invalidate_content();
        }
    }
}
