//! Client-to-tag assignment.

use crate::contexts::WmCtx;
use crate::types::{TagMask, WindowId};

pub fn set_client_tag(ctx: &mut WmCtx, win: WindowId, mask: TagMask) {
    let selmon_id = ctx.core_mut().model_mut().selected_monitor_id();
    let tagmask = ctx.core().model().tags.mask();
    let effective_mask = mask & tagmask;
    if effective_mask.is_empty() {
        return;
    }

    if ctx
        .core()
        .model()
        .client(win)
        .is_some_and(|client| client.is_scratchpad())
    {
        let _ = crate::floating::scratchpad::scratchpad_restore_window(
            ctx,
            win,
            Some((selmon_id, effective_mask)),
        );
        return;
    }

    if let Some(client) = ctx.core_mut().model_mut().client_mut(win) {
        client.is_sticky = false;
        client.set_tag_mask(effective_mask);
    } else {
        return;
    }

    if let WmCtx::X11(x11) = ctx {
        crate::backend::x11::set_client_tag_prop(x11.core.state(), &x11.x11, x11.x11_runtime, win);
    }
    crate::focus::focus(ctx, None);
    ctx.core_mut().queue_layout_for_monitor_urgent(selmon_id);
}

pub fn tag_all(ctx: &mut WmCtx, mask: TagMask) {
    let selmon_id = ctx.core_mut().model_mut().selected_monitor_id();
    let tagmask = ctx.core().model().tags.mask();
    let effective_mask = mask & tagmask;
    if effective_mask.is_empty() {
        return;
    }

    let current_tag = ctx
        .core()
        .model()
        .expect_selected_monitor()
        .current_tag_number();
    let Some(current_tag) = current_tag else {
        return;
    };
    let current_tag_mask = TagMask::single(current_tag).unwrap_or(TagMask::EMPTY);

    let m = ctx.core().model().expect_selected_monitor();
    let clients_on_tag: Vec<_> = m
        .iter_clients(&ctx.core().model().clients)
        .filter(|(_, c)| c.tags.intersects(current_tag_mask))
        .map(|(win, _)| win)
        .collect();

    for win in clients_on_tag {
        if let Some(client) = ctx.core_mut().model_mut().client_mut(win) {
            client.is_sticky = false;
            client.set_tag_mask(effective_mask);
        }
    }

    crate::focus::focus(ctx, None);
    ctx.core_mut().queue_layout_for_monitor_urgent(selmon_id);
}

pub fn follow_tag(ctx: &mut WmCtx, win: WindowId, mask: TagMask) {
    set_client_tag(ctx, win, mask);
    crate::tags::view::view_tags(ctx, mask);
}

pub fn toggle_tag(ctx: &mut WmCtx, win: WindowId, mask: TagMask) {
    let tagmask = ctx.core().model().tags.mask();
    let current_tags = ctx
        .core()
        .state()
        .model
        .client(win)
        .map_or(TagMask::EMPTY, |c| c.tags);
    if current_tags.is_scratchpad_only() {
        set_client_tag(ctx, win, mask);
        return;
    }
    let new_tags = current_tags ^ (mask & tagmask);
    if new_tags.is_empty() {
        return;
    }
    set_client_tag(ctx, win, new_tags);
}

#[cfg(test)]
mod tests {
    use super::set_client_tag;
    use crate::backend::Backend;
    use crate::backend::wayland::WaylandBackend;
    use crate::types::{Client, ClientPlacement, Monitor, TagMask, WindowId};
    use crate::wm::Wm;

    #[test]
    fn assigning_a_tag_explicitly_restores_a_scratchpad() {
        let mut wm = Wm::new(Backend::new_wayland(WaylandBackend::new()));
        wm.core.model.tags.num_tags = 9;
        let monitor_id = wm.core.model.monitors.push(Monitor::default());
        wm.core.model.monitors.set_selected(monitor_id);
        let original_tags = TagMask::single(1).unwrap();
        let target_tags = TagMask::single(3).unwrap();
        wm.core
            .model
            .monitor_mut(monitor_id)
            .unwrap()
            .set_selected_tags(original_tags);

        let win = WindowId(42);
        let mut client = Client {
            win,
            monitor_id,
            tags: original_tags,
            is_sticky: true,
            ..Client::default()
        };
        client
            .promote_to_scratchpad("term", None, 1920, 1080)
            .unwrap();
        assert!(wm.core.model.insert_client(client));
        assert!(wm.core.model.attach_client(win));
        wm.core.model.monitor_mut(monitor_id).unwrap().selected = Some(win);

        set_client_tag(&mut wm.ctx(), win, target_tags);

        let restored = wm.core.model.client(win).unwrap();
        assert!(!restored.is_scratchpad());
        assert_eq!(restored.tags, target_tags);
        assert!(!restored.is_sticky);
        assert_eq!(restored.placement(), ClientPlacement::Tiling);
    }
}
