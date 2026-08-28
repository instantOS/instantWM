use crate::bar::paint::{BarPainter, BarScheme, TextOverflow};
use crate::contexts::CoreCtx;
use crate::types::{
    CLOSE_BUTTON_DETAIL, CLOSE_BUTTON_HEIGHT, CLOSE_BUTTON_WIDTH, Client, CloseButtonColorConfigs,
    Gesture, Monitor, MonitorId, Rect, SchemeClose, SchemeHover, SchemeTag, SchemeWin,
    StatusColorConfig, TagColorConfigs, TagMask, WindowColorConfigs, WindowId,
};

const STARTMENU_ICON_SIZE: i32 = 14;
const STARTMENU_ICON_INNER: i32 = 6;
const TAG_DETAIL_BAR_HEIGHT_NORMAL: i32 = 4;
const TAG_DETAIL_BAR_HEIGHT_HOVER: i32 = 8;

use crate::types::geometry::scaled_px;

fn status_scheme(colors: &StatusColorConfig) -> BarScheme {
    BarScheme::from(&colors.as_scheme())
}

fn tag_hover_fill_scheme(colors: &TagColorConfigs) -> BarScheme {
    colors
        .colors_for(SchemeHover::Hover, SchemeTag::Filled)
        .into()
}

fn tag_scheme(
    model: &crate::model::WmModel,
    monitor: &Monitor,
    tag_index: u32,
    occupied_tags: TagMask,
    urgent_tags: TagMask,
    is_hover: bool,
) -> BarScheme {
    let tag_num = tag_index as usize + 1;
    let tag_role = if urgent_tags.contains(tag_num) {
        SchemeTag::Urgent
    } else if occupied_tags.contains(tag_num) {
        let selected_monitor = model.selected_monitor();
        let selected_client_has_tag = selected_monitor
            .and_then(|monitor| monitor.selected)
            .and_then(|win| model.client(win))
            .is_some_and(|client| client.tags.contains(tag_num));

        if selected_monitor.is_some_and(|selected| selected.num == monitor.num)
            && selected_client_has_tag
        {
            SchemeTag::Focus
        } else if monitor.visible_tags().contains(tag_num) {
            SchemeTag::NoFocus
        } else if !monitor.hide_tags {
            SchemeTag::Filled
        } else {
            SchemeTag::Inactive
        }
    } else if monitor.visible_tags().contains(tag_num) {
        SchemeTag::Empty
    } else {
        SchemeTag::Inactive
    };

    model
        .tags
        .colors
        .colors_for(
            if is_hover {
                SchemeHover::Hover
            } else {
                SchemeHover::NoHover
            },
            tag_role,
        )
        .into()
}

fn window_scheme(
    model: &crate::model::WmModel,
    colors: &WindowColorConfigs,
    client: &Client,
    is_hover: bool,
) -> BarScheme {
    let is_selected = model
        .selected_monitor()
        .and_then(|monitor| monitor.selected)
        == Some(client.win);
    let is_edge_scratchpad = client.is_edge_scratchpad();
    let window_role = if is_selected {
        if is_edge_scratchpad {
            SchemeWin::EdgeScratchpadFocus
        } else if client.is_sticky {
            SchemeWin::StickyFocus
        } else {
            SchemeWin::Focus
        }
    } else if is_edge_scratchpad {
        SchemeWin::EdgeScratchpad
    } else if client.is_sticky {
        SchemeWin::Sticky
    } else if client.is_minimized() {
        SchemeWin::Minimized
    } else if client.is_urgent {
        SchemeWin::Urgent
    } else {
        SchemeWin::Normal
    };

    colors
        .colors_for(
            if is_hover {
                SchemeHover::Hover
            } else {
                SchemeHover::NoHover
            },
            window_role,
        )
        .into()
}

