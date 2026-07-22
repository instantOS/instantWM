#![allow(clippy::too_many_arguments)]
use crate::backend::x11::X11BackendRef;
use crate::backend::x11::X11RuntimeConfig;
use crate::backend::x11::set_client_state;
use crate::contexts::CoreCtx;
use crate::types::*;
use x11rb::CURRENT_TIME;
use x11rb::connection::Connection;
use x11rb::protocol::xproto::ConnectionExt;
use x11rb::protocol::xproto::*;
use x11rb::wrapper::ConnectionExt as WrapperConnectionExt;

const XEMBED_MAPPED: u32 = 1 << 0;
const XEMBED_WINDOW_ACTIVATE: u32 = 1;
const XEMBED_WINDOW_DEACTIVATE: u32 = 2;
const XEMBED_EMBEDDED_VERSION: u32 = 0;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
struct XEmbedInfo {
    _version: u32,
    flags: u32,
}

impl XEmbedInfo {
    fn parse(values: impl IntoIterator<Item = u32>) -> Option<Self> {
        let mut values = values.into_iter();
        Some(Self {
            _version: values.next()?,
            flags: values.next()?,
        })
    }

    fn is_mapped(self) -> bool {
        self.flags & XEMBED_MAPPED != 0
    }
}

#[derive(Debug, Default, PartialEq, Eq)]
struct XEmbedLayout {
    width: u32,
    cells: Vec<XEmbedCell>,
}

#[derive(Debug, Default, PartialEq, Eq)]
struct XEmbedCell {
    offset: u32,
    width: u32,
}

fn layout_xembed_icons(
    icon_widths: impl IntoIterator<Item = i32>,
    bar_height: i32,
    configured_padding: i32,
) -> XEmbedLayout {
    let padding = crate::systray::visual_padding(bar_height, configured_padding);
    let mut cursor = 0u32;
    let mut cells = Vec::new();
    for icon_width in icon_widths.into_iter().filter(|width| *width > 0) {
        let width = crate::systray::cell_width(icon_width, bar_height, padding) as u32;
        cells.push(XEmbedCell {
            offset: cursor,
            width,
        });
        cursor = cursor.saturating_add(width);
    }
    XEmbedLayout {
        width: cursor,
        cells,
    }
}

pub fn get_systray_width(
    globals: &crate::core_state::CoreState,
    systray: Option<&XEmbedTray>,
) -> u32 {
    if !globals.config.systray.show {
        return 0;
    }

    layout_xembed_icons(
        systray.into_iter().flat_map(|tray| {
            tray.icons.iter().filter_map(|icon_win| {
                globals
                    .model
                    .client(*icon_win)
                    .filter(|client| !client.tags.is_empty())
                    .map(|client| client.geo.w)
            })
        }),
        globals.config.derived.bar_height,
        globals.config.systray.spacing,
    )
    .width
}

/// Remove systray icon using dependency injection.
pub fn remove_systray_icon(
    globals: &mut crate::core_state::CoreState,
    systray: Option<&mut XEmbedTray>,
    icon_win: WindowId,
) {
    if !globals.config.systray.show {
        return;
    }

    if let Some(systray) = systray {
        systray.icons.retain(|&w| w != icon_win);
    }

    globals.model.remove_client(icon_win);
}

/// Update systray icon geometry using dependency injection.
pub fn update_systray_icon_geom(
    state: &mut crate::core_state::CoreState,
    x11: &X11BackendRef,
    icon_window: WindowId,
    requested_size: Size,
) {
    let bar_height = state.config.derived.bar_height;

    let Some(position) = state
        .model
        .client(icon_window)
        .map(|client| client.geo.position())
    else {
        return;
    };

    let new_size = crate::systray::fit_icon_size(
        requested_size,
        bar_height,
        crate::systray::IconScale::FitHeight,
    );
    if !new_size.is_positive() {
        return;
    }

    let mut rect = Rect::from_position_and_size(position, new_size);

    let outcome = crate::client::geometry::apply_size_hints(
        &state.model,
        &state.config,
        icon_window,
        &mut rect,
        false,
    );
    if outcome.should_apply_client_hints {
        crate::backend::x11::geometry::apply_icccm_size_hints(
            &mut state.model,
            x11,
            icon_window,
            &mut rect,
        );
    }

    // Now update the client with the computed values
    if let Some(client) = state.model.client_mut(icon_window) {
        client.geo = rect;

        if client.geo.h > bar_height {
            if client.geo.w == client.geo.h {
                client.geo.w = bar_height;
            } else {
                client.geo.w =
                    (bar_height as f32 * (client.geo.w as f32 / client.geo.h as f32)) as i32;
            }
            client.geo.h = bar_height;
        }
    }
}

