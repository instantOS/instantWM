use std::collections::hash_map::DefaultHasher;
use std::hash::{Hash, Hasher};
use std::sync::Arc;

use crate::bar::paint::BarScheme;
use crate::bar::scene;

pub(super) fn render_key(
    bar_show: bool,
    systray_show: bool,
    snapshots: &[scene::MonitorBarSnapshot],
) -> u64 {
    let mut hasher = DefaultHasher::new();
    bar_show.hash(&mut hasher);
    systray_show.hash(&mut hasher);
    for snapshot in snapshots {
        hash_monitor_snapshot(&mut hasher, snapshot);
    }
    hasher.finish()
}

fn hash_monitor_snapshot(hasher: &mut DefaultHasher, snapshot: &scene::MonitorBarSnapshot) {
    snapshot.monitor_id.hash(hasher);
    snapshot.rect.x.hash(hasher);
    snapshot.rect.y.hash(hasher);
    snapshot.rect.w.hash(hasher);
    snapshot.rect.h.hash(hasher);
    snapshot.font_size.to_bits().hash(hasher);
    snapshot.font_families.hash(hasher);
    snapshot.is_selected_monitor.hash(hasher);
    hash_scheme(hasher, &snapshot.status_scheme);
    for value in snapshot.status_hover_color.into_array() {
        value.to_bits().hash(hasher);
    }
    snapshot.startmenu_size.hash(hasher);
    snapshot.horizontal_padding.hash(hasher);
    hash_gesture(hasher, snapshot.gesture);
    snapshot.layout_symbol.hash(hasher);
    snapshot.show_shutdown.hash(hasher);
    snapshot.monitor_rect_x.hash(hasher);
    hash_presentation(hasher, &snapshot.presentation);

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
        systray.visual_padding.hash(hasher);
        hash_scheme(hasher, &systray.base_scheme);
        systray.items.items.len().hash(hasher);
        for item in &systray.items.items {
            item.service.hash(hasher);
            item.path.hash(hasher);
            item.icon_size.w.hash(hasher);
            item.icon_size.h.hash(hasher);
            hash_arc_identity(hasher, &item.icon_rgba);
        }
    }
}

fn hash_presentation(hasher: &mut DefaultHasher, presentation: &scene::BarPresentation) {
    use scene::StatusPresentation;

    std::mem::discriminant(&presentation.status).hash(hasher);
    if let StatusPresentation::WmMode { name, .. } = &presentation.status {
        name.hash(hasher);
    }
    if let Some(content) = presentation.status.content() {
        content.text.hash(hasher);
        content.click_events.hash(hasher);
        hash_status_items(hasher, &content.items);
    }
    presentation
        .tray_menu()
        .map(|menu| menu.session_id)
        .hash(hasher);
    if let Some(menu) = presentation.tray_menu() {
        menu.view.hash(hasher);
    }
}

fn hash_scheme(hasher: &mut DefaultHasher, scheme: &BarScheme) {
    for value in scheme
        .foreground
        .into_array()
        .into_iter()
        .chain(scheme.background.into_array())
        .chain(scheme.detail.into_array())
    {
        value.to_bits().hash(hasher);
    }
}

fn hash_gesture(hasher: &mut DefaultHasher, gesture: crate::types::Gesture) {
    std::mem::discriminant(&gesture).hash(hasher);
    match gesture {
        crate::types::Gesture::WinTitle(win) => win.hash(hasher),
        crate::types::Gesture::Tag(tag) | crate::types::Gesture::StatusBlock(tag) => {
            tag.hash(hasher)
        }
        crate::types::Gesture::None
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
                block.border_widths.top.hash(hasher);
                block.border_widths.right.hash(hasher);
                block.border_widths.bottom.hash(hasher);
                block.border_widths.left.hash(hasher);
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

/// Hash an `Arc<[u8]>` by pointer identity instead of contents.
///
/// Cloning an `Arc` preserves the data pointer, and the systray replaces the
/// `Arc` with a freshly-allocated one whenever the icon pixels change, so
/// pointer identity is a reliable change-detection proxy that avoids hashing
/// potentially hundreds of KB of pixel data on every frame.
fn hash_arc_identity(hasher: &mut DefaultHasher, arc: &Arc<[u8]>) {
    Arc::as_ptr(arc).hash(hasher);
    arc.len().hash(hasher);
}

fn hash_i3_align(hasher: &mut DefaultHasher, align: crate::bar::status::I3Align) {
    match align {
        crate::bar::status::I3Align::Left => 0u8.hash(hasher),
        crate::bar::status::I3Align::Center => 1u8.hash(hasher),
        crate::bar::status::I3Align::Right => 2u8.hash(hasher),
    }
}
