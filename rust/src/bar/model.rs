use crate::contexts::WmCtx;
use crate::globals::Globals;
use crate::systray::get_systray_width;
use crate::tags::{get_tag_at_x, get_tag_width};
use crate::types::*;
use x11rb::protocol::xproto::Window;

#[derive(Clone, Copy, Debug, Default)]
pub(crate) struct ClientBarStats {
    pub occupied_tags: u32,
    pub urgent_tags: u32,
    pub visible_clients: i32,
}

impl ClientBarStats {
    /// Collect bar statistics for the given monitor.
    ///
    /// * `visible_clients` — counted by walking the intrusive linked list so
    ///   the number exactly matches what `draw_window_titles` will draw and
    ///   what `bar_position_at_x` uses for hit-testing.  The draw/hit-test
    ///   code skips clients that fail `is_visible()`, so we apply the same
    ///   predicate here.
    ///
    /// * `occupied_tags` / `urgent_tags` — accumulated from all clients on the
    ///   monitor regardless of list order; order does not matter for bitmasks.
    pub(crate) fn collect(monitor: &Monitor, globals: &Globals) -> Self {
        let mut stats = Self::default();

        // ── Pass 1: visible_clients via the linked list ───────────────────
        // Walking the linked list (monitor.clients → client.next) gives the
        // same iteration order as draw_window_titles and bar_position_at_x,
        // so the count is guaranteed to be consistent with what is drawn and
        // what click regions are calculated for.
        let mut current = monitor.clients;
        while let Some(c_win) = current {
            let Some(client) = globals.clients.get(&c_win) else {
                break;
            };
            current = client.next;

            if client.is_visible() {
                stats.visible_clients += 1;
            }
        }

        // ── Pass 2: occupied / urgent tag bits from all clients on this monitor
        // Use the monitor's numeric id for matching so that clients on other
        // monitors (including ones not yet attached to any list) are excluded.
        for client in globals.clients.values() {
            if client.mon_id != Some(monitor.num as usize) {
                continue;
            }
            stats.occupied_tags |= if client.tags == 255 { 0 } else { client.tags };
            if client.isurgent {
                stats.urgent_tags |= client.tags;
            }
        }

        stats
    }
}

/// Describes precisely what the mouse cursor is positioned over in the bar.
///
/// This enum is the single source of truth for bar hit-testing. All three
/// consumers — click dispatch (`button_press` via `classify_bar_click`),
/// hover/gesture detection (`motion_notify`), and drag hover highlighting
/// (`update_bar_hover` / `handle_bar_drop` / `drag_tag`) — call
/// `bar_position_at_x` and map the result to a `Click`/`Gesture` as needed.
/// This guarantees that every code path uses identical geometry, eliminating
/// the previous duplication where each path reimplemented its own x-coordinate
/// logic and could silently disagree.
///
/// # Variants
///
/// | Variant | What the cursor is over |
/// |---------|------------------------|
/// | `StartMenu` | The start-menu icon at the left edge |
/// | `Tag(tag_index)` | A tag indicator button (0-based tag index) |
/// | `LayoutSymbol` | The layout symbol text next to the tags |
/// | `ShutDown` | The shutdown button (shown when no window is selected) |
/// | `WinTitle(win)` | The title area of a specific client window |
/// | `CloseButton(win)` | The close button of the selected client |
/// | `ResizeWidget(win)` | The resize widget at the right edge of a selected client |
/// | `StatusText` | The status text / command area on the right |
/// | `Root` | Nothing — the cursor is in an unoccupied part of the bar |
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum BarPosition {
    /// The start-menu icon.
    StartMenu,
    /// A tag button. The inner value is the **0-based** tag index.
    Tag(usize),
    /// The layout symbol indicator (e.g. `[]=`).
    LayoutSymbol,
    /// The shutdown/power button visible when no client is selected.
    ShutDown,
    /// The title cell of a specific client window.
    WinTitle(Window),
    /// The close button that overlays the left edge of the selected client's title.
    CloseButton(Window),
    /// The resize widget that overlays the right edge of the selected client's title.
    ResizeWidget(Window),
    /// The status-text / command strip on the right side of the bar.
    StatusText,
    /// An unoccupied area of the bar (no actionable widget).
    Root,
}

impl BarPosition {
    /// Convert to the `Click` enum used by the button-binding dispatch table.
    pub fn to_click(self) -> Click {
        match self {
            BarPosition::StartMenu => Click::StartMenu,
            BarPosition::Tag(_) => Click::TagBar,
            BarPosition::LayoutSymbol => Click::LtSymbol,
            BarPosition::ShutDown => Click::ShutDown,
            BarPosition::WinTitle(_) => Click::WinTitle,
            BarPosition::CloseButton(_) => Click::CloseButton,
            BarPosition::ResizeWidget(_) => Click::ResizeWidget,
            BarPosition::StatusText => Click::StatusText,
            BarPosition::Root => Click::RootWin,
        }
    }

