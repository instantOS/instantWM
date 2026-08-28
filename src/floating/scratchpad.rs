use crate::constants::animation::EMPHASIZED_ANIMATION_MILLIS;
use crate::contexts::WmCtx;
use crate::geometry::MoveResizeOptions;
use crate::ipc_types::ScratchpadInitialStatus;
use crate::layouts::arrange;
use crate::model::WmModel;
use crate::types::input::EdgeDirection;
use crate::types::{MonitorId, Rect, SCRATCHPAD_NAME_LEN, Size, TagMask, WindowId};
use bincode::{Decode, Encode};

const EDGE_MARGIN_X: i32 = 20;
const EDGE_MARGIN_Y: i32 = 40;
const DEFAULT_WIDTH_PERCENT: u32 = 50;
const DEFAULT_HEIGHT_PERCENT: u32 = 60;

pub const DEFAULT_EDGE_SCRATCHPAD_NAME: &str = "instantwm_edge_scratchpad";
pub(crate) const SCRATCHPAD_IDENTITY_PREFIX: &str = "scratchpad_";

/// Infer the scratchpad role advertised by launchers such as `ins` before the
/// client participates in its first layout. Native Wayland uses `class` as the
/// app-id; X11 terminals may put the same identity in either WM_CLASS field.
pub(crate) fn name_from_window_identity<'a>(class: &'a str, instance: &'a str) -> Option<&'a str> {
    [class, instance].into_iter().find_map(|identity| {
        identity
            .strip_prefix(SCRATCHPAD_IDENTITY_PREFIX)
            .filter(|name| !name.is_empty())
    })
}

/// Resolve a centered regular-scratchpad rectangle inside usable monitor space.
///
/// Percentages describe the complete decorated window, matching compositor
/// integrations such as Sway's `resize set ... ppt`. The returned client
/// geometry accounts for instantWM's compositor-side border.
fn regular_scratchpad_rect(
    content: Rect,
    border_width: i32,
    width_percent: u32,
    height_percent: u32,
) -> Result<Rect, String> {
    if !(1..=100).contains(&width_percent) || !(1..=100).contains(&height_percent) {
        return Err("scratchpad width and height must be between 1 and 100 percent".to_string());
    }
    if !content.is_valid() {
        return Err("scratchpad monitor has no usable content area".to_string());
    }

    let border = border_width.max(0);
    let total_width = ((i64::from(content.w) * i64::from(width_percent)) / 100)
        .max(i64::from(2 * border + 1))
        .min(i64::from(content.w)) as i32;
    let total_height = ((i64::from(content.h) * i64::from(height_percent)) / 100)
        .max(i64::from(2 * border + 1))
        .min(i64::from(content.h)) as i32;
    let width = (total_width - 2 * border).max(1);
    let height = (total_height - 2 * border).max(1);

    Ok(Rect::new(
        content.x + (content.w - total_width) / 2,
        content.y + (content.h - total_height) / 2,
        width,
        height,
    ))
}

pub(crate) fn default_regular_scratchpad_rect(
    content: Rect,
    border_width: i32,
) -> Result<Rect, String> {
    regular_scratchpad_rect(
        content,
        border_width,
        DEFAULT_WIDTH_PERCENT,
        DEFAULT_HEIGHT_PERCENT,
    )
}

/// Complete geometry for one edge-scratchpad transition.
///
/// `shown` is always contained by the monitor's visible content rectangle;
/// `hidden` is immediately outside the same edge. Keeping the pair together
/// prevents show and hide paths from drifting into different policies.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
struct EdgeSlideRects {
    hidden: Rect,
    shown: Rect,
}

