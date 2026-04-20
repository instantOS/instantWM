use crate::bar::paint::{BarPainter, BarScheme};
use crate::contexts::CoreCtx;
use crate::types::{
    CLOSE_BUTTON_DETAIL, CLOSE_BUTTON_HEIGHT, CLOSE_BUTTON_WIDTH, Gesture, Monitor, MonitorId,
    Rect, WindowId,
};
use std::collections::HashMap;

const STARTMENU_ICON_SIZE: i32 = 14;
const STARTMENU_ICON_INNER: i32 = 6;
const TAG_DETAIL_BAR_HEIGHT_NORMAL: i32 = 4;
const TAG_DETAIL_BAR_HEIGHT_HOVER: i32 = 8;

#[derive(Clone)]
pub(crate) struct TagCellSnapshot {
    pub slot: usize,
    pub label: String,
    pub scheme: BarScheme,
}

#[derive(Clone)]
pub(crate) struct TitleCellSnapshot {
    pub win: WindowId,
    pub name: String,
    pub scheme: BarScheme,
    pub close_scheme: Option<BarScheme>,
}

#[derive(Clone)]
pub(crate) struct SystraySnapshot {
    pub items: crate::types::WaylandSystray,
    pub menu: Option<crate::types::WaylandSystrayMenu>,
    pub spacing: i32,
    pub base_scheme: BarScheme,
}

#[derive(Clone)]
pub(crate) struct MonitorBarSnapshot {
    pub monitor_id: MonitorId,
    pub origin_x: i32,
    pub origin_y: i32,
    pub width: i32,
    pub height: i32,
    pub font_size: f32,
    pub is_selected_monitor: bool,
    pub status_scheme: BarScheme,
    pub startmenu_size: i32,
    pub horizontal_padding: i32,
    pub gesture: Gesture,
    pub layout_symbol: String,
    pub tags: Vec<TagCellSnapshot>,
    pub show_shutdown: bool,
    pub titles: Vec<TitleCellSnapshot>,
    pub monitor_rect_x: i32,
    pub status_text: Option<String>,
    pub status_items: Vec<crate::bar::status::StatusItem>,
    pub systray: Option<SystraySnapshot>,
}

pub(crate) struct MonitorRenderOutput {
    pub hit_cache: crate::bar::MonitorHitCache,
    pub bar_clients_width: i32,
    pub activeoffset: u32,
}

pub(crate) struct MonitorRenderOutputWithId {
    pub monitor_id: MonitorId,
    pub output: MonitorRenderOutput,
}

#[derive(Clone)]
pub(crate) struct WorkerTrayLayout {
    pub tray_total_w: i32,
    pub tray_start_x: i32,
    pub tray_slots: Vec<crate::bar::SystrayHitSlot>,
    pub menu_total_w: i32,
    pub menu_start_x: i32,
    pub menu_slots: Vec<crate::bar::SystrayHitSlot>,
}