    /// Convert to the `Gesture` used for hover highlighting.
    ///
    /// Not every `BarPosition` maps to a meaningful gesture; those that don't
    /// return `Gesture::None`.
    pub fn to_gesture(self) -> Gesture {
        match self {
            BarPosition::StartMenu => Gesture::StartMenu,
            BarPosition::Tag(idx) => Gesture::Tag(idx),
            BarPosition::CloseButton(_) => Gesture::CloseButton,
            // Title hover is tracked via `Gesture::None` (bar redraws handle
            // highlighting based on `selmon.sel`); other positions also use None.
            _ => Gesture::None,
        }
    }
}

/// Compute which logical bar region the cursor's **monitor-local** x coordinate
/// falls in for the given monitor.
///
/// `local_x` must already be relative to the left edge of the monitor
/// (i.e. `root_x − monitor.monitor_rect.x` for root-window coordinates, or
/// `event_x` for bar-window coordinates which are already monitor-local).
///
/// This function is the **single canonical hit-test** for the bar. Both click
/// handling and hover-gesture detection should call it rather than reimplementing
/// the geometry themselves.
pub fn bar_position_at_x(mon: &Monitor, ctx: &mut WmCtx, local_x: i32) -> BarPosition {
    use crate::bar::get_layout_symbol_width;

    let start_menu_size = ctx.g.cfg.startmenusize;
    let (tag_end, tag_idx) = (get_tag_width(ctx), get_tag_at_x(ctx, local_x));
    let blw = get_layout_symbol_width(mon);

    // ── Start menu ────────────────────────────────────────────────────────
    if local_x < start_menu_size {
        return BarPosition::StartMenu;
    }

    // ── Tag buttons ───────────────────────────────────────────────────────
    if tag_idx >= 0 {
        return BarPosition::Tag(tag_idx as usize);
    }

    // ── Layout symbol ─────────────────────────────────────────────────────
    if local_x < tag_end + blw {
        return BarPosition::LayoutSymbol;
    }

    // ── Shutdown button (only when no client is selected) ─────────────────
    let bh = ctx.g.cfg.bh;
    if mon.sel.is_none() && local_x < tag_end + blw + bh {
        return BarPosition::ShutDown;
    }

    // ── Status text ───────────────────────────────────────────────────────
    let status_hit_x = mon.work_rect.w - get_systray_width(ctx) as i32 - ctx.g.status_text_width
        + ctx.g.cfg.lrpad
        - 2;
    if local_x > status_hit_x {
        return BarPosition::StatusText;
    }

    // ── Window title cells ────────────────────────────────────────────────
    // Build the ordered list of visible clients exactly as draw_window_titles
    // does (intrusive linked list walk). draw_window_titles and all click/hover
    // consumers delegate to this function, so the order is always consistent.
    let mut visible_clients: Vec<Window> = Vec::new();
    let mut current = mon.clients;
    while let Some(c_win) = current {
        let Some(c) = ctx.g.clients.get(&c_win) else {
            break;
        };
        current = c.next;
        if c.is_visible() {
            visible_clients.push(c_win);
        }
    }

    if !visible_clients.is_empty() {
        // Total width of the title area.
        let title_area_start = tag_end + blw;
        let total_width = if mon.bar_clients_width > 0 {
            mon.bar_clients_width + 1
        } else {
            (mon.work_rect.w - title_area_start).max(0)
        };

        let n = visible_clients.len() as i32;
        let each_width = total_width / n;
        let mut remainder = total_width % n;

        let mut cell_start = title_area_start;
        for c_win in visible_clients {
            let this_width = if remainder > 0 {
                remainder -= 1;
                each_width + 1
            } else {
                each_width
            };
            let cell_end = cell_start + this_width;

            if local_x <= cell_end {
                // Cursor is inside this cell.
                let resize_start = cell_start + this_width - RESIZE_WIDGET_WIDTH;
                if mon.sel == Some(c_win) && local_x < cell_start + CLOSE_BUTTON_HIT_WIDTH {
                    return BarPosition::CloseButton(c_win);
                }
                if mon.sel == Some(c_win) && local_x > resize_start {
                    return BarPosition::ResizeWidget(c_win);
                }
                return BarPosition::WinTitle(c_win);
            }

            cell_start = cell_end;
        }
    }

    BarPosition::Root
}