impl EdgeSlideRects {
    fn new(content: Rect, direction: EdgeDirection, requested_size: Size) -> Self {
        let content = Rect::new(content.x, content.y, content.w.max(1), content.h.max(1));
        let horizontal_margin = EDGE_MARGIN_X.min((content.w - 1) / 2);
        let vertical_margin = EDGE_MARGIN_Y.min((content.h - 1) / 2);
        let horizontal_span = (content.w - 2 * horizontal_margin).max(1);
        let vertical_span = (content.h - 2 * vertical_margin).max(1);
        let requested_width = requested_size.w.max(1).min(content.w);
        let requested_height = requested_size.h.max(1).min(content.h);

        let shown = match direction {
            EdgeDirection::Top => Rect::new(
                content.x + horizontal_margin,
                content.y,
                horizontal_span,
                requested_height,
            ),
            EdgeDirection::Right => Rect::new(
                content.right() - requested_width,
                content.y + vertical_margin,
                requested_width,
                vertical_span,
            ),
            EdgeDirection::Bottom => Rect::new(
                content.x + horizontal_margin,
                content.bottom() - requested_height,
                horizontal_span,
                requested_height,
            ),
            EdgeDirection::Left => Rect::new(
                content.x,
                content.y + vertical_margin,
                requested_width,
                vertical_span,
            ),
        };

        let hidden = match direction {
            EdgeDirection::Top => Rect::new(shown.x, content.y - shown.h, shown.w, shown.h),
            EdgeDirection::Right => Rect::new(content.right(), shown.y, shown.w, shown.h),
            EdgeDirection::Bottom => Rect::new(shown.x, content.bottom(), shown.w, shown.h),
            EdgeDirection::Left => Rect::new(content.x - shown.w, shown.y, shown.w, shown.h),
        };

        Self { hidden, shown }
    }
}

#[derive(Debug, Clone, Decode, Encode, serde::Serialize, serde::Deserialize)]
pub struct ScratchpadInfo {
    pub name: String,
    pub visible: bool,
    pub window_id: Option<u32>,
    pub monitor: Option<usize>,
    pub x: Option<i32>,
    pub y: Option<i32>,
    pub width: Option<i32>,
    pub height: Option<i32>,
    pub mode: crate::types::ClientMode,
    pub direction: Option<String>,
}

impl ScratchpadInfo {
    pub(crate) fn from_client(
        c: &crate::types::client::Client,
        monitor_position: usize,
    ) -> Option<Self> {
        if !c.is_scratchpad() {
            return None;
        }
        let sp = c.scratchpad()?;
        Some(Self {
            name: sp.name().to_string(),
            visible: c.is_scratchpad_visible(),
            window_id: Some(c.win.0),
            monitor: Some(monitor_position),
            x: Some(c.geo.x),
            y: Some(c.geo.y),
            width: Some(c.geo.w),
            height: Some(c.geo.h),
            mode: c.mode(),
            direction: sp.direction().map(|d| d.as_str().to_string()),
        })
    }
}

fn selected_or_explicit_window(model: &WmModel, window_id: Option<WindowId>) -> Option<WindowId> {
    window_id.or_else(|| model.selected_win())
}

fn attach_client_to_monitor_top(model: &mut WmModel, win: WindowId, monitor_id: MonitorId) {
    let reassigned = model.reassign_client_monitor(win, monitor_id);
    debug_assert!(reassigned, "scratchpad target must be a managed monitor");
}

fn sync_scratchpad_backend_projection(ctx: &mut WmCtx<'_>, win: WindowId) {
    ctx.sync_client_tag_props(win);
}

fn prepare_scratchpad_for_show(model: &mut WmModel, win: WindowId, monitor_id: MonitorId) {
    attach_client_to_monitor_top(model, win, monitor_id);
}

fn reveal_scratchpad_window(ctx: &mut WmCtx<'_>, win: WindowId) -> bool {
    let was_hidden = ctx
        .core()
        .state()
        .model
        .client(win)
        .map(|c| c.is_hidden)
        .unwrap_or(false);

    if was_hidden {
        crate::client::show_window(ctx, win);
    }

    ctx.window_backend().map_window(win);
    ctx.window_backend().flush();

    was_hidden
}