pub(crate) fn build_monitor_snapshots(
    core: &mut CoreCtx,
    wayland_systray: Option<(
        &crate::types::WaylandSystray,
        Option<&crate::types::WaylandSystrayMenu>,
    )>,
    include_status_items: bool,
) -> Vec<MonitorBarSnapshot> {
    let selected_monitor_num = core.globals().selected_monitor().num;
    let show_systray = core.globals().cfg.show_systray;
    let systray_spacing = core.globals().cfg.systray_spacing;
    let base_font_size =
        crate::wayland::common::wayland_font_size_from_config(&core.globals().cfg.fonts);
    let drag_bar_active = core.globals().drag.bar_active;
    let current_mode = core.globals().behavior.current_mode.clone();
    let status_text = if crate::overview::is_mode_name(&current_mode) {
        Some("mode: overview".to_string())
    } else if !current_mode.is_empty() && current_mode != "default" {
        let mode_display = core
            .globals()
            .cfg
            .modes
            .get(&current_mode)
            .and_then(|m| m.description.as_ref())
            .cloned()
            .unwrap_or_else(|| current_mode.clone());
        Some(format!("mode: {}", mode_display))
    } else {
        let status_text = core.globals().bar_runtime.status_text.clone();
        if status_text.is_empty() {
            None
        } else {
            Some(status_text)
        }
    };
    let monitors: Vec<Monitor> = core.globals().monitors_iter_all().cloned().collect();
    let mut monitor_stats: HashMap<MonitorId, crate::bar::model::ClientBarStats> = HashMap::new();
    for client in core.globals().clients.values() {
        let entry = monitor_stats.entry(client.monitor_id).or_default();
        entry.occupied_tags = entry.occupied_tags | client.tags;
        if client.is_urgent {
            entry.urgent_tags = entry.urgent_tags | client.tags;
        }
    }

    let mut snapshots = Vec::new();
    for mon in monitors {
        if !crate::bar::monitor_bar_visible(core.globals(), &mon) {
            continue;
        }
        let font_size = (base_font_size * mon.ui_scale as f32).max(1.0);

        let mut stats = monitor_stats.get(&mon.id()).copied().unwrap_or_default();
        stats.occupied_tags = stats.occupied_tags.without_scratchpad();

        let is_selected_monitor = mon.num == selected_monitor_num;
        let gesture = if is_selected_monitor {
            core.globals().selected_monitor().gesture
        } else {
            Gesture::None
        };
        let mut tags = Vec::new();
        for tag in crate::tags::bar::visible_tags_ctx(core, &mon, stats.occupied_tags) {
            let is_hover = gesture == Gesture::Tag(tag.slot);
            let mut scheme = core.globals().tag_scheme(
                &mon,
                tag.tag_index as u32,
                stats.occupied_tags,
                stats.urgent_tags,
                is_hover,
            );
            if is_hover && drag_bar_active {
                scheme = core.globals().tag_hover_fill_scheme();
            }
            tags.push(TagCellSnapshot {
                slot: tag.slot,
                label: tag.label.to_string(),
                scheme,
            });
        }

        let selected_tags = mon.selected_tags();
        let mut titles = Vec::new();
        for (_c_win, c) in mon
            .iter_clients(core.globals().clients.map())
            .filter(|(_, c)| c.shows_in_bar(selected_tags))
        {
            stats.visible_clients += 1;
            let is_hover = gesture == Gesture::WinTitle(c.win);
            let scheme = core.globals().window_scheme(c, is_hover);
            let close_scheme = if is_selected_monitor && mon.sel == Some(c.win) {
                let is_fullscreen = c.mode.is_fullscreen();
                Some(core.globals().close_button_scheme(
                    gesture == Gesture::CloseButton,
                    c.is_locked,
                    is_fullscreen,
                ))
            } else {
                None
            };
            titles.push(TitleCellSnapshot {
                win: c.win,
                name: c.name.clone(),
                scheme,
                close_scheme,
            });
        }

        let status_text = if is_selected_monitor {
            status_text.clone()
        } else {
            None
        };
        let status_items = if include_status_items {
            status_text
                .as_deref()
                .map(|text| core.bar.status_items_for_text(text).to_vec())
                .unwrap_or_default()
        } else {
            Vec::new()
        };

        let systray = if show_systray && is_selected_monitor {
            wayland_systray.map(|(items, menu)| SystraySnapshot {
                items: items.clone(),
                menu: menu.cloned(),
                spacing: systray_spacing,
                base_scheme: core.globals().status_scheme(),
            })
        } else {
            None
        };

        snapshots.push(MonitorBarSnapshot {
            monitor_id: mon.id(),
            origin_x: mon.work_rect.x,
            origin_y: mon.bar_y,
            width: mon.work_rect.w,
            height: mon.bar_height,
            font_size,
            is_selected_monitor,
            status_scheme: core.globals().status_scheme(),
            startmenu_size: mon.startmenu_size,
            horizontal_padding: mon.horizontal_padding,
            gesture,
            layout_symbol: if crate::overview::is_active(core)
                && core.globals().selected_monitor_id() == mon.id()
            {
                "OVR".to_string()
            } else {
                mon.layouts_for_mask(selected_tags).symbol().to_string()
            },
            tags,
            show_shutdown: mon.sel.is_none(),
            titles,
            monitor_rect_x: mon.monitor_rect.x,
            status_text,
            status_items,
            systray,
        });
    }

    snapshots
}