fn close_button_scheme(
    colors: &CloseButtonColorConfigs,
    is_hover: bool,
    is_locked: bool,
    is_fullscreen: bool,
) -> BarScheme {
    let close_role = if is_locked {
        SchemeClose::Locked
    } else if is_fullscreen {
        SchemeClose::Fullscreen
    } else {
        SchemeClose::Normal
    };

    colors
        .colors_for(
            if is_hover {
                SchemeHover::Hover
            } else {
                SchemeHover::NoHover
            },
            close_role,
        )
        .into()
}

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
    pub base_scheme: BarScheme,
    pub layout: crate::systray::TrayLayout,
}

#[derive(Clone)]
pub(crate) struct StatusContent {
    pub text: String,
    pub items: Vec<crate::bar::status::StatusItem>,
    pub click_events: bool,
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
    /// Effective logical UI scale used for decorative metrics that are not
    /// derived from font advances or bar height.
    pub ui_scale: f64,
    pub fonts: crate::core_state::FontConfig,
    pub is_selected_monitor: bool,
    pub status_scheme: BarScheme,
    pub status_separator_color: crate::types::color::Rgba,
    pub status_hover_color: crate::types::color::Rgba,
    pub startmenu_size: i32,
    pub horizontal_padding: i32,
    pub gesture: Gesture,
    pub layout_symbol: String,
    pub tags: Vec<TagCellSnapshot>,
    pub show_shutdown: bool,
    pub titles: Vec<TitleCellSnapshot>,
    pub presentation: BarPresentation,
    pub systray: Option<SystraySnapshot>,
    /// Right-edge width occupied by content rendered outside this scene.
    /// XEmbed children use this reservation; compositor-rendered tray items
    /// remain represented by `systray` above.
    pub external_right_width: i32,
}

pub(crate) struct MonitorRenderOutput {
    pub hit_cache: crate::bar::MonitorHitCache,
    pub bar_clients_width: i32,
}

pub(crate) struct MonitorRenderOutputWithId {
    pub monitor_id: MonitorId,
    pub output: MonitorRenderOutput,
}