fn arrange_visible_scratchpad(ctx: &mut WmCtx<'_>, win: WindowId, was_hidden: bool) {
    if was_hidden {
        return;
    }

    let Some(mid) = ctx.core().model().client(win).map(|c| c.monitor_id) else {
        return;
    };
    arrange(ctx, Some(mid));
    crate::layouts::sync_monitor_z_order(ctx, mid);
}

fn scratchpad_names(model: &WmModel, visible: bool) -> Vec<String> {
    model
        .clients
        .values()
        .filter(|c| c.is_scratchpad() && c.is_scratchpad_visible() == visible)
        .filter_map(|c| c.scratchpad().map(|sp| sp.name().to_string()))
        .collect()
}

/// Resolve a scratchpad only while its backend window still exists.
///
/// Destruction events are the primary cleanup path. This check is deliberately
/// shared by every scratchpad action so a missed backend event cannot leave a
/// model-only scratchpad that can be toggled forever.
fn find_live_scratchpad(ctx: &mut WmCtx<'_>, name: &str) -> Result<WindowId, String> {
    let Some(win) = ctx.core().model().scratchpad_find(name) else {
        return Err(format!("scratchpad '{}' not found", name));
    };
    if ctx.window_backend().window_exists(win) {
        return Ok(win);
    }

    crate::client::lifecycle::remove_managed_client(ctx, win);
    Err(format!("scratchpad '{}' no longer exists", name))
}

pub fn scratchpad_create(
    ctx: &mut WmCtx,
    name: &str,
    window_id: Option<WindowId>,
    direction: Option<EdgeDirection>,
    status: ScratchpadInitialStatus,
) -> Result<String, String> {
    if name.is_empty() {
        return Err("scratchpad name cannot be empty".to_string());
    }
    if name.chars().count() > SCRATCHPAD_NAME_LEN {
        return Err(format!(
            "scratchpad name cannot exceed {} characters",
            SCRATCHPAD_NAME_LEN
        ));
    }

    let target = selected_or_explicit_window(ctx.core().model(), window_id);
    let selected_window = target.ok_or_else(|| "no window selected".to_string())?;

    if find_live_scratchpad(ctx, name).is_ok() {
        return Err(format!("scratchpad '{}' already exists", name));
    }

    let (mode, already_scratchpad) = ctx
        .core()
        .model()
        .client(selected_window)
        .map(|client| (client.mode(), client.is_scratchpad()))
        .ok_or_else(|| format!("window {} is not managed", selected_window.0))?;
    if already_scratchpad {
        return Err(format!(
            "window {} is already a scratchpad",
            selected_window.0
        ));
    }
    if mode.is_fullscreen() {
        crate::client::fullscreen::set_fullscreen(ctx, selected_window, false);
    } else if mode.is_maximized() {
        crate::client::fullscreen::leave_maximized(ctx, selected_window);
    }

    // Resolve geometry from the target window's monitor, not whichever output
    // happens to be selected by the invoking backend or IPC client.
    let (mon_ww, mon_wh, regular_rect) = {
        let monitor_id = ctx
            .core()
            .model()
            .client(selected_window)
            .map(|client| client.monitor_id)
            .ok_or_else(|| format!("window {} is not managed", selected_window.0))?;
        let mon = ctx
            .core()
            .model()
            .monitor(monitor_id)
            .ok_or_else(|| format!("window {} has no managed monitor", selected_window.0))?;
        let client = ctx
            .core()
            .model()
            .client(selected_window)
            .expect("validated target client must remain managed");
        let content = mon.visible_content_rect(&ctx.core().model().clients);
        let regular_rect = direction
            .is_none()
            .then(|| default_regular_scratchpad_rect(content, client.border_width))
            .transpose()?;
        (mon.work_rect().w, mon.work_rect().h, regular_rect)
    };

    let client = ctx
        .core_mut()
        .state_mut()
        .model
        .client_mut(selected_window)
        .ok_or_else(|| format!("window {} is not managed", selected_window.0))?;
    client.promote_to_scratchpad(name, direction, mon_ww, mon_wh)?;
    if let Some(rect) = regular_rect {
        client.geo = rect;
        client.set_preferred_floating_size(rect.size());
    }

    sync_scratchpad_backend_projection(ctx, selected_window);

    crate::client::hide(ctx, selected_window);

    if matches!(status, ScratchpadInitialStatus::Shown) {
        scratchpad_show_name(ctx, name)?;
    }

    Ok(format!("created scratchpad '{}'", name))
}