fn draw_startmenu_icon_snapshot(
    painter: &mut dyn BarPainter,
    scheme: &BarScheme,
    startmenu_size: i32,
    gesture: Gesture,
    bar_height: i32,
) {
    let icon_offset = (bar_height - CLOSE_BUTTON_WIDTH) / 2;
    let startmenu_invert = gesture == Gesture::StartMenu;
    painter.set_scheme(scheme.clone());
    painter.rect(
        Rect::new(0, 0, startmenu_size, bar_height),
        true,
        !startmenu_invert,
    );
    painter.rect(
        Rect::new(5, icon_offset, STARTMENU_ICON_SIZE, STARTMENU_ICON_SIZE),
        true,
        startmenu_invert,
    );
    painter.rect(
        Rect::new(
            9,
            icon_offset + 4,
            STARTMENU_ICON_INNER,
            STARTMENU_ICON_INNER,
        ),
        true,
        !startmenu_invert,
    );
    painter.rect(
        Rect::new(
            19,
            icon_offset + STARTMENU_ICON_SIZE,
            STARTMENU_ICON_INNER,
            STARTMENU_ICON_INNER,
        ),
        true,
        startmenu_invert,
    );
}

fn draw_shutdown_button_snapshot(
    painter: &mut dyn BarPainter,
    scheme: &BarScheme,
    x: i32,
    bar_height: i32,
) -> i32 {
    painter.set_scheme(scheme.clone());
    painter.rect(Rect::new(x, 0, bar_height, bar_height), true, true);

    let icon_size = bar_height * 5 / 8;
    let icon_x = x + (bar_height - icon_size) / 2;
    let icon_y = (bar_height - icon_size) / 2;
    let stroke = (icon_size / 6).max(2);
    let gap = stroke;
    let stem_w = stroke;
    let stem_h = icon_size / 2;
    let stem_x = icon_x + (icon_size - stem_w) / 2;
    let stem_y = icon_y;
    let arc_x = icon_x;
    let arc_y = icon_y + gap + stroke;
    let arc_w = stroke;
    let arc_h = icon_size - gap - stroke;
    let bot_x = icon_x + stroke;
    let bot_y = icon_y + icon_size - stroke;
    let bot_w = (icon_size - stroke * 2).max(0);
    let bot_h = stroke;

    painter.rect(Rect::new(stem_x, stem_y, stem_w, stem_h), true, false);
    painter.rect(Rect::new(arc_x, arc_y, arc_w, arc_h), true, false);
    painter.rect(
        Rect::new(arc_x + icon_size - stroke, arc_y, arc_w, arc_h),
        true,
        false,
    );
    painter.rect(Rect::new(bot_x, bot_y, bot_w, bot_h), true, false);

    x + bar_height
}

fn draw_close_button_snapshot(
    painter: &mut dyn BarPainter,
    scheme: &BarScheme,
    is_hover: bool,
    x: i32,
    bar_height: i32,
) {
    let mut scheme = scheme.clone();
    scheme.fg = scheme.detail;
    painter.set_scheme(scheme);
    let button_x = x + bar_height / 6;
    let detail_offset = if is_hover { CLOSE_BUTTON_DETAIL } else { 0 };
    let button_y = (bar_height - CLOSE_BUTTON_WIDTH) / 2 - detail_offset;
    painter.rect(
        Rect::new(button_x, button_y, CLOSE_BUTTON_WIDTH, CLOSE_BUTTON_HEIGHT),
        true,
        true,
    );
    painter.rect(
        Rect::new(
            button_x,
            (bar_height - CLOSE_BUTTON_WIDTH) / 2 + CLOSE_BUTTON_HEIGHT - detail_offset,
            CLOSE_BUTTON_WIDTH,
            CLOSE_BUTTON_DETAIL + detail_offset,
        ),
        true,
        false,
    );
}

