//! Client-to-tag assignment.

use crate::contexts::WmCtx;
use crate::types::{MonitorId, TagMask, WindowId};

/// The resolved facts a tag assignment needs about its client.
#[derive(Clone, Copy)]
struct TagTarget {
    win: WindowId,
    monitor_id: MonitorId,
    is_scratchpad: bool,
}

/// Assign `win` to `mask`.
///
/// Returns the client's monitor when the assignment was applied, or `None`
/// when `win` is unmanaged or `mask` selects no configured tag.
pub fn set_client_tag(ctx: &mut WmCtx, win: WindowId, mask: TagMask) -> Option<MonitorId> {
    let model = &ctx.core().state.model;
    let view = model.client_view(win)?;
    let target = TagTarget {
        win,
        monitor_id: view.monitor.id(),
        is_scratchpad: view.client.is_scratchpad(),
    };
    let effective_mask = mask & model.tags.mask();
    apply_client_tags(ctx, target, effective_mask)
}

fn apply_client_tags(
    ctx: &mut WmCtx,
    target: TagTarget,
    effective_mask: TagMask,
) -> Option<MonitorId> {
    let TagTarget {
        win,
        monitor_id,
        is_scratchpad,
    } = target;
    if effective_mask.is_empty() {
        return None;
    }

    if is_scratchpad {
        return crate::floating::scratchpad::scratchpad_restore_window(
            ctx,
            win,
            Some((monitor_id, effective_mask)),
        )
        .ok()
        .map(|_| monitor_id);
    }

    {
        let monitor = ctx
            .core_mut()
            .state
            .model
            .monitor_mut(monitor_id)
            .expect("tag target monitor was resolved as the client's owner");
        let client = monitor
            .client_mut(win)
            .expect("tag target was resolved from its owning monitor");
        client.is_sticky = false;
        client.set_tag_mask(effective_mask);
        // Record the window as most-recently-focused on the destination tag so
        // that a subsequent view switch brings it to the front instead of
        // falling back to a stale focus-history entry.
        monitor.record_focus(effective_mask, win);
    }

    ctx.sync_client_tag_props(win);
    if ctx.core().state.model.selected_monitor_id() == monitor_id {
        crate::focus::focus(ctx, None);
    }
    ctx.core_mut().queue_layout_for_monitor_urgent(monitor_id);
    Some(monitor_id)
}

pub fn tag_all(ctx: &mut WmCtx, mask: TagMask) {
    let selmon_id = ctx.core_mut().state.model.selected_monitor_id();
    let tagmask = ctx.core().state.model.tags.mask();
    let effective_mask = mask & tagmask;
    if effective_mask.is_empty() {
        return;
    }

    let current_tag = ctx
        .core()
        .state
        .model
        .expect_selected_monitor()
        .current_tag_number();
    let Some(current_tag) = current_tag else {
        return;
    };
    let current_tag_mask = TagMask::single(current_tag).unwrap_or(TagMask::EMPTY);

    let clients_on_tag: Vec<_> = ctx
        .core()
        .state
        .model
        .expect_selected_monitor()
        .iter_clients()
        .filter(|(_, c)| c.tags.intersects(current_tag_mask))
        .map(|(win, _)| win)
        .collect();

    let monitor = ctx
        .core_mut()
        .state
        .model
        .monitor_mut(selmon_id)
        .expect("selected monitor resolved above");
    for win in clients_on_tag {
        let client = monitor
            .client_mut(win)
            .expect("client collected from this monitor in the same pass");
        client.is_sticky = false;
        client.set_tag_mask(effective_mask);
    }

    crate::focus::focus(ctx, None);
    ctx.core_mut().queue_layout_for_monitor_urgent(selmon_id);
}

/// Assign `win` to `mask` and switch its monitor's view there.
///
/// Nothing is followed when the assignment was rejected.
pub fn follow_tag(ctx: &mut WmCtx, win: WindowId, mask: TagMask) {
    let Some(monitor_id) = set_client_tag(ctx, win, mask) else {
        return;
    };
    crate::focus::select_monitor(ctx, monitor_id);
    crate::tags::view::view_tags(ctx, mask);
    crate::focus::focus(ctx, Some(win));
}