/// Restore a scratchpad to an ordinary window.
///
/// With no explicit destination, the complete pre-scratchpad state is
/// restored. Tag assignment uses the same transition with an explicit monitor
/// and tag mask, making "send scratchpad to tag" an intentional conversion.
pub(crate) fn scratchpad_restore_window(
    ctx: &mut WmCtx,
    window: WindowId,
    destination: Option<(MonitorId, TagMask)>,
) -> Result<String, String> {
    let explicit_destination = destination.is_some();
    let tag_mask = ctx.core().model().tags.mask();
    let current_monitor = ctx.core().model().selected_monitor_id();
    let (name, source_monitor, original_monitor, original_tags, was_hidden) = {
        let client = ctx
            .core()
            .model()
            .client(window)
            .ok_or_else(|| format!("window {} is not managed", window.0))?;
        let scratchpad = client
            .scratchpad()
            .ok_or_else(|| format!("window {} is not a scratchpad", window.0))?;
        (
            scratchpad.name().to_string(),
            client.monitor_id,
            scratchpad.original_monitor(),
            scratchpad.original_tags(),
            client.is_hidden,
        )
    };

    let requested_monitor = destination
        .map(|(monitor, _)| monitor)
        .unwrap_or(original_monitor);
    let target_monitor = if ctx.core().model().monitor(requested_monitor).is_some() {
        requested_monitor
    } else if ctx.core().model().monitor(current_monitor).is_some() {
        current_monitor
    } else {
        source_monitor
    };
    let requested_tags = destination.map(|(_, tags)| tags).unwrap_or(original_tags) & tag_mask;
    let target_tags = if requested_tags.is_empty() {
        ctx.core()
            .model()
            .monitor(target_monitor)
            .map(|monitor| monitor.selected_tags())
            .unwrap_or(TagMask::EMPTY)
    } else {
        requested_tags
    };
    if target_tags.is_empty() {
        return Err("cannot restore scratchpad without a normal target tag".to_string());
    }

    let client = ctx
        .core_mut()
        .model_mut()
        .client_mut(window)
        .expect("validated managed scratchpad must remain present during restoration");
    client.restore_from_scratchpad(Some(target_tags))?;
    if explicit_destination {
        client.is_sticky = false;
    }
    let previous_focus = ctx.core().model().selected_win();
    let reassigned = ctx
        .core_mut()
        .mutate_selection(|model| model.reassign_client_monitor(window, target_monitor));
    debug_assert!(
        reassigned,
        "validated restore monitor must accept the client"
    );

    if was_hidden {
        crate::client::show_window(ctx, window);
        crate::focus::refresh_focus_after_selection(ctx, previous_focus, Some(window));
    } else if ctx.core().model().selected_win() != previous_focus {
        crate::focus::refresh_focus_after_selection(ctx, previous_focus, None);
    }
    arrange(ctx, Some(source_monitor));
    if target_monitor != source_monitor {
        arrange(ctx, Some(target_monitor));
    }

    sync_scratchpad_backend_projection(ctx, window);

    Ok(format!("restored scratchpad '{}' as a normal window", name))
}

pub fn scratchpad_restore(
    ctx: &mut WmCtx,
    name: Option<&str>,
    window_id: Option<WindowId>,
) -> Result<String, String> {
    let window = if let Some(name) = name {
        find_live_scratchpad(ctx, name)?
    } else {
        selected_or_explicit_window(ctx.core().model(), window_id)
            .ok_or_else(|| "no scratchpad name, window ID, or selected window".to_string())?
    };
    scratchpad_restore_window(ctx, window, None)
}