pub(crate) fn worker_systray_layout(
    snapshot: &SystraySnapshot,
    monitor_width: i32,
    icon_h: i32,
) -> WorkerTrayLayout {
    let spacing = snapshot.spacing.max(0);
    let mut tray_total_w = 0;
    if !snapshot.items.items.is_empty() {
        tray_total_w = spacing;
        for item in &snapshot.items.items {
            tray_total_w +=
                crate::systray::wayland::scale_icon_width(item.icon_w, item.icon_h, icon_h)
                    + spacing;
        }
    }
    let tray_start_x = monitor_width - tray_total_w;

    let mut tray_slots = Vec::new();
    let mut x = tray_start_x + spacing;
    for (idx, item) in snapshot.items.items.iter().enumerate() {
        let w = crate::systray::wayland::scale_icon_width(item.icon_w, item.icon_h, icon_h);
        if w > 0 && item.icon_w > 0 && item.icon_h > 0 {
            tray_slots.push(crate::bar::SystrayHitSlot {
                idx,
                start: x,
                end: x + w,
            });
        }
        x += w + spacing;
    }

    let mut menu_total_w = 0;
    let mut menu_slots = Vec::new();
    let mut menu_start_x = 0;
    if let Some(menu) = &snapshot.menu {
        for item in &menu.items {
            menu_total_w += item.width.max(24);
        }
        menu_start_x = (tray_start_x - menu_total_w).max(0);
        let mut mx = menu_start_x;
        for (idx, item) in menu.items.iter().enumerate() {
            let w = item.width.max(24);
            menu_slots.push(crate::bar::SystrayHitSlot {
                idx,
                start: mx,
                end: mx + w,
            });
            mx += w;
        }
    }

    WorkerTrayLayout {
        tray_total_w,
        tray_start_x,
        tray_slots,
        menu_total_w,
        menu_start_x,
        menu_slots,
    }
}