/// Update systray icon state using dependency injection.
pub fn update_systray_icon_state(
    core: &mut CoreCtx,
    x11: &X11BackendRef,
    x11_runtime: &X11RuntimeConfig,
    systray: Option<&XEmbedTray>,
    icon_win: WindowId,
    ev: Option<&PropertyNotifyEvent>,
) {
    if !core.config().systray.show {
        return;
    }

    let xembed_info_atom = x11_runtime.xatom.xembed_info;
    if let Some(ev) = ev
        && ev.atom != xembed_info_atom
    {
        return;
    }

    let x11_icon_win: Window = icon_win.into();

    let Some(xembed_info) = read_xembed_info(x11, icon_win, xembed_info_atom) else {
        return;
    };

    let (current_tags, _has_systray) = {
        if let Some(client) = core.model_mut().client_mut(icon_win) {
            (client.tags, systray.is_some())
        } else {
            return;
        }
    };

    if xembed_info.is_mapped() && current_tags.is_empty() {
        if let Some(client) = core.model_mut().client_mut(icon_win) {
            client.tags = crate::types::TagMask::single(1).unwrap_or(crate::types::TagMask::EMPTY);
        }

        let systray_win = systray.as_ref().map(|s| s.win).unwrap_or_default();
        let conn = x11.conn;
        let _ = conn.map_window(x11_icon_win);
        let _ = conn.configure_window(
            x11_icon_win,
            &ConfigureWindowAux::new().stack_mode(StackMode::ABOVE),
        );
        send_event(
            conn,
            icon_win,
            xembed_info_atom,
            xembed_info_atom,
            CURRENT_TIME as i64,
            XEMBED_WINDOW_ACTIVATE as i64,
            0,
            u32::from(systray_win) as i64,
            XEMBED_EMBEDDED_VERSION as i64,
        );
        set_client_state(x11, x11_runtime, icon_win, 1);
    } else if !xembed_info.is_mapped() && !current_tags.is_empty() {
        if let Some(client) = core.model_mut().client_mut(icon_win) {
            client.tags = crate::types::TagMask::EMPTY;
        }

        let systray_win = systray.as_ref().map(|s| s.win).unwrap_or_default();
        let conn = x11.conn;
        let _ = conn.unmap_window(x11_icon_win);
        send_event(
            conn,
            icon_win,
            xembed_info_atom,
            xembed_info_atom,
            CURRENT_TIME as i64,
            XEMBED_WINDOW_DEACTIVATE as i64,
            0,
            u32::from(systray_win) as i64,
            XEMBED_EMBEDDED_VERSION as i64,
        );
        set_client_state(x11, x11_runtime, icon_win, 0);
    }
}