#[derive(Debug, Clone, Copy)]
pub(crate) struct ScratchpadShowOptions {
    pub monitor_id: MonitorId,
    pub focus: bool,
    pub warp_pointer: bool,
}

pub fn scratchpad_show_name(ctx: &mut WmCtx, name: &str) -> Result<String, String> {
    let options = ScratchpadShowOptions {
        monitor_id: ctx.core().model().selected_monitor_id(),
        focus: true,
        warp_pointer: ctx.core().behavior().focus_follows_mouse.is_enabled(),
    };
    scratchpad_show_name_with_options(ctx, name, options)
}

/// Resize and center a regular scratchpad as a percentage of usable monitor space.
pub fn scratchpad_resize_name(
    ctx: &mut WmCtx,
    name: &str,
    width_percent: u32,
    height_percent: u32,
) -> Result<String, String> {
    let win = find_live_scratchpad(ctx, name)?;
    let rect = {
        let view = ctx
            .core()
            .model()
            .client_view(win)
            .ok_or_else(|| format!("scratchpad '{}' has no assigned monitor", name))?;
        if view.client.is_edge_scratchpad() {
            return Err(format!(
                "scratchpad '{}' is edge-anchored and cannot use percentage sizing",
                name
            ));
        }
        regular_scratchpad_rect(
            view.monitor
                .visible_content_rect(&ctx.core().model().clients),
            view.client.border_width,
            width_percent,
            height_percent,
        )?
    };

    ctx.move_resize(win, rect, MoveResizeOptions::immediate());
    Ok(format!(
        "resized scratchpad '{}' to {}% x {}%",
        name, width_percent, height_percent
    ))
}

pub(crate) fn scratchpad_show_name_with_options(
    ctx: &mut WmCtx,
    name: &str,
    options: ScratchpadShowOptions,
) -> Result<String, String> {
    let found = find_live_scratchpad(ctx, name)?;
    // The window-level helper cannot name the scratchpad (the transfer path
    // has no name to give); re-attach it here for user-facing errors.
    let shown = show_scratchpad_window_with_options(ctx, found, options)
        .map_err(|error| format!("scratchpad '{}': {}", name, error))?;
    if shown {
        Ok(format!("shown scratchpad '{}'", name))
    } else {
        Ok(format!("scratchpad '{}' is already visible", name))
    }
}

