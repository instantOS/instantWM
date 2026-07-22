use smithay::xwayland::{X11Surface, xwm::WmWindowType};
use x11rb::properties::{WmHints, WmSizeHints};

use crate::client::rules::WindowProperties;
use crate::model::WmModel;
use crate::types::{Client, ClientMode, MonitorId, SizeHints, WindowId};

/// Complete XWayland policy snapshot delivered by the backend.
pub(crate) struct XWaylandPolicyUpdate {
    pub hints: Option<WmHints>,
    pub size_hints: Option<WmSizeHints>,
    pub is_fullscreen: bool,
    pub is_hidden: bool,
    pub is_above: bool,
}

/// Owned scheduling information produced by one authoritative policy commit.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[must_use = "the outcome reports required layout and bar invalidation"]
pub(crate) struct XWaylandPolicyOutcome {
    monitor_id: MonitorId,
    layout_changed: bool,
    bar_changed: bool,
}

impl XWaylandPolicyOutcome {
    pub(crate) fn monitor_id(self) -> MonitorId {
        self.monitor_id
    }

    pub(crate) fn layout_changed(self) -> bool {
        self.layout_changed
    }

    pub(crate) fn bar_changed(self) -> bool {
        self.bar_changed
    }
}

#[derive(Clone, Copy, PartialEq)]
struct PolicyState {
    mode: ClientMode,
    hidden: bool,
    urgent: bool,
    fixed_size: bool,
    size_hints: SizeHints,
    min_aspect: f32,
    max_aspect: f32,
}

impl PolicyState {
    fn capture(client: &Client) -> Self {
        Self {
            mode: client.mode(),
            hidden: client.is_hidden,
            urgent: client.is_urgent,
            fixed_size: client.is_fixed_size,
            size_hints: client.size_hints,
            min_aspect: client.min_aspect,
            max_aspect: client.max_aspect,
        }
    }
}

/// Reconcile a complete XWayland policy update with one client lookup.
///
/// The returned value owns everything the runtime needs after the model borrow
/// ends. This prevents partially applied policy and makes forgotten layout/bar
/// scheduling visible in the transaction's API.
pub(crate) fn apply_xwayland_policy(
    model: &mut WmModel,
    win: WindowId,
    update: XWaylandPolicyUpdate,
) -> Option<XWaylandPolicyOutcome> {
    let clients = &mut model.clients;
    let monitors = &model.monitors;
    let client = clients.get_mut(&win)?;
    monitors.get(client.monitor_id)?;
    let before = PolicyState::capture(client);

    apply_wm_hints_to_client(client, update.hints);
    apply_size_hints_to_client(client, update.size_hints);
    crate::client::mode::set_client_fullscreen(client, update.is_fullscreen);
    client.is_hidden = update.is_hidden;

    if update.is_above && client.base_mode() != crate::types::BaseClientMode::Floating {
        client.float_geo = client.geo;
        client.set_base_mode(crate::types::BaseClientMode::Floating);
    }

    let after = PolicyState::capture(client);
    let layout_changed = before.mode != after.mode
        || before.hidden != after.hidden
        || before.fixed_size != after.fixed_size
        || before.size_hints != after.size_hints
        || before.min_aspect != after.min_aspect
        || before.max_aspect != after.max_aspect;
    let bar_changed =
        before.mode != after.mode || before.hidden != after.hidden || before.urgent != after.urgent;

    Some(XWaylandPolicyOutcome {
        monitor_id: client.monitor_id,
        layout_changed,
        bar_changed,
    })
}

pub fn window_properties_from_x11_surface(surface: &X11Surface) -> WindowProperties {
    WindowProperties {
        class: surface.class(),
        instance: surface.instance(),
        title: surface.title(),
        size_hints: None,
    }
}

pub fn apply_wm_hints_to_client(client: &mut Client, hints: Option<WmHints>) {
    let (never_focus, is_urgent) = hints
        .map(|hints| (!hints.input.unwrap_or(true), hints.urgent))
        .unwrap_or((false, false));
    client.never_focus = never_focus;
    client.is_urgent = is_urgent;
}

pub fn apply_size_hints_to_client(client: &mut Client, hints: Option<WmSizeHints>) {
    client.size_hints = Default::default();
    client.min_aspect = 0.0;
    client.max_aspect = 0.0;

    if let Some(hints) = hints {
        let base_size = hints.base_size.or(hints.min_size).unwrap_or((0, 0));
        client.size_hints.basew = base_size.0;
        client.size_hints.baseh = base_size.1;

        let min_size = hints.min_size.or(hints.base_size).unwrap_or((0, 0));
        client.size_hints.minw = min_size.0;
        client.size_hints.minh = min_size.1;

        let max_size = hints.max_size.unwrap_or((0, 0));
        client.size_hints.maxw = max_size.0;
        client.size_hints.maxh = max_size.1;

        let increments = hints.size_increment.unwrap_or((0, 0));
        client.size_hints.incw = increments.0;
        client.size_hints.inch = increments.1;

        if let Some((min_aspect, max_aspect)) = hints.aspect {
            if min_aspect.denominator != 0 {
                client.min_aspect = min_aspect.numerator as f32 / min_aspect.denominator as f32;
            }
            if max_aspect.denominator != 0 {
                client.max_aspect = max_aspect.numerator as f32 / max_aspect.denominator as f32;
            }
        }
    }

    client.is_fixed_size = client.size_hints.is_fixed();
    client.size_hints_valid = true;
}

pub fn should_float_for_x11_type(window_type: Option<WmWindowType>) -> bool {
    matches!(
        window_type,
        Some(
            WmWindowType::Dialog
                | WmWindowType::Utility
                | WmWindowType::Toolbar
                | WmWindowType::Splash
        )
    )
}