/// Update systray using dependency injection.
pub fn update_systray(
    core: &mut CoreCtx,
    x11: &X11BackendRef,
    x11_runtime: &X11RuntimeConfig,
    systray: &mut Option<XEmbedTray>,
) {
    if !core.config().systray.show {
        return;
    }

    if x11_runtime.xlibdisplay.0.is_null() {
        return;
    }

    // Flush Xlib display to ensure all Xlib requests are sent before using x11rb
    unsafe {
        crate::backend::x11::draw::XFlush(x11_runtime.xlibdisplay.0);
    }

    let (tray_right, bar_x, full_bar_width, bar_y, bar_win) = {
        let m = systray_to_mon(core.model(), &core.config().systray, None);
        let mon = match core.model().monitor(m) {
            Some(mon) => mon,
            None => return,
        };
        (
            mon.monitor_rect.x + mon.monitor_rect.w,
            mon.work_rect().x,
            mon.work_rect().w,
            mon.bar_y(),
            mon.bar_win,
        )
    };

    const MIN_MANAGER_WINDOW_WIDTH: u32 = 1;

    if systray.is_none() {
        let root = x11_runtime.root;
        let bar_height = core.config().derived.bar_height;
        let net_system_tray = x11_runtime.netatom.system_tray;
        let net_system_tray_horz = x11_runtime.netatom.system_tray_orientation_horz;
        let manager_atom = x11_runtime.xatom.manager;
        let bg_pixel = x11_runtime.status_scheme.bg.color.pixel as u32;

        let systray_win = Some(x11.conn).and_then(|conn| {
            let systray_win = conn.generate_id().ok()?;

            let result = conn.create_window(
                x11rb::COPY_FROM_PARENT as u8,
                systray_win,
                root,
                tray_right as i16,
                bar_y as i16,
                MIN_MANAGER_WINDOW_WIDTH as u16,
                bar_height as u16,
                0,
                WindowClass::INPUT_OUTPUT,
                x11rb::COPY_FROM_PARENT,
                &CreateWindowAux::new()
                    .event_mask(EventMask::BUTTON_PRESS | EventMask::EXPOSURE)
                    .override_redirect(1)
                    .background_pixel(bg_pixel),
            );

            if result.is_err() {
                return None;
            }

            let _ = result.and_then(|cookie| {
                cookie
                    .check()
                    .map_err(|_| x11rb::errors::ConnectionError::UnknownError)
            });

            let _ = conn.change_property32(
                PropMode::REPLACE,
                systray_win,
                net_system_tray,
                AtomEnum::CARDINAL,
                &[net_system_tray_horz],
            );

            let _ = conn.change_window_attributes(
                systray_win,
                &ChangeWindowAttributesAux::new().event_mask(EventMask::SUBSTRUCTURE_NOTIFY),
            );

            let _ = conn.map_window(systray_win);

            let _ = conn.change_window_attributes(
                systray_win,
                &ChangeWindowAttributesAux::new().background_pixel(bg_pixel),
            );

            let _ = conn.set_selection_owner(systray_win, net_system_tray, CURRENT_TIME);

            // Send MANAGER event to root window to announce systray
            // Use non-blocking approach
            let event = ClientMessageEvent {
                response_type: CLIENT_MESSAGE_EVENT,
                format: 32,
                sequence: 0,
                window: root,
                type_: manager_atom,
                data: ClientMessageData::from([CURRENT_TIME, net_system_tray, systray_win, 0, 0]),
            };
            let _ = conn.send_event(false, root, EventMask::STRUCTURE_NOTIFY, event);

            Some(systray_win)
        });

        let Some(systray_win) = systray_win else {
            return;
        };

        *systray = Some(XEmbedTray {
            win: WindowId::from(systray_win),
            icons: Vec::new(),
        });
    }

    let tray = systray
        .as_ref()
        .expect("tray manager creation must initialize owned XEmbed state");
    let (systray_win, icons) = (tray.win, tray.icons.clone());

    let bar_height = core.config().derived.bar_height;
    let bg_pixel = x11_runtime.status_scheme.bg.color.pixel as u32;

    let icon_layout: Vec<(WindowId, Size)> = icons
        .iter()
        .filter_map(|icon_win| {
            core.state()
                .model
                .clients
                .get(icon_win)
                .filter(|client| !client.tags.is_empty() && client.geo.w > 0 && client.geo.h > 0)
                .map(|client| (*icon_win, client.geo.size()))
        })
        .collect();

    let layout = layout_xembed_icons(
        icon_layout.iter().map(|(_, icon_size)| icon_size.w),
        bar_height,
        core.config().systray.spacing,
    );

    {
        let conn = x11.conn;
        for ((icon_win, _icon_size), cell) in icon_layout.into_iter().zip(&layout.cells) {
            let x11_icon_win: Window = icon_win.into();
            let _ = conn.change_window_attributes(
                x11_icon_win,
                &ChangeWindowAttributesAux::new().background_pixel(bg_pixel),
            );
            let _ = conn.map_window(x11_icon_win);

            let _ = conn.configure_window(
                x11_icon_win,
                &ConfigureWindowAux::new()
                    .x(cell.offset as i32)
                    .y(0)
                    .width(cell.width)
                    .height(bar_height as u32),
            );
        }
    }

    let x11_systray_win: Window = systray_win.into();
    let x11_bar_win: Window = bar_win.into();

    let reserved_width = layout.width;
    let manager_width = reserved_width.max(MIN_MANAGER_WINDOW_WIDTH);
    let tray_x = tray_right - manager_width as i32;
    let bar_width = full_bar_width.saturating_sub(reserved_width as i32).max(1) as u32;

    core.bar.runtime.systray_width = reserved_width as i32;
    core.bar.mark_dirty();

    let conn = x11.conn;

    let _ = conn.configure_window(
        x11_systray_win,
        &ConfigureWindowAux::new()
            .x(tray_x)
            .y(bar_y)
            .width(manager_width)
            .height(bar_height as u32),
    );

    let _ = conn.configure_window(
        x11_systray_win,
        &ConfigureWindowAux::new()
            .stack_mode(StackMode::ABOVE)
            .sibling(x11_bar_win),
    );

    let _ = conn.configure_window(
        x11_bar_win,
        &ConfigureWindowAux::new()
            .x(bar_x)
            .y(bar_y)
            .width(bar_width)
            .height(bar_height as u32),
    );

    let _ = conn.map_window(x11_systray_win);

    let _ = conn.flush();
}