fn show_scratchpad_window_with_options(
    ctx: &mut WmCtx,
    found: WindowId,
    options: ScratchpadShowOptions,
) -> Result<bool, String> {
    if ctx.core().model().monitor(options.monitor_id).is_none() {
        return Err(format!(
            "target monitor {:?} does not exist",
            options.monitor_id
        ));
    }

    let Some((was_visible, direction)) = ctx.core().model().client(found).and_then(|client| {
        client
            .scratchpad()
            .map(|scratchpad| (client.is_scratchpad_visible(), scratchpad.direction()))
    }) else {
        return Err(format!("window {} is not a managed scratchpad", found.0));
    };
    let mut reversing_slide_out = false;

    if was_visible {
        // A pending hide means the slide-out is still animating; reversing the
        // toggle must cancel that pending hide so the deferred concealment
        // never runs, then fall through so the entrance animation retargets
        // the window from wherever the slide currently is.
        reversing_slide_out = ctx.core().pending_work().has_pending_scratchpad_hide(found);
        ctx.core_mut()
            .pending_work_mut()
            .cancel_pending_scratchpad_hide(found);
        if !reversing_slide_out {
            return Ok(false);
        }
    }

    let target_monitor = options.monitor_id;
    let restore_focus = ctx
        .core()
        .model()
        .monitor(target_monitor)
        .and_then(|monitor| monitor.selected)
        .filter(|&selected| selected != found);
    let previous_focus = ctx.core().model().selected_win();
    ctx.core_mut()
        .mutate_selection(|model| prepare_scratchpad_for_show(model, found, target_monitor));
    // A reversed slide-out keeps the focus target remembered when it was
    // first shown; overwriting it here would degrade the eventual hand-off.
    if !reversing_slide_out
        && let Some(client) = ctx.core_mut().model_mut().client_mut(found)
        && let Some(scratchpad) = client.scratchpad_mut()
    {
        scratchpad.remember_focus(restore_focus);
    }

    if let Some(dir) = direction {
        let (content_rect, client_size) = {
            let mon = ctx
                .core()
                .model()
                .monitor(target_monitor)
                .expect("validated target monitor must exist while showing scratchpad");
            let client = ctx
                .core()
                .model()
                .client(found)
                .expect("scratchpad client must exist after window_exists check");
            (
                mon.visible_content_rect(&ctx.core().model().clients),
                client.geo.size(),
            )
        };

        let slide = EdgeSlideRects::new(content_rect, dir, client_size);

        // A reversed slide-out is still mapped mid-flight; re-parking it
        // off-screen first would make the window jump. The animate_to below
        // continues the existing transition from its current frame instead.
        if !reversing_slide_out {
            ctx.move_resize(found, slide.hidden, MoveResizeOptions::immediate());
            reveal_scratchpad_window(ctx, found);
        }
        ctx.move_resize(
            found,
            slide.shown,
            MoveResizeOptions::animate_to(EMPHASIZED_ANIMATION_MILLIS),
        );
    } else {
        let was_hidden = reveal_scratchpad_window(ctx, found);
        arrange_visible_scratchpad(ctx, found, was_hidden);
    }

    if options.focus {
        crate::focus::refresh_focus_after_selection(ctx, previous_focus, Some(found));
    } else if ctx.core().model().selected_win() != previous_focus {
        crate::focus::refresh_focus_after_selection(ctx, previous_focus, None);
    }
    ctx.window_backend().raise_window_visual_only(found);

    if options.warp_pointer {
        ctx.warp_cursor_to_client(found);
    }

    Ok(true)
}

/// Show a scratchpad after its model ownership has moved to another monitor.
///
/// A transfer must not change the globally selected monitor or steal focus;
/// those are governed by the transfer's explicit focus policy. The target is
/// therefore passed directly instead of being inferred from global selection.
pub(crate) fn show_transferred_scratchpad(ctx: &mut WmCtx, win: WindowId, monitor_id: MonitorId) {
    if ctx
        .core()
        .model()
        .client(win)
        .is_none_or(|client| !client.is_scratchpad() || client.is_scratchpad_visible())
    {
        return;
    }

    let _ = show_scratchpad_window_with_options(
        ctx,
        win,
        ScratchpadShowOptions {
            monitor_id,
            focus: false,
            warp_pointer: false,
        },
    );
}

pub fn scratchpad_show_all(ctx: &mut WmCtx) -> Option<String> {
    let scratchpad_names = scratchpad_names(ctx.core().model(), false);

    let mut shown_count = 0;

    for name in scratchpad_names {
        if scratchpad_show_name(ctx, &name).is_ok() {
            shown_count += 1;
        }
    }

    if shown_count > 0 {
        Some(format!(
            "shown {} scratchpad{}",
            shown_count,
            if shown_count == 1 { "" } else { "s" }
        ))
    } else {
        None
    }
}

pub fn scratchpad_hide_all(ctx: &mut WmCtx) -> Option<String> {
    let scratchpad_names = scratchpad_names(ctx.core().model(), true);

    let mut hidden_count = 0;

    for name in scratchpad_names {
        let was_visible = ctx.core().model().clients.values().any(|c| {
            c.is_scratchpad()
                && c.scratchpad().is_some_and(|sp| sp.name() == name)
                && c.is_scratchpad_visible()
        });
        scratchpad_hide_name(ctx, &name);
        if was_visible {
            hidden_count += 1;
        }
    }

    if hidden_count > 0 {
        Some(format!(
            "hid {} scratchpad{}",
            hidden_count,
            if hidden_count == 1 { "" } else { "s" }
        ))
    } else {
        None
    }
}

