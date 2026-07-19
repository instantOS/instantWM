use crate::bar::paint::{BarPainter, BarScheme};
use crate::contexts::CoreCtx;
use crate::types::{
    CLOSE_BUTTON_DETAIL, CLOSE_BUTTON_HEIGHT, CLOSE_BUTTON_WIDTH, Gesture, MonitorId, Rect,
    WindowId,
};

const STARTMENU_ICON_SIZE: i32 = 14;
const STARTMENU_ICON_INNER: i32 = 6;
const TAG_DETAIL_BAR_HEIGHT_NORMAL: i32 = 4;
const TAG_DETAIL_BAR_HEIGHT_HOVER: i32 = 8;

#[derive(Clone)]
pub(crate) struct TagCellSnapshot {
    pub slot: usize,
    pub tag_index: usize,
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
    pub items: crate::systray::StatusNotifierTray,
    pub visual_padding: i32,
    pub base_scheme: BarScheme,
}

#[derive(Clone)]
pub(crate) struct StatusContent {
    pub text: String,
    pub items: Vec<crate::bar::status::StatusItem>,
}

#[derive(Clone)]
pub(crate) enum StatusPresentation {
    Hidden,
    Runtime(StatusContent),
    WmMode {
        name: String,
        content: StatusContent,
    },
    Overview(StatusContent),
}

impl StatusPresentation {
    pub fn content(&self) -> Option<&StatusContent> {
        match self {
            Self::Hidden => None,
            Self::Runtime(content) | Self::WmMode { content, .. } | Self::Overview(content) => {
                Some(content)
            }
        }
    }

    pub fn ensure_items_parsed(&mut self) {
        let content = match self {
            Self::Hidden => return,
            Self::Runtime(content) | Self::WmMode { content, .. } | Self::Overview(content) => {
                content
            }
        };
        if content.items.is_empty() {
            content.items = crate::bar::status::parse_status(content.text.as_bytes()).items;
        }
    }
}

#[derive(Clone)]
pub(crate) enum BarOverlay {
    TrayMenu(crate::systray::TrayMenuPresentation),
}

/// Immutable, derived presentation consumed by both bar renderers.
///
/// Runtime mode and tray-menu session state remain authoritative elsewhere;
/// snapshots never become a second source of truth.
#[derive(Clone)]
pub(crate) struct BarPresentation {
    pub status: StatusPresentation,
    pub overlay: Option<BarOverlay>,
}

impl BarPresentation {
    pub fn tray_menu(&self) -> Option<&crate::systray::TrayMenuPresentation> {
        match self.overlay.as_ref() {
            Some(BarOverlay::TrayMenu(menu)) => Some(menu),
            None => None,
        }
    }
}