pub fn toggle_tag(ctx: &mut WmCtx, win: WindowId, mask: TagMask) {
    let model = &ctx.core().state.model;
    let Some(view) = model.client_view(win) else {
        return;
    };
    let target = TagTarget {
        win,
        monitor_id: view.monitor.id(),
        is_scratchpad: view.client.is_scratchpad(),
    };
    let tagmask = model.tags.mask();
    let current_tags = view.client.tags;
    let new_tags = if current_tags.is_scratchpad_only() {
        mask & tagmask
    } else {
        current_tags ^ (mask & tagmask)
    };
    let _ = apply_client_tags(ctx, target, new_tags);
}

#[cfg(test)]
mod tests {
    use super::set_client_tag;
    use crate::test_support::TestWm as Wm;

    use crate::test_support::{add_client, add_selected_client};
    use crate::types::{Client, ClientPlacement, Monitor, TagMask, WindowId};

    #[test]
    fn assigning_a_tag_explicitly_restores_a_scratchpad() {
        let mut wm = Wm::new(crate::backend::WaylandBackendData::default());
        wm.core.state.model.tags.num_tags = 9;
        let monitor_id = wm.core.state.model.monitors.push(Monitor::default());
        wm.core.state.model.monitors.set_selected(monitor_id);
        let original_tags = TagMask::single(1).unwrap();
        let target_tags = TagMask::single(3).unwrap();
        wm.core
            .state
            .model
            .monitor_mut(monitor_id)
            .unwrap()
            .set_selected_tags(original_tags);

        let win = WindowId(42);
        let mut client = Client {
            win,
            tags: original_tags,
            is_sticky: true,
            ..Client::default()
        };
        client
            .promote_to_scratchpad(monitor_id, "term", None, 1920, 1080)
            .unwrap();
        add_selected_client(&mut wm.core.state.model, monitor_id, client);

        set_client_tag(&mut wm.test_ctx(), win, target_tags);

        let restored = wm.core.state.model.client(win).unwrap();
        assert!(!restored.is_scratchpad());
        assert_eq!(restored.tags, target_tags);
        assert!(!restored.is_sticky);
        assert_eq!(restored.placement(), ClientPlacement::Tiling);
    }

    #[test]
    fn moved_client_should_be_focused_on_arrival_at_maximized_tag() {
        let tag1 = TagMask::single(1).unwrap();
        let tag2 = TagMask::single(2).unwrap();

        let mut wm = Wm::new(crate::backend::WaylandBackendData::default());
        wm.core.state.model.tags.num_tags = 9;
        let monitor_id = wm.core.state.model.monitors.push(Monitor::default());
        wm.core.state.model.monitors.set_selected(monitor_id);

        // --- Tag 2: maximized, with two existing windows B and C. ---
        {
            let mon = wm.core.state.model.monitor_mut(monitor_id).unwrap();
            mon.set_selected_tags(tag2);
            mon.per_tag_state().presentation = crate::layouts::PresentationMode::Maximized;
        }
        let win_b = WindowId(2);
        let win_c = WindowId(3);
        for win in [win_b, win_c] {
            add_client(
                &mut wm.core.state.model,
                monitor_id,
                Client {
                    win,
                    tags: tag2,
                    ..Client::default()
                },
            );
        }
        // Populate focus history for tag 2 by viewing it and focusing B.
        crate::tags::view::view_tags(&mut wm.test_ctx(), tag2);
        crate::focus::focus(&mut wm.test_ctx(), Some(win_b));
        assert_eq!(wm.core.state.model.selected_win(), Some(win_b));

        // --- Tag 1: one window A, which we will move. ---
        {
            let mon = wm.core.state.model.monitor_mut(monitor_id).unwrap();
            mon.set_selected_tags(tag1);
        }
        let win_a = WindowId(1);
        add_client(
            &mut wm.core.state.model,
            monitor_id,
            Client {
                win: win_a,
                tags: tag1,
                ..Client::default()
            },
        );
        crate::tags::view::view_tags(&mut wm.test_ctx(), tag1);
        crate::focus::focus(&mut wm.test_ctx(), Some(win_a));
        assert_eq!(wm.core.state.model.selected_win(), Some(win_a));

        // --- Move A to tag 2, then immediately switch to tag 2. ---
        crate::tags::client_tags::set_client_tag(&mut wm.test_ctx(), win_a, tag2);
        crate::tags::view::view_tags(&mut wm.test_ctx(), tag2);

        // EXPECTED: A should be on top because the user just moved it there.
        assert_eq!(
            wm.core.state.model.selected_win(),
            Some(win_a),
            "the moved window should be focused after switching to the target tag"
        );
    }
}