pub fn scratchpad_hide_name(ctx: &mut WmCtx, name: &str) {
    let Ok(found) = find_live_scratchpad(ctx, name) else {
        return;
    };
    hide_scratchpad_window(ctx, found);
}

/// Hide one scratchpad window whose liveness the caller already validated.
///
/// Edge-anchored scratchpads play their slide-out first and defer the logical
/// hide until the backend reports the animation finished; see
/// [`finish_scratchpad_hides`].
pub(crate) fn hide_scratchpad_window(ctx: &mut WmCtx, found: WindowId) {
    let direction = ctx
        .core()
        .state()
        .model
        .client(found)
        .and_then(|c| c.scratchpad().and_then(|sp| sp.direction()));

    let slide = {
        let Some(client) = ctx.core().model().client(found) else {
            return;
        };
        if !client.is_scratchpad_visible() {
            return;
        }
        let Some(mon) = ctx.core().model().monitor(client.monitor_id) else {
            return;
        };
        direction.map(|direction| {
            EdgeSlideRects::new(
                mon.visible_content_rect(&ctx.core().model().clients),
                direction,
                client.geo.size(),
            )
        })
    };

    match slide {
        Some(slide) => {
            // Animate the slide-out and defer the logical hide until the
            // backend reports the transition finished. Hiding synchronously
            // would unmap the window on the same tick and destroy the
            // animation it just started. The remembered focus stays on the
            // scratchpad so the deferred hide can hand focus back.
            ctx.move_resize(
                found,
                slide.hidden,
                MoveResizeOptions::animate_to(EMPHASIZED_ANIMATION_MILLIS),
            );
            ctx.core_mut()
                .pending_work_mut()
                .queue_pending_scratchpad_hide(found);
        }
        None => {
            let restore_focus = take_scratchpad_restore_focus(ctx, found);
            crate::client::visibility::hide_with_focus(ctx, found, restore_focus);
        }
    }
}

/// Consume a scratchpad's remembered pre-show focus target, if any.
fn take_scratchpad_restore_focus(ctx: &mut WmCtx, win: WindowId) -> Option<WindowId> {
    ctx.core_mut()
        .model_mut()
        .client_mut(win)
        .and_then(|client| client.scratchpad_mut())
        .and_then(|scratchpad| scratchpad.take_restore_focus())
}

/// Complete deferred edge-scratchpad hides whose exit animation has finished.
///
/// Backend animation ticks call this once their animation bookkeeping shows the
/// window is no longer transitioning. Each completion performs the logical
/// hide that [`scratchpad_hide_name`] postponed, preserving the focus
/// hand-off captured when the overlay was shown.
pub fn finish_scratchpad_hides(ctx: &mut WmCtx, wins: &[WindowId]) {
    for &win in wins {
        // The window may have been restored, remoted, or removed while its
        // exit animation played; only still-visible scratchpads conceal.
        let should_hide = ctx
            .core()
            .model()
            .client(win)
            .is_some_and(|client| client.is_scratchpad_visible());
        if !should_hide {
            continue;
        }
        let restore_focus = take_scratchpad_restore_focus(ctx, win);
        crate::client::visibility::hide_with_focus(ctx, win, restore_focus);
    }
}

pub fn scratchpad_toggle(ctx: &mut WmCtx, name: Option<&str>) {
    let name = match name {
        Some(n) => n,
        None => return,
    };

    let is_overview = ctx.core().model().is_overview_active();

    if is_overview {
        return;
    }

    let found = match find_live_scratchpad(ctx, name) {
        Ok(w) => w,
        Err(_) => return,
    };

    let Some(client) = ctx.core().model().client(found) else {
        return;
    };
    // A pending hide means the slide-out is still playing; the next toggle
    // must reverse it rather than restart an identical slide-out.
    let is_visible = client.is_scratchpad_visible()
        && !ctx.core().pending_work().has_pending_scratchpad_hide(found);

    if is_visible {
        scratchpad_hide_name(ctx, name);
    } else {
        let _ = scratchpad_show_name(ctx, name);
    }
}

