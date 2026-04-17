use std::collections::hash_map::DefaultHasher;
use std::hash::{Hash, Hasher};

use crate::bar::paint::BarScheme;
use crate::bar::scene;

pub(super) fn render_key(
    core: &crate::contexts::CoreCtx,
    snapshots: &[scene::MonitorBarSnapshot],
    wayland_systray_menu: Option<&crate::types::WaylandSystrayMenu>,
) -> u64 {
    let mut hasher = DefaultHasher::new();
    core.globals().cfg.show_bar.hash(&mut hasher);
    core.globals().cfg.show_systray.hash(&mut hasher);
    wayland_systray_menu.is_some().hash(&mut hasher);
    for snapshot in snapshots {
        hash_monitor_snapshot(&mut hasher, snapshot);
    }
    hasher.finish()
}

fn hash_monitor_snapshot(hasher: &mut DefaultHasher, snapshot: &scene::MonitorBarSnapshot) {
    snapshot.monitor_id.index().hash(hasher);
    snapshot.origin_x.hash(hasher);
    snapshot.origin_y.hash(hasher);
    snapshot.width.hash(hasher);
    snapshot.height.hash(hasher);
    snapshot.font_size.to_bits().hash(hasher);
    snapshot.is_selected_monitor.hash(hasher);
    hash_scheme(hasher, &snapshot.status_scheme);
    snapshot.startmenu_size.hash(hasher);
    snapshot.horizontal_padding.hash(hasher);
    hash_gesture(hasher, snapshot.gesture);
    snapshot.layout_symbol.hash(hasher);
    snapshot.show_shutdown.hash(hasher);
    snapshot.monitor_rect_x.hash(hasher);
    snapshot.status_text.hash(hasher);
    hash_status_items(hasher, &snapshot.status_items);

    snapshot.tags.len().hash(hasher);
    for tag in &snapshot.tags {
        tag.slot.hash(hasher);
        tag.label.hash(hasher);
        hash_scheme(hasher, &tag.scheme);
    }

    snapshot.titles.len().hash(hasher);
    for title in &snapshot.titles {
        title.win.hash(hasher);
        title.name.hash(hasher);
        hash_scheme(hasher, &title.scheme);
        title.close_scheme.is_some().hash(hasher);
        if let Some(scheme) = &title.close_scheme {
            hash_scheme(hasher, scheme);
        }
    }

    snapshot.systray.is_some().hash(hasher);
    if let Some(systray) = &snapshot.systray {
        systray.spacing.hash(hasher);
        hash_scheme(hasher, &systray.base_scheme);
        systray.items.items.len().hash(hasher);
        for item in &systray.items.items {
            item.service.hash(hasher);
            item.path.hash(hasher);
            item.icon_w.hash(hasher);
            item.icon_h.hash(hasher);
            item.icon_rgba.hash(hasher);
        }
        systray.menu.is_some().hash(hasher);
        if let Some(menu) = &systray.menu {
            menu.service.hash(hasher);
            menu.path.hash(hasher);
            menu.item_h.hash(hasher);
            menu.items.len().hash(hasher);
            for item in &menu.items {
                item.id.hash(hasher);
                item.label.hash(hasher);
                item.width.hash(hasher);
                item.enabled.hash(hasher);
                item.separator.hash(hasher);
            }
        }
    }
}

fn hash_scheme(hasher: &mut DefaultHasher, scheme: &BarScheme) {
    for value in scheme
        .fg
        .iter()
        .chain(scheme.bg.iter())
        .chain(scheme.detail.iter())
    {
        value.to_bits().hash(hasher);
    }
}

fn hash_gesture(hasher: &mut DefaultHasher, gesture: crate::types::Gesture) {
    std::mem::discriminant(&gesture).hash(hasher);
    match gesture {
        crate::types::Gesture::WinTitle(win) => win.hash(hasher),
        crate::types::Gesture::Tag(tag) => tag.hash(hasher),
        crate::types::Gesture::None
        | crate::types::Gesture::Overlay
        | crate::types::Gesture::CloseButton
        | crate::types::Gesture::StartMenu => {}
    }
}

fn hash_status_items(hasher: &mut DefaultHasher, items: &[crate::bar::status::StatusItem]) {
    items.len().hash(hasher);
    for item in items {
        match item {
            crate::bar::status::StatusItem::Text(text) => {
                0u8.hash(hasher);
                text.hash(hasher);
            }
            crate::bar::status::StatusItem::I3Block(block) => {
                1u8.hash(hasher);
                block.full_text.hash(hasher);
                block.short_text.hash(hasher);
                block.color.hash(hasher);
                block.background.hash(hasher);
                block.border.hash(hasher);
                block.border_top.hash(hasher);
                block.border_right.hash(hasher);
                block.border_bottom.hash(hasher);
                block.border_left.hash(hasher);
                match &block.min_width {
                    Some(crate::bar::status::I3MinWidth::Text(text)) => {
                        1u8.hash(hasher);
                        text.hash(hasher);
                    }
                    Some(crate::bar::status::I3MinWidth::Pixels(px)) => {
                        2u8.hash(hasher);
                        px.hash(hasher);
                    }
                    None => 0u8.hash(hasher),
                }
                hash_i3_align(hasher, block.align);
                block.urgent.hash(hasher);
                block.separator.hash(hasher);
                block.separator_block_width.hash(hasher);
                block.name.hash(hasher);
                block.instance.hash(hasher);
                block.markup.hash(hasher);
            }
        }
    }
}

fn hash_i3_align(hasher: &mut DefaultHasher, align: crate::bar::status::I3Align) {
    match align {
        crate::bar::status::I3Align::Left => 0u8.hash(hasher),
        crate::bar::status::I3Align::Center => 1u8.hash(hasher),
        crate::bar::status::I3Align::Right => 2u8.hash(hasher),
    }
}