#[derive(Clone)]
pub(crate) struct MonitorBarSnapshot {
    pub monitor_id: MonitorId,
    pub rect: Rect,
    pub font_size: f32,
    pub font_families: Vec<String>,
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
    pub presentation: BarPresentation,
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

pub(crate) fn build_monitor_snapshots(
    core: &mut CoreCtx,
    status_notifier_tray: Option<&crate::systray::StatusNotifierTray>,
    tray_menu: Option<&crate::systray::TrayMenuPresentation>,
    include_status_items: bool,
) -> Vec<MonitorBarSnapshot> {
    let selected_monitor_num = core.model().selected_monitor().num;
    let show_systray = core.config().systray.show;
    let systray_spacing = core.config().systray.spacing;
    let base_font_size = crate::wayland::common::font_size_from_config(&core.config().fonts.fonts);
    let font_families =
        crate::wayland::common::font_families_from_config(&core.config().fonts.fonts);
    let drag_bar_active = core.drag_state().bar_active;
    let selected_status =
        match core.behavior().current_mode.clone() {
            crate::core_state::ActiveWmMode::Overview => StatusPresentation::Overview(
                status_content(core, "mode: overview".to_string(), include_status_items),
            ),
            crate::core_state::ActiveWmMode::Named(name) => {
                let mode_display = core
                    .state()
                    .config
                    .bindings
                    .modes
                    .get(&name)
                    .and_then(|mode| mode.description.as_ref())
                    .cloned()
                    .unwrap_or_else(|| name.clone());
                StatusPresentation::WmMode {
                    name,
                    content: status_content(
                        core,
                        format!("mode: {mode_display}"),
                        include_status_items,
                    ),
                }
            }
            crate::core_state::ActiveWmMode::Default => {
                let text = core.bar.runtime.status_text.clone();
                if text.is_empty() {
                    StatusPresentation::Hidden
                } else {
                    StatusPresentation::Runtime(status_content(core, text, include_status_items))
                }
            }
        };
    let monitor_ids: Vec<MonitorId> = core.model().monitors_iter().map(|(id, _)| id).collect();

    let mut snapshots = Vec::new();
    for monitor_id in monitor_ids {
        let Some(mon) = core.model().monitor(monitor_id) else {
            continue;
        };
        if !mon.bar_visible(&core.model().clients) {
            continue;
        }
        let font_size = (base_font_size * mon.ui_scale as f32).max(1.0);

        let mut stats = crate::bar::model::ClientBarStats::collect(mon, core.model());

        let is_selected_monitor = mon.num == selected_monitor_num;
        let gesture = if is_selected_monitor {
            core.model().selected_monitor().gesture
        } else {
            Gesture::None
        };
        let mut tags = Vec::new();
        for tag in crate::tags::bar::visible_tags(core.state(), core.bar, mon, stats.occupied_tags)
        {
            let is_hover = gesture == Gesture::Tag(tag.slot);
            let mut scheme = core.tag_scheme(
                mon,
                tag.tag_index as u32,
                stats.occupied_tags,
                stats.urgent_tags,
                is_hover,
            );
            if is_hover && drag_bar_active {
                scheme = core.tag_hover_fill_scheme();
            }
            tags.push(TagCellSnapshot {
                slot: tag.slot,
                tag_index: tag.tag_index,
                label: tag.label.to_string(),
                scheme,
            });
        }

        let selected_tags = mon.selected_tags();
        let mut titles = Vec::new();
        for (_c_win, c) in mon
            .iter_clients(&core.model().clients)
            .filter(|(_, c)| c.shows_in_bar(selected_tags))
        {
            stats.visible_clients += 1;
            let is_hover = gesture == Gesture::WinTitle(c.win);
            let scheme = core.window_scheme(c, is_hover);
            let close_scheme = if is_selected_monitor && mon.selected == Some(c.win) {
                let is_fullscreen = c.mode.is_fullscreen();
                Some(core.close_button_scheme(
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

        let presentation = BarPresentation {
            status: if is_selected_monitor {
                selected_status.clone()
            } else {
                StatusPresentation::Hidden
            },
            overlay: if is_selected_monitor {
                tray_menu.cloned().map(BarOverlay::TrayMenu)
            } else {
                None
            },
        };

        let systray = if show_systray && is_selected_monitor {
            status_notifier_tray.map(|items| SystraySnapshot {
                items: items.clone(),
                visual_padding: systray_spacing,
                base_scheme: core.status_scheme(),
            })
        } else {
            None
        };

        snapshots.push(MonitorBarSnapshot {
            monitor_id: mon.id(),
            rect: Rect::new(
                mon.work_rect().x,
                mon.bar_y(),
                mon.work_rect().w,
                mon.bar_height,
            ),
            font_size,
            font_families: font_families.clone(),
            is_selected_monitor,
            status_scheme: core.status_scheme(),
            startmenu_size: mon.startmenu_size,
            horizontal_padding: mon.horizontal_padding,
            gesture,
            layout_symbol: if core.model().is_overview_active_on(mon) {
                "OVR".to_string()
            } else {
                mon.layouts_for_mask(selected_tags).symbol().to_string()
            },
            tags,
            show_shutdown: mon.selected.is_none(),
            titles,
            monitor_rect_x: mon.monitor_rect.x,
            presentation,
            systray,
        });
    }

    snapshots
}

fn status_content(core: &mut CoreCtx, text: String, include_items: bool) -> StatusContent {
    let items = if include_items {
        core.bar.status_items_for_text(&text).to_vec()
    } else {
        Vec::new()
    };
    StatusContent { text, items }
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

    let symbol = "\u{f011}";
    let text_w = painter.text_width(symbol);
    let lpad = ((bar_height - text_w) / 2).max(0);
    painter.text(
        Rect::new(x, 0, bar_height, bar_height),
        lpad,
        symbol,
        true,
        0,
    );

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
    scheme.foreground = scheme.detail;
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

fn render_monitor_snapshot_base(
    snapshot: &MonitorBarSnapshot,
    painter: &mut dyn BarPainter,
) -> MonitorRenderOutput {
    let bar_height = snapshot.rect.h;
    let tray_layout = snapshot.systray.as_ref().map(|s| {
        let menu = snapshot.presentation.tray_menu().map(|menu| &menu.view);
        crate::systray::layout(
            &s.items,
            menu,
            snapshot.rect.w,
            bar_height,
            s.visual_padding,
        )
    });
    let systray_width = if snapshot.is_selected_monitor {
        tray_layout.as_ref().map(|l| l.total_width).unwrap_or(0)
    } else {
        0
    };

    let mut hit = crate::bar::MonitorHitCache::default();

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
            tag_index: tag.tag_index,
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

    let status_output = if snapshot.is_selected_monitor
        && snapshot
            .presentation
            .status
            .content()
            .is_some_and(|content| !content.items.is_empty())
    {
        let content = snapshot.presentation.status.content().unwrap();
        let status_right = snapshot.rect.w - systray_width;
        crate::bar::status::draw_status_items(
            Rect::new(x, 0, (status_right - x).max(0), bar_height),
            content.items.as_slice(),
            snapshot.status_scheme.clone(),
            painter,
        )
    } else {
        crate::bar::status::StatusRenderOutput::default()
    };
    hit.status_click_targets = status_output.click_targets;

    let title_end_x = if status_output.bounds.w > 0 {
        status_output.bounds.x
    } else {
        snapshot.rect.w - systray_width
    };
    let title_width = (title_end_x - x).max(0);
    hit.status_hit_x = if status_output.bounds.w > 0 {
        status_output.bounds.x
    } else {
        snapshot.rect.w - systray_width
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
        hit.systray_slots = layout
            .cells
            .iter()
            .map(|cell| crate::bar::SystrayHitSlot {
                idx: cell.idx,
                start: cell.hit_start,
                end: cell.hit_end,
            })
            .collect();
        if layout.menu_width > 0 {
            hit.overlay = Some(crate::bar::BarOverlayHit::TrayMenu {
                start: layout.menu_start_x,
                end: layout.menu_start_x + layout.menu_width,
                slots: layout.menu_cells.clone(),
            });
        }
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