/// Convert window to systray icon using dependency injection.
pub fn win_to_systray_icon(
    systray_show: bool,
    systray: Option<&XEmbedTray>,
    win: WindowId,
) -> Option<WindowId> {
    if !systray_show {
        return None;
    }

    if let Some(systray) = systray {
        for &icon_win in &systray.icons {
            if icon_win == win {
                return Some(win);
            }
        }
    }
    None
}

/// Get monitor for systray using dependency injection.
pub fn systray_to_mon(
    model: &crate::model::WmModel,
    config: &crate::core_state::SystrayConfig,
    m: Option<MonitorId>,
) -> MonitorId {
    if config.pinning == 0 {
        return match m {
            Some(id) => {
                if id == model.selected_monitor_id() {
                    id
                } else {
                    model.selected_monitor_id()
                }
            }
            None => model.selected_monitor_id(),
        };
    }

    let n = model.monitors.len();
    let target = config.pinning.min(n);

    if config.pinning > n {
        model
            .monitors
            .first()
            .unwrap_or(model.selected_monitor_id())
    } else {
        model
            .monitors
            .id_at_position(target.saturating_sub(1))
            .unwrap_or(model.selected_monitor_id())
    }
}

/// Get atom property using dependency injection.
fn read_xembed_info(x11: &X11BackendRef, win: WindowId, atom: u32) -> Option<XEmbedInfo> {
    let conn = x11.conn;
    let x11_win: Window = win.into();
    let reply = conn
        .get_property(false, x11_win, atom, AtomEnum::ANY, 0, 2)
        .ok()?
        .reply()
        .ok()?;
    XEmbedInfo::parse(reply.value32()?)
}

/// Send X event using dependency injection.
fn send_event(
    conn: &impl Connection,
    win: WindowId,
    proto: u32,
    mask: u32,
    d0: i64,
    d1: i64,
    d2: i64,
    d3: i64,
    d4: i64,
) {
    let x11_win: Window = win.into();
    let event = ClientMessageEvent {
        response_type: CLIENT_MESSAGE_EVENT,
        format: 32,
        sequence: 0,
        window: x11_win,
        type_: proto,
        data: ClientMessageData::from([d0 as u32, d1 as u32, d2 as u32, d3 as u32, d4 as u32]),
    };
    let _ = conn.send_event(false, x11_win, EventMask::from(mask), event);
}

#[cfg(test)]
mod tests {
    use super::{XEmbedCell, XEmbedInfo, XEmbedLayout, layout_xembed_icons};

    #[test]
    fn xembed_layout_uses_contiguous_full_hitbox_cells() {
        assert_eq!(
            layout_xembed_icons([16, 24], 30, 2),
            XEmbedLayout {
                width: 60,
                cells: vec![
                    XEmbedCell {
                        offset: 0,
                        width: 30
                    },
                    XEmbedCell {
                        offset: 30,
                        width: 30
                    },
                ],
            }
        );
    }

    #[test]
    fn xembed_layout_rejects_invalid_widths_and_negative_spacing() {
        assert_eq!(
            layout_xembed_icons([0, -4, 16], 30, -10),
            XEmbedLayout {
                width: 30,
                cells: vec![XEmbedCell {
                    offset: 0,
                    width: 30
                }],
            }
        );
        assert_eq!(layout_xembed_icons([], 30, 2).width, 0);
    }

    #[test]
    fn xembed_info_reads_flags_after_the_version_word() {
        let mapped = XEmbedInfo::parse([0, 1]).unwrap();
        assert!(mapped.is_mapped());

        let unmapped = XEmbedInfo::parse([1, 0]).unwrap();
        assert!(!unmapped.is_mapped());
        assert!(XEmbedInfo::parse([0]).is_none());
    }
}