pub(crate) fn build_monitor_snapshots(
    core: &mut CoreCtx,
    include_status_items: bool,
    external_right_width: i32,
) -> Vec<MonitorBarSnapshot> {
    let selected_monitor_num = core.model().expect_selected_monitor().num;
    let show_systray = core.config().systray.show;
    let systray_spacing = core.config().systray.spacing;
    let base_fonts = core.config().fonts.clone();
    let bar_hover = core.bar.hover;
    enum ModeStatus {
        Default,
        Overview,
        Named { name: String, display: String },
    }
    let mode_status = match &core.behavior().current_mode {
        crate::core_state::ActiveWmMode::Overview => ModeStatus::Overview,
        crate::core_state::ActiveWmMode::Named(name) => {
            let display = core
                .config()
                .bindings
                .modes
                .get(name)
                .and_then(|mode| mode.description.as_ref())
                .cloned()
                .unwrap_or_else(|| name.clone());
            ModeStatus::Named {
                name: name.clone(),
                display,
            }
        }
        crate::core_state::ActiveWmMode::TreePlacement(_) => {
            let name = crate::core_state::TREE_PLACEMENT_MODE_NAME.to_string();
            let display = core
                .config()
                .bindings
                .modes
                .get(&name)
                .and_then(|mode| mode.description.as_ref())
                .cloned()
                .unwrap_or_else(|| "place window".to_string());
            ModeStatus::Named { name, display }
        }
        crate::core_state::ActiveWmMode::Default => ModeStatus::Default,
    };
    let selected_status = match mode_status {
        ModeStatus::Overview => StatusPresentation::Overview(status_content(
            core,
            "mode: overview".to_string(),
            include_status_items,
            false,
        )),
        ModeStatus::Named { name, display } => StatusPresentation::WmMode {
            name,
            content: status_content(
                core,
                format!("mode: {display}"),
                include_status_items,
                false,
            ),
        },
        ModeStatus::Default => {
            let text = core.bar.runtime.status_text.clone();
            if text.is_empty() {
                StatusPresentation::Hidden
            } else {
                let click_events = core.bar.runtime.status_click_events;
                StatusPresentation::Runtime(status_content(
                    core,
                    text,
                    include_status_items,
                    click_events,
                ))
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
        let fonts = base_fonts.scaled(mon.ui_scale as f32);

        let stats = crate::bar::model::ClientBarStats::collect(mon, core.model());

        let is_selected_monitor = mon.num == selected_monitor_num;
        let gesture = bar_hover.gesture_on(monitor_id);
        let mut tags = Vec::new();
        for tag in crate::tags::bar::visible_tags(core.state(), mon, stats.occupied_tags) {
            let is_hover = gesture == Gesture::Tag(tag.slot);
            let mut scheme = tag_scheme(
                core.model(),
                mon,
                tag.tag_index as u32,
                stats.occupied_tags,
                stats.urgent_tags,
                is_hover,
            );
            if is_hover && bar_hover.drag_active {
                scheme = tag_hover_fill_scheme(&core.model().tags.colors);
            }
            tags.push(TagCellSnapshot {
                slot: tag.slot,
                tag_index: tag.tag_index,
                label: tag.label.to_string(),
                scheme,
            });
        }

        let selected_tags = mon.visible_tags();
        let mut titles = Vec::new();
        for win in mon.bar_client_order(&core.model().clients) {
            let Some(c) = core.model().client(win) else {
                continue;
            };
            let is_hover = gesture == Gesture::WinTitle(c.win);
            let scheme = window_scheme(core.model(), &core.config().colors.window, c, is_hover);
            let close_scheme = if is_selected_monitor && mon.selected == Some(c.win) {
                let is_fullscreen = c.mode().is_fullscreen();
                Some(close_button_scheme(
                    &core.config().colors.close_button,
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

        // While the external instantMENU presents the tray menu, the bar
        // neither reserves width for it nor renders the overlay.
        let instantmenu_hosts_menu = core
            .bar
            .systray_host
            .instantmenu
            .hosting(core.config().systray.menu_backend);
        let presentation = BarPresentation {
            status: if is_selected_monitor {
                selected_status.clone()
            } else {
                StatusPresentation::Hidden
            },
            overlay: if is_selected_monitor && !instantmenu_hosts_menu {
                core.bar
                    .systray_host
                    .menu
                    .presentation()
                    .map(BarOverlay::TrayMenu)
            } else {
                None
            },
        };

        let tray_menu = presentation.tray_menu().cloned();
        let systray = if show_systray && is_selected_monitor {
            let items = core.bar.systray_host.tray.clone();
            let layout = crate::systray::layout(
                &items,
                tray_menu.as_ref().map(|menu| &menu.view),
                mon.work_rect().w,
                mon.bar_height,
                systray_spacing,
                // Compositor-rendered icons sit left of any externally
                // rendered tray strip (XEmbed windows on X11).
                external_right_width.max(0),
            );
            Some(SystraySnapshot {
                items,
                base_scheme: status_scheme(&core.config().colors.status),
                layout,
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
            ui_scale: mon.ui_scale,
            fonts,
            is_selected_monitor,
            status_scheme: status_scheme(&core.config().colors.status),
            status_separator_color: core.config().colors.status.separator,
            status_hover_color: core.config().colors.status.hover,
            startmenu_size: mon.startmenu_size,
            horizontal_padding: mon.horizontal_padding,
            gesture,
            layout_symbol: if core.model().is_overview_active_on(mon) {
                "OVR".to_string()
            } else {
                mon.layout_symbol_for_mask(selected_tags).to_string()
            },
            tags,
            show_shutdown: mon.selected.is_none(),
            titles,
            presentation,
            systray,
            external_right_width: if show_systray && is_selected_monitor {
                external_right_width.max(0)
            } else {
                0
            },
        });
    }

    snapshots
}

fn status_content(
    core: &mut CoreCtx,
    text: String,
    include_items: bool,
    click_events: bool,
) -> StatusContent {
    let items = if include_items {
        core.bar.status_items_for_text(&text).to_vec()
    } else {
        Vec::new()
    };
    StatusContent {
        text,
        items,
        click_events,
    }
}

fn draw_startmenu_icon(
    painter: &mut dyn BarPainter,
    scheme: &BarScheme,
    startmenu_size: i32,
    gesture: Gesture,
    bar_height: i32,
    ui_scale: f64,
) {
    let icon_size = scaled_px(STARTMENU_ICON_SIZE, ui_scale);
    let inner_size = scaled_px(STARTMENU_ICON_INNER, ui_scale);
    let icon_offset = (bar_height - icon_size) / 2;
    let outer_x = scaled_px(5, ui_scale);
    let inner_x = scaled_px(9, ui_scale);
    let inner_y_offset = scaled_px(4, ui_scale);
    let trailing_x = scaled_px(19, ui_scale);
    let startmenu_invert = gesture == Gesture::StartMenu;
    painter.set_scheme(scheme.clone());
    painter.rect(
        Rect::new(0, 0, startmenu_size, bar_height),
        !startmenu_invert,
    );
    painter.rect(
        Rect::new(outer_x, icon_offset, icon_size, icon_size),
        startmenu_invert,
    );
    painter.rect(
        Rect::new(
            inner_x,
            icon_offset + inner_y_offset,
            inner_size,
            inner_size,
        ),
        !startmenu_invert,
    );
    painter.rect(
        Rect::new(trailing_x, icon_offset + icon_size, inner_size, inner_size),
        startmenu_invert,
    );
}

fn draw_shutdown_button(
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
        TextOverflow::Ellipsis,
    );

    x + bar_height
}

fn draw_close_button(
    painter: &mut dyn BarPainter,
    scheme: &BarScheme,
    is_hover: bool,
    x: i32,
    bar_height: i32,
    ui_scale: f64,
) {
    let mut scheme = scheme.clone();
    scheme.foreground = scheme.detail;
    painter.set_scheme(scheme);
    let button_width = scaled_px(CLOSE_BUTTON_WIDTH, ui_scale);
    let button_height = scaled_px(CLOSE_BUTTON_HEIGHT, ui_scale);
    let button_detail = scaled_px(CLOSE_BUTTON_DETAIL, ui_scale);
    let button_x = x + bar_height / 6;
    let detail_offset = if is_hover { button_detail } else { 0 };
    let button_y = (bar_height - button_width) / 2 - detail_offset;
    painter.rect(
        Rect::new(button_x, button_y, button_width, button_height),
        true,
    );
    painter.rect(
        Rect::new(
            button_x,
            (bar_height - button_width) / 2 + button_height - detail_offset,
            button_width,
            button_detail + detail_offset,
        ),
        false,
    );
}

fn draw_tags_section(
    painter: &mut dyn BarPainter,
    snapshot: &MonitorBarSnapshot,
    mut x: i32,
    bar_height: i32,
    hit: &mut crate::bar::MonitorHitCache,
) -> i32 {
    for tag in &snapshot.tags {
        let text_w = painter.text_width(&tag.label);
        let width = (text_w + snapshot.horizontal_padding).max(snapshot.horizontal_padding);
        painter.set_scheme(tag.scheme.clone());
        let detail_height = if snapshot.gesture == Gesture::Tag(tag.slot) {
            scaled_px(TAG_DETAIL_BAR_HEIGHT_HOVER, snapshot.ui_scale)
        } else {
            scaled_px(TAG_DETAIL_BAR_HEIGHT_NORMAL, snapshot.ui_scale)
        };
        let lpad = (snapshot.horizontal_padding / 2).max(0);
        x = painter.text(
            Rect::new(x, 0, width, bar_height),
            lpad,
            &tag.label,
            false,
            detail_height,
            TextOverflow::Ellipsis,
        );
        hit.tag_ranges.push(crate::bar::TagHitRange {
            start: x - width,
            end: x,
            tag_index: tag.tag_index,
        });
    }
    x
}

fn draw_layout_symbol_section(
    painter: &mut dyn BarPainter,
    snapshot: &MonitorBarSnapshot,
    x: i32,
    bar_height: i32,
    hit: &mut crate::bar::MonitorHitCache,
) -> i32 {
    let text_w = painter.text_width(&snapshot.layout_symbol);
    let layout_w = (text_w + snapshot.horizontal_padding).max(snapshot.horizontal_padding);
    let lpad = ((layout_w - text_w) / 2).max(0);
    painter.set_scheme(snapshot.status_scheme.clone());
    let layout_start = x;
    let x = painter.text(
        Rect::new(x, 0, layout_w, bar_height),
        lpad,
        &snapshot.layout_symbol,
        false,
        0,
        TextOverflow::Ellipsis,
    );
    hit.layout_start = layout_start;
    hit.layout_end = x;
    x
}

fn draw_shutdown_section(
    painter: &mut dyn BarPainter,
    snapshot: &MonitorBarSnapshot,
    mut x: i32,
    bar_height: i32,
    hit: &mut crate::bar::MonitorHitCache,
) -> i32 {
    if snapshot.show_shutdown {
        x = draw_shutdown_button(painter, &snapshot.status_scheme, x, bar_height);
    }
    hit.shutdown_end = x;
    x
}

fn draw_status_section(
    painter: &mut dyn BarPainter,
    snapshot: &MonitorBarSnapshot,
    x: i32,
    systray_width: i32,
    bar_height: i32,
    hit: &mut crate::bar::MonitorHitCache,
) -> Rect {
    let status_output = if snapshot.is_selected_monitor {
        let Some(content) = snapshot
            .presentation
            .status
            .content()
            .filter(|content| !content.items.is_empty())
        else {
            return crate::bar::status::StatusRenderOutput::default().bounds;
        };
        let hover = if content.click_events {
            match snapshot.gesture {
                Gesture::StatusBlock(block_index) => Some(crate::bar::status::StatusBlockHover {
                    block_index,
                    color: snapshot.status_hover_color,
                }),
                _ => None,
            }
        } else {
            None
        };
        let status_right = snapshot.rect.w - systray_width;
        crate::bar::status::draw_status_items(
            Rect::new(x, 0, (status_right - x).max(0), bar_height),
            content.items.as_slice(),
            crate::bar::status::StatusRenderOptions {
                base_scheme: snapshot.status_scheme.clone(),
                separator_color: snapshot.status_separator_color,
                hover,
                edge_padding: snapshot.horizontal_padding / 2,
            },
            painter,
        )
    } else {
        crate::bar::status::StatusRenderOutput::default()
    };
    hit.status_click_targets = status_output.click_targets;
    status_output.bounds
}

fn draw_titles_section(
    painter: &mut dyn BarPainter,
    snapshot: &MonitorBarSnapshot,
    x: i32,
    title_width: i32,
    bar_height: i32,
    hit: &mut crate::bar::MonitorHitCache,
) {
    if snapshot.titles.is_empty() {
        painter.set_scheme(snapshot.status_scheme.clone());
        painter.rect(Rect::new(x, 0, title_width, bar_height), true);
        return;
    }

    let total_width = title_width + 1;
    let mut title_x = x;
    for (title, this_width) in snapshot
        .titles
        .iter()
        .zip(crate::bar::model::distribute_cells(
            total_width,
            snapshot.titles.len() as i32,
        ))
    {
        let text_w = painter.text_width(&title.name);
        painter.set_scheme(title.scheme.clone());
        let roomy_text_margin = scaled_px(64, snapshot.ui_scale);
        let close_button_threshold = scaled_px(32, snapshot.ui_scale);
        let close_button_text_offset = scaled_px(20, snapshot.ui_scale);
        let lpad = if text_w < this_width - roomy_text_margin {
            ((this_width - text_w) as f32 * 0.5) as i32
        } else {
            snapshot.horizontal_padding / 2
                + if this_width >= close_button_threshold {
                    close_button_text_offset
                } else {
                    0
                }
        };
        painter.text(
            Rect::new(title_x, 0, this_width, bar_height),
            lpad,
            &title.name,
            false,
            scaled_px(TAG_DETAIL_BAR_HEIGHT_NORMAL, snapshot.ui_scale),
            TextOverflow::Ellipsis,
        );
        if let Some(close_scheme) = &title.close_scheme
            && this_width >= close_button_threshold
        {
            draw_close_button(
                painter,
                close_scheme,
                snapshot.gesture == Gesture::CloseButton,
                title_x,
                bar_height,
                snapshot.ui_scale,
            );
        }
        hit.title_ranges.push(crate::bar::TitleHitRange {
            start: title_x,
            end: title_x + this_width,
            win: title.win,
        });
        title_x += this_width;
    }
}

fn record_systray_hits(snapshot: &MonitorBarSnapshot, hit: &mut crate::bar::MonitorHitCache) {
    if let Some(systray) = &snapshot.systray {
        let layout = &systray.layout;
        hit.systray_slots = layout
            .cells
            .iter()
            .map(|cell| crate::bar::SystrayHitSlot {
                idx: cell.idx,
                start: cell.hit_start,
                end: cell.hit_end,
            })
            .collect();
        if layout.menu.width > 0 {
            hit.overlay = Some(crate::bar::BarOverlayHit::TrayMenu {
                start: layout.menu.start_x,
                end: layout.menu.start_x + layout.menu.width,
                slots: layout.menu.cells.clone(),
            });
        }
    }
}

pub(crate) fn render_monitor_snapshot(
    snapshot: &MonitorBarSnapshot,
    painter: &mut dyn BarPainter,
) -> MonitorRenderOutput {
    let bar_height = snapshot.rect.h;
    let systray_width = if snapshot.is_selected_monitor {
        snapshot
            .systray
            .as_ref()
            .map(|s| s.layout.total_width)
            .unwrap_or(0)
            + snapshot.external_right_width
    } else {
        0
    };

    let mut hit = crate::bar::MonitorHitCache::default();

    draw_startmenu_icon(
        painter,
        &snapshot.status_scheme,
        snapshot.startmenu_size,
        snapshot.gesture,
        bar_height,
        snapshot.ui_scale,
    );

    let mut x = snapshot.startmenu_size;
    x = draw_tags_section(painter, snapshot, x, bar_height, &mut hit);
    x = draw_layout_symbol_section(painter, snapshot, x, bar_height, &mut hit);
    x = draw_shutdown_section(painter, snapshot, x, bar_height, &mut hit);

    let status_bounds =
        draw_status_section(painter, snapshot, x, systray_width, bar_height, &mut hit);
    let title_end_x = if status_bounds.w > 0 {
        status_bounds.x
    } else {
        snapshot.rect.w - systray_width
    };
    hit.status_hit_x = title_end_x;
    let title_width = (title_end_x - x).max(0);

    draw_titles_section(painter, snapshot, x, title_width, bar_height, &mut hit);
    record_systray_hits(snapshot, &mut hit);
    draw_systray_section(painter, snapshot);

    MonitorRenderOutput {
        hit_cache: hit,
        bar_clients_width: title_width,
    }
}

/// Paint compositor-rendered tray icons and any hosted tray menu.
///
/// Shared by every backend through [`BarPainter`] so X11 and Wayland bars
/// render StatusNotifier items identically.
fn draw_systray_section(painter: &mut dyn BarPainter, snapshot: &MonitorBarSnapshot) {
    let Some(systray) = &snapshot.systray else {
        return;
    };
    let layout = &systray.layout;
    let bar_height = snapshot.rect.h;

    painter.set_scheme(systray.base_scheme.clone());
    if layout.total_width > 0 {
        painter.rect(
            Rect::new(layout.start_x, 0, layout.total_width, bar_height),
            true,
        );
    }
    if layout.menu.width > 0 {
        painter.rect(
            Rect::new(layout.menu.start_x, 0, layout.menu.width, bar_height),
            true,
        );
    }

    for cell in &layout.cells {
        let Some(item) = systray.items.items.get(cell.idx) else {
            continue;
        };
        painter.blit_rgba(cell.icon, item.icon_size, &item.icon_rgba);
    }

    if let Some(menu) = snapshot.presentation.tray_menu() {
        let hover = match snapshot.gesture {
            Gesture::TrayMenuEntry(entry_index) => Some(crate::systray::render::TrayMenuHover {
                entry_index,
                color: snapshot.status_hover_color,
            }),
            _ => None,
        };
        crate::systray::render::draw_menu(
            painter,
            &menu.view,
            &layout.menu.cells,
            &systray.base_scheme,
            bar_height,
            hover,
            snapshot.ui_scale,
        );
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::model::WmModel;
    use crate::types::color::Rgba;
    use crate::types::{
        Client, CloseButtonColorConfigs, ColorSchemeRgba, Monitor, SchemeClose, SchemeHover,
        SchemeTag, SchemeWin, StatusColorConfig, TagColorConfigs, TagMask, WindowColorConfigs,
        WindowId,
    };

    fn marker(value: f32) -> ColorSchemeRgba {
        let color = Rgba::new(value, value, value, 1.0);
        ColorSchemeRgba::new(color, color, color)
    }

    #[test]
    fn decorative_pixels_follow_monitor_scale() {
        assert_eq!(scaled_px(4, 1.0), 4);
        assert_eq!(scaled_px(4, 1.5), 6);
        assert_eq!(scaled_px(1, 0.5), 1);
    }

    #[test]
    fn status_scheme_uses_status_colors() {
        let colors = StatusColorConfig {
            fg: Rgba::new(0.1, 0.1, 0.1, 1.0),
            bg: Rgba::new(0.2, 0.2, 0.2, 1.0),
            detail: Rgba::new(0.3, 0.3, 0.3, 1.0),
            separator: Rgba::new(0.4, 0.4, 0.4, 1.0),
            hover: Rgba::ZERO,
        };

        let scheme = status_scheme(&colors);

        assert_eq!(scheme.foreground, colors.fg);
        assert_eq!(scheme.background, colors.bg);
        assert_eq!(scheme.detail, colors.detail);
    }

    #[test]
    fn tag_hover_fill_scheme_uses_hover_filled_colors() {
        let mut colors = TagColorConfigs::default();
        colors.hover.filled = marker(0.4);

        let scheme = tag_hover_fill_scheme(&colors);

        assert_eq!(scheme.background, colors.hover.filled.bg);
    }

    #[test]
    fn tag_scheme_gives_urgent_tags_precedence() {
        let mut model = WmModel::new();
        model.tags.colors.no_hover.urgent = marker(0.5);
        let monitor = Monitor::default();

        let scheme = tag_scheme(
            &model,
            &monitor,
            0,
            TagMask::single(1).unwrap(),
            TagMask::single(1).unwrap(),
            false,
        );

        assert_eq!(
            scheme.background,
            model
                .tags
                .colors
                .colors_for(SchemeHover::NoHover, SchemeTag::Urgent)
                .bg
        );
    }

    #[test]
    fn window_scheme_uses_focused_sticky_role() {
        let mut model = WmModel::new();
        let monitor_id = model.monitors.push(Monitor::default());
        model.monitors.set_selected(monitor_id);
        let win = WindowId(42);
        model.insert_client(Client {
            win,
            monitor_id,
            is_sticky: true,
            ..Client::default()
        });
        model.monitor_mut(monitor_id).unwrap().selected = Some(win);
        let mut colors = WindowColorConfigs::default();
        colors.no_hover.sticky_focus = marker(0.6);

        let scheme = window_scheme(&model, &colors, model.client(win).unwrap(), false);

        assert_eq!(
            scheme.background,
            colors
                .colors_for(SchemeHover::NoHover, SchemeWin::StickyFocus)
                .bg
        );
    }

    #[test]
    fn close_button_scheme_gives_locked_state_precedence() {
        let mut colors = CloseButtonColorConfigs::default();
        colors.hover.locked = marker(0.7);

        let scheme = close_button_scheme(&colors, true, true, true);

        assert_eq!(
            scheme.background,
            colors
                .colors_for(SchemeHover::Hover, SchemeClose::Locked)
                .bg
        );
    }
}
