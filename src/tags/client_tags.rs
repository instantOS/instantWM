//! Client-to-tag assignment.

use crate::contexts::WmCtx;
use crate::types::{MonitorId, TagMask, WindowId};

/// Spread ordinary windows on the selected monitor round-robin across all
/// configured tags, starting at tag 1. Include hidden windows and windows on
/// other tags, but leave scratchpads and other monitors alone.
pub fn distribute_clients(ctx: &mut WmCtx) {
    let model = &ctx.core().state.model;
    let tag_count = model.tags.mask().count() as usize;
    if tag_count == 0 {
        return;
    }
    let Some(monitor) = model.selected_monitor() else {
        return;
    };
    let monitor_id = monitor.id();
    // Snapshot the order before focus changes can reorder the monitor's list.
    let windows: Vec<_> = monitor
        .iter_clients()
        .filter(|(_, client)| !client.is_scratchpad())
        .map(|(win, _)| win)
        .collect();
    if windows.is_empty() {
        return;
    }

    let monitor = ctx
        .core_mut()
        .state
        .model
        .monitor_mut(monitor_id)
        .expect("selected monitor was resolved above");
    for (index, &win) in windows.iter().enumerate() {
        let tags = TagMask::from_index(index % tag_count).unwrap();
        let client = monitor
            .client_mut(win)
            .expect("window collected from this monitor in the same pass");
        client.is_sticky = false;
        client.set_tag_mask(tags);
        monitor.record_focus(tags, win);
    }
    for win in windows {
        ctx.sync_client_tag_props(win);
    }
    crate::focus::focus(ctx, None);
    ctx.core_mut().queue_layout_for_monitor_urgent(monitor_id);
}

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
    use super::{distribute_clients, set_client_tag};
    use crate::actions::{KeyAction, NamedAction};
    use crate::backend::BackendKind;
    use crate::config::keybindings::{CONTROL, MODKEY, default_keybinds};
    use crate::config::keysyms::XK_D;
    use crate::test_support::TestWm as Wm;

    use crate::test_support::{MonitorBuilder, add_client, add_selected_client};
    use crate::types::{
        Client, ClientMode, ClientPlacement, Monitor, MonitorId, Rect, SnapPosition, TagMask,
        WindowId,
    };

    fn distribution_wm(tag_count: usize) -> (Wm, MonitorId) {
        let mut wm = Wm::new(crate::backend::WaylandBackendData::default());
        wm.core.state.model.tags.num_tags = tag_count;
        let monitor_id = wm.core.state.model.monitors.push(
            MonitorBuilder::new()
                .monitor_rect(Rect::new(0, 0, 1200, 800))
                .bar(0, false)
                .tag_count(tag_count)
                .selected_tags(TagMask::single(1).unwrap())
                .build(),
        );
        wm.core.state.model.monitors.set_selected(monitor_id);
        (wm, monitor_id)
    }

    #[test]
    fn distribute_wraps_across_configured_tags_for_all_ordinary_windows() {
        for tag_count in [1, 3, 9, 12] {
            let (mut wm, monitor_id) = distribution_wm(tag_count);
            for id in 1..=(2 * tag_count + 1) as u32 {
                let mut client = Client {
                    win: WindowId(id),
                    tags: TagMask::all(tag_count),
                    geo: Rect::new(10, 20, 200, 100),
                    is_hidden: id == 1,
                    is_sticky: id == 2,
                    is_fixed_size: id == 3,
                    snap_status: if id == 4 {
                        SnapPosition::Left
                    } else {
                        SnapPosition::None
                    },
                    ..Client::default()
                };
                client.set_mode_for_test(match id % 3 {
                    0 => ClientMode::floating(),
                    1 => ClientMode::tiled().as_fullscreen(),
                    _ => ClientMode::tiled(),
                });
                add_client(&mut wm.core.state.model, monitor_id, client);
            }
            let before: Vec<_> = wm
                .core
                .state
                .model
                .monitor(monitor_id)
                .unwrap()
                .iter_clients()
                .map(|(win, c)| (win, c.clone()))
                .collect();

            distribute_clients(&mut wm.test_ctx());

            for (index, (win, original)) in before.iter().enumerate() {
                let client = wm.core.state.model.client(*win).unwrap();
                assert_eq!(client.tags, TagMask::from_index(index % tag_count).unwrap());
                assert!(!client.is_sticky);
                assert_eq!(client.is_hidden, original.is_hidden);
                assert_eq!(client.mode(), original.mode());
                assert_eq!(client.geo, original.geo);
                assert_eq!(client.snap_status, original.snap_status);
            }
        }
    }

    #[test]
    fn distribute_shortcut_skips_scratchpads_and_other_monitors_and_repairs_focus() {
        let (mut wm, monitor_id) = distribution_wm(3);
        for id in 1..=4 {
            add_selected_client(
                &mut wm.core.state.model,
                monitor_id,
                Client {
                    win: WindowId(id),
                    tags: TagMask::single(3).unwrap(),
                    ..Client::default()
                },
            );
        }
        wm.core
            .state
            .model
            .monitor_mut(monitor_id)
            .unwrap()
            .set_selected_tags(TagMask::single(3).unwrap());
        // Order is [4, 3, 2, 1]; window 4 will leave the selected tag.
        for (id, direction, hidden) in [
            (10, None, false),
            (11, Some(crate::types::EdgeDirection::Top), true),
        ] {
            let mut client = Client::new(WindowId(id));
            client
                .promote_to_scratchpad(monitor_id, "test", direction, 1200, 800)
                .unwrap();
            client.is_hidden = hidden;
            add_client(&mut wm.core.state.model, monitor_id, client);
        }
        let other = wm.core.state.model.monitors.push(Monitor::default());
        add_selected_client(
            &mut wm.core.state.model,
            other,
            Client {
                win: WindowId(20),
                tags: TagMask::single(2).unwrap(),
                ..Client::default()
            },
        );

        for backend in [BackendKind::X11, BackendKind::Wayland] {
            let binding = default_keybinds(backend)
                .into_iter()
                .find(|key| key.mod_mask == MODKEY | CONTROL && key.keysym == XK_D)
                .unwrap();
            assert!(matches!(
                binding.action,
                KeyAction::Named(NamedAction::DistributeClients)
            ));
        }
        crate::actions::execute_key_action(
            &mut wm.test_ctx(),
            &KeyAction::named(NamedAction::DistributeClients),
        );

        for (win, tag) in [(4, 1), (3, 2), (2, 3), (1, 1)] {
            assert_eq!(
                wm.core.state.model.client(WindowId(win)).unwrap().tags,
                TagMask::single(tag).unwrap()
            );
        }
        for (id, hidden) in [(10, false), (11, true)] {
            let client = wm.core.state.model.client(WindowId(id)).unwrap();
            assert!(client.is_scratchpad());
            assert_eq!(client.tags, TagMask::SCRATCHPAD);
            assert_eq!(client.is_hidden, hidden);
        }
        assert_eq!(
            wm.core.state.model.client(WindowId(20)).unwrap().tags,
            TagMask::single(2).unwrap()
        );
        assert_eq!(
            wm.core.state.model.monitor(other).unwrap().selected,
            Some(WindowId(20))
        );
        // A shown scratchpad may retain focus; in every case focus must be visible.
        let selected = wm.core.state.model.selected_win().unwrap();
        assert_ne!(selected, WindowId(4));
        assert!(
            wm.core
                .state
                .model
                .client_is_visible_on_selected_monitor(selected)
        );
        assert_eq!(
            wm.core
                .state
                .model
                .monitor(monitor_id)
                .unwrap()
                .selected_tags(),
            TagMask::single(3).unwrap()
        );
    }

    #[test]
    fn distribute_handles_empty_state() {
        let mut wm = Wm::new(crate::backend::WaylandBackendData::default());
        distribute_clients(&mut wm.test_ctx());
        let (mut wm, _) = distribution_wm(3);
        distribute_clients(&mut wm.test_ctx());
        wm.core.state.model.tags.num_tags = 0;
        distribute_clients(&mut wm.test_ctx());
    }

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