/// Toggle a scratchpad on a specific monitor without moving the pointer.
///
/// This is used by pointer-edge triggers: warping from inside a hot corner
/// would leave its hysteresis zone and make the interaction unstable.
pub(crate) fn scratchpad_toggle_from_hot_corner(
    ctx: &mut WmCtx,
    name: &str,
    monitor_id: MonitorId,
) {
    if ctx.core().model().is_overview_active() {
        return;
    }
    let Ok(found) = find_live_scratchpad(ctx, name) else {
        return;
    };
    let Some(is_visible) = ctx
        .core()
        .model()
        .client(found)
        .map(|client| client.is_sticky)
    else {
        return;
    };
    // A pending hide means the slide-out is still playing; the next trigger
    // must reverse it rather than restart an identical slide-out.
    let is_visible = is_visible && !ctx.core().pending_work().has_pending_scratchpad_hide(found);

    if is_visible {
        scratchpad_hide_name(ctx, name);
    } else {
        // Focusing is monitor-relative in the WM model. Make the corner's
        // monitor current before attaching and focusing the scratchpad there.
        crate::focus::select_monitor(ctx, monitor_id);
        let _ = scratchpad_show_name_with_options(
            ctx,
            name,
            ScratchpadShowOptions {
                monitor_id,
                focus: true,
                warp_pointer: false,
            },
        );
    }
}

pub fn collect_scratchpad_info(model: &WmModel) -> Vec<ScratchpadInfo> {
    model
        .clients
        .values()
        .filter_map(|c| {
            let pos = model.monitors.position_of(c.monitor_id)?;
            ScratchpadInfo::from_client(c, pos)
        })
        .collect()
}

pub fn set_scratchpad_direction(ctx: &mut WmCtx, win: WindowId, direction: EdgeDirection) {
    let was_visible = ctx
        .core()
        .state()
        .model
        .client(win)
        .is_some_and(|c| c.is_scratchpad_visible());

    let (mon_ww, mon_wh) = {
        let Some(monitor_id) = ctx
            .core()
            .model()
            .client(win)
            .map(|client| client.monitor_id)
        else {
            return;
        };
        let Some(mon) = ctx.core().model().monitor(monitor_id) else {
            return;
        };
        (mon.work_rect().w, mon.work_rect().h)
    };

    if let Some(client) = ctx.core_mut().model_mut().client_mut(win)
        && let Some(sp) = client.scratchpad_mut()
    {
        sp.set_direction(direction);
        client.border_width = 0;
        client.is_locked = true;
        if direction.is_vertical() {
            client.geo.h = mon_wh / 3;
        } else {
            client.geo.w = mon_ww / 3;
        }
    }

    if was_visible {
        let Some(name) = ctx
            .core()
            .state()
            .model
            .client(win)
            .and_then(|c| c.scratchpad().map(|sp| sp.name().to_string()))
        else {
            return;
        };
        scratchpad_hide_name(ctx, &name);
        let _ = scratchpad_show_name(ctx, &name);
    }
}

pub fn edge_scratchpad_create(ctx: &mut WmCtx) {
    if let Ok(existing) = find_live_scratchpad(ctx, DEFAULT_EDGE_SCRATCHPAD_NAME) {
        let _ = scratchpad_restore_window(ctx, existing, None);
        return;
    }

    let _ = scratchpad_create(
        ctx,
        DEFAULT_EDGE_SCRATCHPAD_NAME,
        None,
        Some(EdgeDirection::Top),
        ScratchpadInitialStatus::Shown,
    );
}

#[cfg(test)]
mod tests;