fn render_monitor_snapshot_base(
    snapshot: &MonitorBarSnapshot,
    painter: &mut dyn BarPainter,
) -> MonitorRenderOutput {
    let bar_height = snapshot.height;
    let tray_layout = snapshot
        .systray
        .as_ref()
        .map(|s| worker_systray_layout(s, snapshot.width, bar_height.max(1)));
    let systray_width = if snapshot.is_selected_monitor {
        tray_layout.as_ref().map(|l| l.tray_total_w).unwrap_or(0)
    } else {
        0
    };

    let mut hit = crate::bar::MonitorHitCache::default();
    let mut temp_mon = Monitor::default();
    temp_mon.work_rect.w = snapshot.width;

    let (status_start_x, status_width, status_click_targets) =
        if snapshot.is_selected_monitor && !snapshot.status_items.is_empty() {
            crate::bar::status::draw_status_items(
                systray_width,
                &temp_mon,
                bar_height,
                snapshot.status_items.as_slice(),
                snapshot.status_scheme.clone(),
                painter,
            )
        } else {
            (0, 0, Vec::new())
        };
    hit.status_click_targets = status_click_targets;

    draw_startmenu_icon_snapshot(
        painter,
        &snapshot.status_scheme,
        snapshot.startmenu_size,
        snapshot.gesture,
        bar_height,
    );

    let mut x = snapshot.startmenu_size;
    for tag in &snapshot.tags {
        let text_w = painter.text_width(&tag.label);
        let width = (text_w + snapshot.horizontal_padding).max(snapshot.horizontal_padding);
        painter.set_scheme(tag.scheme.clone());
        let detail_height = if snapshot.gesture == Gesture::Tag(tag.slot) {
            TAG_DETAIL_BAR_HEIGHT_HOVER
        } else {
            TAG_DETAIL_BAR_HEIGHT_NORMAL
        };
        let lpad = (snapshot.horizontal_padding / 2).max(0);
        x = painter.text(
            Rect::new(x, 0, width, bar_height),
            lpad,
            &tag.label,
            false,
            detail_height,
        );
        hit.tag_ranges.push(crate::bar::TagHitRange {
            start: x - width,
            end: x,
            tag_index: tag.slot,
        });
    }

    let text_w = painter.text_width(&snapshot.layout_symbol);
    let layout_w = (text_w + snapshot.horizontal_padding).max(snapshot.horizontal_padding);
    let lpad = ((layout_w - text_w) / 2).max(0);
    painter.set_scheme(snapshot.status_scheme.clone());
    let layout_start = x;
    x = painter.text(
        Rect::new(x, 0, layout_w, bar_height),
        lpad,
        &snapshot.layout_symbol,
        false,
        0,
    );
    hit.layout_start = layout_start;
    hit.layout_end = x;

    if snapshot.show_shutdown {
        x = draw_shutdown_button_snapshot(painter, &snapshot.status_scheme, x, bar_height);
    }
    hit.shutdown_end = x;

    let title_end_x = if snapshot.is_selected_monitor && status_width > 0 {
        status_start_x
    } else {
        snapshot.width - systray_width
    };
    let title_width = (title_end_x - x).max(0);
    hit.status_hit_x = if snapshot.is_selected_monitor && status_width > 0 {
        status_start_x
    } else {
        snapshot.width - systray_width
    };

    let mut activeoffset = 0u32;
    if !snapshot.titles.is_empty() {
        let total_width = title_width + 1;
        let each_width = total_width / snapshot.titles.len() as i32;
        let mut remainder = total_width % snapshot.titles.len() as i32;
        let mut title_x = x;
        for title in &snapshot.titles {
            let this_width = if remainder > 0 {
                remainder -= 1;
                each_width + 1
            } else {
                each_width
            };
            let text_w = painter.text_width(&title.name);
            painter.set_scheme(title.scheme.clone());
            let lpad = if text_w < this_width - 64 {
                ((this_width - text_w) as f32 * 0.5) as i32
            } else {
                snapshot.horizontal_padding / 2 + if this_width >= 32 { 20 } else { 0 }
            };
            painter.text(
                Rect::new(title_x, 0, this_width, bar_height),
                lpad,
                &title.name,
                false,
                4,
            );
            if let Some(close_scheme) = &title.close_scheme {
                if this_width >= 32 {
                    draw_close_button_snapshot(
                        painter,
                        close_scheme,
                        snapshot.gesture == Gesture::CloseButton,
                        title_x,
                        bar_height,
                    );
                }
                activeoffset = (snapshot.monitor_rect_x + title_x) as u32;
            }
            hit.title_ranges.push(crate::bar::TitleHitRange {
                start: title_x,
                end: title_x + this_width,
                win: title.win,
            });
            title_x += this_width;
        }
    } else {
        painter.set_scheme(snapshot.status_scheme.clone());
        painter.rect(Rect::new(x, 0, title_width, bar_height), true, true);
    }

    if let (Some(_systray), Some(layout)) = (&snapshot.systray, &tray_layout) {
        hit.systray_slots = layout.tray_slots.clone();
        hit.systray_menu_slots = layout.menu_slots.clone();
    }

    MonitorRenderOutput {
        hit_cache: hit,
        bar_clients_width: title_width,
        activeoffset,
    }
}

pub(crate) fn render_monitor_snapshot(
    snapshot: &MonitorBarSnapshot,
    painter: &mut dyn BarPainter,
) -> MonitorRenderOutput {
    render_monitor_snapshot_base(snapshot, painter)
}
