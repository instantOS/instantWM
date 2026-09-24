//! X11 XRandR support for display configuration.

use crate::backend::BackendOutputInfo;
use crate::backend::BackendVrrSupport;
use crate::backend::output::{
    MonitorModeRequest, OutputMode, OutputPlacement, OutputPositionSource,
    plan_automatic_output_positions, position_after,
};
use crate::config::config_toml::{MirrorFit, MonitorConfig};
use crate::output_mirror::MonitorPolicy;
use crate::types::{MonitorPosition, Point, Rect, Size};
use std::collections::{HashMap, HashSet};
use x11rb::connection::Connection;
use x11rb::protocol::randr::{self, ConnectionExt as RandrExt};
use x11rb::protocol::xproto::{ConnectionExt as XprotoExt, Window};
use x11rb::rust_connection::RustConnection;

fn mode_refresh_millihertz(mode: &randr::ModeInfo) -> Option<u32> {
    let mut numerator = u64::from(mode.dot_clock).saturating_mul(1000);
    let mut divisor = u64::from(mode.htotal).checked_mul(u64::from(mode.vtotal))?;
    if numerator == 0 || divisor == 0 {
        return None;
    }

    let flags = u32::from(mode.mode_flags);
    if flags & u32::from(randr::ModeFlag::INTERLACE) != 0 {
        numerator = numerator.saturating_mul(2);
    }
    if flags & u32::from(randr::ModeFlag::DOUBLE_SCAN) != 0 {
        divisor = divisor.saturating_mul(2);
    }

    u32::try_from(numerator / divisor).ok()
}

struct RandrOutput {
    id: randr::Output,
    name: String,
    info: randr::GetOutputInfoReply,
}

/// One consistent view of the RandR screen resources: every output, every
/// CRTC and the mode table, all fetched against one config timestamp.
pub struct RandrSnapshot {
    config_timestamp: u32,
    modes: Vec<randr::ModeInfo>,
    outputs: Vec<RandrOutput>,
    crtcs: HashMap<randr::Crtc, randr::GetCrtcInfoReply>,
}

impl RandrSnapshot {
    /// Fetch the current resources, falling back to the hardware-probing
    /// legacy request only when the cheap one reports no connected output
    /// (the server has not probed yet).
    pub fn fetch(conn: &RustConnection, root: Window) -> Option<Self> {
        let current = conn
            .randr_get_screen_resources_current(root)
            .ok()
            .and_then(|cookie| cookie.reply().ok())
            .map(|r| Self::resolve(conn, r.config_timestamp, &r.outputs, &r.crtcs, r.modes));
        if current
            .as_ref()
            .is_some_and(|snapshot| snapshot.connected().next().is_some())
        {
            return current;
        }
        conn.randr_get_screen_resources(root)
            .ok()
            .and_then(|cookie| cookie.reply().ok())
            .map(|r| Self::resolve(conn, r.config_timestamp, &r.outputs, &r.crtcs, r.modes))
            .or(current)
    }

    /// Pipeline every output and CRTC query before collecting any reply.
    fn resolve(
        conn: &RustConnection,
        config_timestamp: u32,
        output_ids: &[randr::Output],
        crtc_ids: &[randr::Crtc],
        modes: Vec<randr::ModeInfo>,
    ) -> Self {
        let output_cookies: Vec<_> = output_ids
            .iter()
            .filter_map(|&id| Some((id, conn.randr_get_output_info(id, config_timestamp).ok()?)))
            .collect();
        let crtc_cookies: Vec<_> = crtc_ids
            .iter()
            .filter_map(|&id| Some((id, conn.randr_get_crtc_info(id, config_timestamp).ok()?)))
            .collect();
        let outputs = output_cookies
            .into_iter()
            .filter_map(|(id, cookie)| {
                let info = cookie.reply().ok()?;
                Some(RandrOutput {
                    id,
                    name: String::from_utf8_lossy(&info.name).into_owned(),
                    info,
                })
            })
            .collect();
        let crtcs = crtc_cookies
            .into_iter()
            .filter_map(|(id, cookie)| Some((id, cookie.reply().ok()?)))
            .collect();
        Self {
            config_timestamp,
            modes,
            outputs,
            crtcs,
        }
    }

    fn connected(&self) -> impl Iterator<Item = &RandrOutput> {
        self.outputs
            .iter()
            .filter(|output| output.info.connection == randr::Connection::CONNECTED)
    }

    fn output(&self, name: &str) -> Option<&RandrOutput> {
        self.connected().find(|output| output.name == name)
    }

    fn mode(&self, id: randr::Mode) -> Option<&randr::ModeInfo> {
        self.modes.iter().find(|mode| mode.id == id)
    }

    /// Physical connector identity, independent of CRTC state.
    pub fn connected_names(&self) -> HashSet<String> {
        self.connected().map(|output| output.name.clone()).collect()
    }

    /// Connected outputs being scanned out, with their framebuffer region.
    ///
    /// A connected output without a CRTC is a physical head, not a logical
    /// monitor: publishing it at an invented position would create a phantom
    /// monitor overlapping the real desktop.
    fn active(&self) -> impl Iterator<Item = (&RandrOutput, Rect)> {
        self.connected().filter_map(|output| {
            let crtc = self.crtcs.get(&output.info.crtc)?;
            let (w, h) = self
                .mode(crtc.mode)
                .map_or((crtc.width, crtc.height), |mode| (mode.width, mode.height));
            Some((
                output,
                Rect::new(
                    i32::from(crtc.x),
                    i32::from(crtc.y),
                    i32::from(w),
                    i32::from(h),
                ),
            ))
        })
    }

    pub fn active_names(&self) -> HashSet<String> {
        self.active()
            .map(|(output, _)| output.name.clone())
            .collect()
    }

    fn active_rects(&self) -> Vec<(String, Rect)> {
        self.active()
            .map(|(output, rect)| (output.name.clone(), rect))
            .collect()
    }

    pub fn active_outputs(&self) -> Vec<BackendOutputInfo> {
        self.active()
            .map(|(output, rect)| BackendOutputInfo {
                name: output.name.clone(),
                rect,
                scale: 1.0,
                vrr_support: BackendVrrSupport::Unsupported,
                vrr_mode: None,
                vrr_enabled: false,
                mirrors: Vec::new(),
            })
            .collect()
    }

    /// Every mode advertised by connected output `name`.
    pub fn modes_of(&self, name: &str) -> Vec<OutputMode> {
        let Some(output) = self.output(name) else {
            return Vec::new();
        };
        let mut modes: Vec<_> = output
            .info
            .modes
            .iter()
            .filter_map(|&id| self.mode(id))
            .filter_map(|mode| {
                Some(OutputMode {
                    width: i32::from(mode.width),
                    height: i32::from(mode.height),
                    refresh_millihertz: i32::try_from(mode_refresh_millihertz(mode)?).ok()?,
                })
            })
            .collect();
        modes.sort_by_key(|mode| (mode.width, mode.height, mode.refresh_millihertz));
        modes.dedup();
        modes
    }

    /// The fastest refresh rate any CRTC currently scans out.
    ///
    /// X11 has one geometry-update stream for all outputs, so pacing it for
    /// the fastest active output avoids undersampling animations on
    /// mixed-refresh desktops.
    pub fn max_active_refresh_millihertz(&self) -> Option<u32> {
        self.crtcs
            .values()
            .filter_map(|crtc| mode_refresh_millihertz(self.mode(crtc.mode)?))
            .max()
    }
}

/// Every mode advertised by a connected RandR output.
pub fn get_output_modes(conn: &RustConnection, root: Window, output_name: &str) -> Vec<OutputMode> {
    RandrSnapshot::fetch(conn, root)
        .map_or_else(Vec::new, |snapshot| snapshot.modes_of(output_name))
}

/// How a CRTC request chooses its mode. Every variant falls back to the
/// output's current mode, then its preferred mode, so an unavailable request
/// never switches to an unexpected resolution.
#[derive(Debug, Clone, Copy, PartialEq)]
enum ModeRequest {
    Current,
    Preferred,
    Match(MonitorModeRequest),
}

/// Typed desired state for one output's CRTC.
#[derive(Debug, Clone, PartialEq)]
struct CrtcRequest {
    enable: bool,
    mode: ModeRequest,
    /// `None` keeps the output's current position, or places a newly
    /// enabled output right of the layout.
    position: Option<MonitorPosition>,
}

impl CrtcRequest {
    const DISABLE: Self = Self {
        enable: false,
        mode: ModeRequest::Current,
        position: None,
    };

    fn from_config(config: &MonitorConfig) -> Self {
        Self {
            enable: config.enable != Some(false),
            mode: config
                .resolution
                .as_deref()
                .and_then(|resolution| MonitorModeRequest::parse(resolution, config.refresh_rate))
                .map_or(ModeRequest::Current, ModeRequest::Match),
            position: config.position.as_deref().map(|position| {
                MonitorPosition::parse(position).unwrap_or_else(|| {
                    log::warn!("invalid monitor position {position:?}, using 0,0");
                    MonitorPosition::Absolute(Point::default())
                })
            }),
        }
    }

    fn at(position: Point) -> Self {
        Self {
            enable: true,
            mode: ModeRequest::Current,
            position: Some(MonitorPosition::Absolute(position)),
        }
    }
}

/// One `SetCrtcConfig` request.
struct CrtcChange {
    crtc: randr::Crtc,
    x: i16,
    y: i16,
    mode: randr::Mode,
    rotation: randr::Rotation,
    outputs: Vec<randr::Output>,
    /// Bottom-right corner the framebuffer must contain first.
    extent: Option<(i32, i32)>,
}

/// Applies CRTC requests against a snapshot that is refetched only after a
/// CRTC actually changed.
struct RandrConfigurator<'a> {
    conn: &'a RustConnection,
    root: Window,
    snapshot: RandrSnapshot,
    stale: bool,
}

impl<'a> RandrConfigurator<'a> {
    fn new(conn: &'a RustConnection, root: Window) -> Option<Self> {
        Some(Self {
            conn,
            root,
            snapshot: RandrSnapshot::fetch(conn, root)?,
            stale: false,
        })
    }

    fn snapshot(&mut self) -> &RandrSnapshot {
        if self.stale
            && let Some(snapshot) = RandrSnapshot::fetch(self.conn, self.root)
        {
            self.snapshot = snapshot;
            self.stale = false;
        }
        &self.snapshot
    }

    fn configure(&mut self, name: &str, request: &CrtcRequest) {
        let Some(change) = plan_crtc_change(self.snapshot(), name, request) else {
            return;
        };
        if let Some((right, bottom)) = change.extent {
            self.grow_framebuffer(right, bottom);
        }
        let status = self
            .conn
            .randr_set_crtc_config(
                change.crtc,
                x11rb::CURRENT_TIME,
                self.snapshot.config_timestamp,
                change.x,
                change.y,
                change.mode,
                change.rotation,
                &change.outputs,
            )
            .ok()
            .and_then(|cookie| cookie.reply().ok())
            .map(|reply| reply.status);
        if status != Some(randr::SetConfig::SUCCESS) {
            log::warn!("RandR rejected the configuration of output {name}: {status:?}");
        }
        self.stale = true;
    }

    fn screen_size(&self) -> Option<(u16, u16)> {
        let geometry = self.conn.get_geometry(self.root).ok()?.reply().ok()?;
        Some((geometry.width, geometry.height))
    }

    fn grow_framebuffer(&self, right: i32, bottom: i32) {
        let Some((width, height)) = self.screen_size() else {
            return;
        };
        let (Ok(required_width), Ok(required_height)) = (
            u16::try_from(right.max(i32::from(width))),
            u16::try_from(bottom.max(i32::from(height))),
        ) else {
            return;
        };
        if (required_width, required_height) != (width, height) {
            self.set_framebuffer_size(required_width, required_height);
        }
    }

    /// Resize the framebuffer to exactly contain all active outputs, so
    /// unplugging an edge output does not leave applications observing a
    /// permanently oversized root window.
    fn fit_framebuffer(&mut self) {
        let rects = self.snapshot().active_rects();
        let (Some(right), Some(bottom)) = (
            rects.iter().map(|(_, rect)| rect.right()).max(),
            rects.iter().map(|(_, rect)| rect.bottom()).max(),
        ) else {
            return;
        };
        let (Ok(width), Ok(height)) = (u16::try_from(right.max(1)), u16::try_from(bottom.max(1)))
        else {
            return;
        };
        if self
            .screen_size()
            .is_some_and(|size| size != (width, height))
        {
            self.set_framebuffer_size(width, height);
        }
    }

    fn set_framebuffer_size(&self, width: u16, height: u16) {
        let Some(screen) = self
            .conn
            .setup()
            .roots
            .iter()
            .find(|screen| screen.root == self.root)
        else {
            return;
        };
        let mm_width = u32::from(screen.width_in_millimeters)
            .saturating_mul(u32::from(width))
            .checked_div(u32::from(screen.width_in_pixels).max(1))
            .unwrap_or(u32::from(screen.width_in_millimeters));
        let mm_height = u32::from(screen.height_in_millimeters)
            .saturating_mul(u32::from(height))
            .checked_div(u32::from(screen.height_in_pixels).max(1))
            .unwrap_or(u32::from(screen.height_in_millimeters));
        if let Ok(cookie) = self
            .conn
            .randr_set_screen_size(self.root, width, height, mm_width, mm_height)
        {
            let _ = cookie.check();
        }
    }
}

/// Whether `current` already scans out `output` at this position and mode.
fn crtc_shows(
    current: &randr::GetCrtcInfoReply,
    x: i16,
    y: i16,
    mode: randr::Mode,
    output: randr::Output,
) -> bool {
    current.x == x
        && current.y == y
        && current.mode == mode
        && current.rotation == randr::Rotation::ROTATE0
        && current.outputs.contains(&output)
}

/// The `SetCrtcConfig` that realizes `request` for connected output `name`,
/// or `None` when nothing needs to change (or nothing can).
fn plan_crtc_change(
    snapshot: &RandrSnapshot,
    name: &str,
    request: &CrtcRequest,
) -> Option<CrtcChange> {
    let output = snapshot.output(name)?;
    let free_crtc = |exclude: randr::Crtc| {
        output.info.crtcs.iter().copied().find(|&candidate| {
            candidate != exclude
                && snapshot
                    .crtcs
                    .get(&candidate)
                    .is_some_and(|info| info.outputs.is_empty())
        })
    };

    if !request.enable {
        let current_crtc = output.info.crtc;
        if current_crtc == 0 {
            return None;
        }
        // RandR replaces a CRTC's entire outputs array; keep any other head
        // sharing this CRTC scanning out.
        let current = snapshot.crtcs.get(&current_crtc);
        let remaining: Vec<_> = current
            .map(|info| {
                info.outputs
                    .iter()
                    .copied()
                    .filter(|&other| other != output.id)
                    .collect()
            })
            .unwrap_or_default();
        let (x, y, mode, rotation) = match current {
            Some(info) if !remaining.is_empty() => (info.x, info.y, info.mode, info.rotation),
            _ => (0, 0, 0, randr::Rotation::ROTATE0),
        };
        return Some(CrtcChange {
            crtc: current_crtc,
            x,
            y,
            mode,
            rotation,
            outputs: remaining,
            extent: None,
        });
    }

    let mut crtc = if output.info.crtc != 0 {
        output.info.crtc
    } else {
        free_crtc(0)?
    };
    let current = snapshot.crtcs.get(&crtc);
    let mode = select_output_mode(
        &output.info,
        current.map(|current| current.mode),
        request.mode,
        &snapshot.modes,
    )?;
    let known = snapshot.active_rects();
    let position = match &request.position {
        Some(position) => position
            .resolve(
                Size::new(i32::from(mode.width), i32::from(mode.height)),
                known.iter().map(|(name, rect)| (name.as_str(), *rect)),
            )
            .unwrap_or_default(),
        None => known
            .iter()
            .find(|(known, _)| known == name)
            .map(|(_, rect)| Point::new(rect.x, rect.y))
            .unwrap_or_else(|| position_after(known.iter().map(|(_, rect)| *rect))),
    };
    let (Ok(x), Ok(y)) = (i16::try_from(position.x), i16::try_from(position.y)) else {
        log::warn!("RandR output position is outside the protocol range: {position:?}");
        return None;
    };

    if let Some(current) = current {
        if crtc == output.info.crtc && crtc_shows(current, x, y, mode.id, output.id) {
            return None;
        }
        if current.outputs.len() > 1 {
            // Move this output to a free compatible CRTC before changing its
            // mode or location, so the other heads keep scanning out.
            let Some(spare) = free_crtc(crtc) else {
                log::warn!(
                    "cannot reconfigure output {name}: it shares CRTC {crtc} and no free compatible CRTC exists"
                );
                return None;
            };
            crtc = spare;
        }
    }

    Some(CrtcChange {
        crtc,
        x,
        y,
        mode: mode.id,
        rotation: randr::Rotation::ROTATE0,
        outputs: vec![output.id],
        extent: Some((
            position.x.saturating_add(i32::from(mode.width)),
            position.y.saturating_add(i32::from(mode.height)),
        )),
    })
}

fn select_output_mode(
    output_info: &randr::GetOutputInfoReply,
    current_mode: Option<randr::Mode>,
    request: ModeRequest,
    modes: &[randr::ModeInfo],
) -> Option<randr::ModeInfo> {
    let current = || current_mode.and_then(|id| modes.iter().find(|mode| mode.id == id).copied());
    let preferred = || find_preferred_mode(output_info, modes);
    match request {
        ModeRequest::Match(request) => find_mode_by_resolution(output_info, modes, request)
            .or_else(current)
            .or_else(preferred),
        ModeRequest::Current => current().or_else(preferred),
        ModeRequest::Preferred => preferred().or_else(current),
    }
}

/// The EDID-preferred mode: the first one the output advertises.
fn find_preferred_mode(
    output_info: &randr::GetOutputInfoReply,
    modes: &[randr::ModeInfo],
) -> Option<randr::ModeInfo> {
    output_info
        .modes
        .first()
        .and_then(|mode_id| modes.iter().find(|m| &m.id == mode_id).copied())
}

fn find_mode_by_resolution(
    output_info: &randr::GetOutputInfoReply,
    modes: &[randr::ModeInfo],
    request: MonitorModeRequest,
) -> Option<randr::ModeInfo> {
    output_info
        .modes
        .iter()
        .filter_map(|id| modes.iter().find(|mode| mode.id == *id))
        .find(|mode| {
            request.matches(
                i32::from(mode.width),
                i32::from(mode.height),
                mode_refresh_millihertz(mode),
            )
        })
        .copied()
}

/// Apply the complete monitor policy and settle the layout.
pub fn apply_output_policy(
    conn: &RustConnection,
    runtime: &mut crate::backend::x11::X11RuntimeConfig,
    policy: &MonitorPolicy,
) {
    if let Some(mut randr) = RandrConfigurator::new(conn, runtime.root) {
        apply_policy(&mut randr, runtime, policy);
    }
}

/// Pass 1 configures every output that is not about to mirror a presenting
/// source, pass 2 glues declared mirrors onto the state pass 1 produced and
/// releases heads that stopped mirroring, then automatic outputs are
/// compacted (re-gluing mirrors whose source moved) and the framebuffer is
/// fitted to the result.
fn apply_policy(
    randr: &mut RandrConfigurator<'_>,
    runtime: &mut crate::backend::x11::X11RuntimeConfig,
    policy: &MonitorPolicy,
) {
    apply_monitor_configs(randr, policy);
    apply_mirror_configs(
        randr,
        policy,
        &mut runtime.mirror_heads,
        &mut runtime.automatic_outputs,
    );
    if compact_automatic_output_layout(
        randr,
        policy,
        &runtime.automatic_outputs,
        &runtime.mirror_heads,
    ) {
        apply_mirror_configs(
            randr,
            policy,
            &mut runtime.mirror_heads,
            &mut runtime.automatic_outputs,
        );
    }
    randr.fit_framebuffer();
}

/// Reconcile a RandR topology change: queue newly plugged connectors for
/// automatic activation, re-apply the policy and record the resulting
/// connector and CRTC state for the next change.
pub fn refresh_topology(
    conn: &RustConnection,
    runtime: &mut crate::backend::x11::X11RuntimeConfig,
    policy: &MonitorPolicy,
) {
    let Some(mut randr) = RandrConfigurator::new(conn, runtime.root) else {
        return;
    };
    let connected = randr.snapshot().connected_names();
    let active_before = randr.snapshot().active_names();

    // A CRTC disappearing while its physical connector remains present is an
    // external disable, not a hot-plug. Relinquish automatic placement and do
    // not queue it for re-enabling.
    for name in runtime
        .active_outputs
        .difference(&active_before)
        .filter(|name| connected.contains(*name))
    {
        runtime.automatic_outputs.remove(name);
    }
    runtime
        .pending_output_enable
        .retain(|name| connected.contains(name) && !policy.is_explicitly_disabled(name));
    runtime
        .automatic_outputs
        .retain(|name| connected.contains(name));
    let candidates = new_auto_enable_candidates(
        &runtime.connected_outputs,
        &connected,
        &active_before,
        policy,
    );
    runtime.pending_output_enable.extend(candidates);
    runtime.connected_outputs = connected;

    let automatic = configure_new_outputs(&mut randr, policy, &runtime.pending_output_enable);
    runtime.automatic_outputs.extend(automatic);
    apply_policy(&mut randr, runtime, policy);

    let active_after = randr.snapshot().active_names();
    runtime
        .pending_output_enable
        .retain(|name| !active_after.contains(name));
    runtime
        .automatic_outputs
        .retain(|name| active_after.contains(name));
    runtime
        .mirror_heads
        .retain(|name| active_after.contains(name));
    runtime.active_outputs = active_after;
}

/// Declared mirrors whose source will present after pass 1 (connected and
/// not disabled by policy). Pass 2 owns their policy; every other head,
/// including a mirror of an absent or disabled source, is configured by
/// pass 1 as an ordinary output.
fn gluable_mirrors(policy: &MonitorPolicy, connected: &HashSet<String>) -> HashSet<String> {
    policy
        .mirrors
        .active_pairs(|name| connected.contains(name) && !policy.is_explicitly_disabled(name))
        .map(|(mirror, _)| mirror.to_string())
        .collect()
}

/// Pass 1: apply exactly one effective policy per connected output. A named
/// entry shadows the wildcard instead of relying on two order-dependent
/// modesets.
fn apply_monitor_configs(randr: &mut RandrConfigurator<'_>, policy: &MonitorPolicy) {
    let connected = randr.snapshot().connected_names();
    let gluable = gluable_mirrors(policy, &connected);
    let mut names: Vec<_> = connected.difference(&gluable).collect();
    names.sort();
    for name in names {
        if let Some(config) = policy.effective(name) {
            randr.configure(name, &CrtcRequest::from_config(config));
        }
    }
}

/// Pass 2: point declared mirrors at their presenting source and release the
/// heads that stopped mirroring.
///
/// `mirror_heads` records the heads this policy glued, so only those are ever
/// released; outputs cloned by other tools (`xrandr --same-as`) are left
/// alone and merely fold into one monitor. A mirror whose source is not
/// presenting behaves as an ordinary output until the source returns.
fn apply_mirror_configs(
    randr: &mut RandrConfigurator<'_>,
    policy: &MonitorPolicy,
    mirror_heads: &mut HashSet<String>,
    automatic_outputs: &mut HashSet<String>,
) {
    if policy.mirrors.is_empty() && mirror_heads.is_empty() {
        return;
    }
    // Sources are never mirrors, so this pass does not move them: one
    // snapshot answers every source and mode lookup.
    let snapshot = randr.snapshot();
    let connected = snapshot.connected_names();
    let source_rects: HashMap<String, Rect> = snapshot.active_rects().into_iter().collect();
    let glued: Vec<(String, CrtcRequest)> = policy
        .mirrors
        .iter()
        .filter(|(mirror, _)| connected.contains(*mirror))
        .filter_map(|(mirror, target)| {
            let rect = *source_rects.get(&target.source)?;
            let modes = snapshot.modes_of(mirror);
            let request = mirror_policy_for(policy, mirror, &target.source, rect, &modes);
            Some((mirror.clone(), request))
        })
        .collect();
    mirror_heads.retain(|name| connected.contains(name));

    for (mirror, request) in glued {
        randr.configure(&mirror, &request);
        automatic_outputs.remove(&mirror);
        if request.enable {
            mirror_heads.insert(mirror);
        } else {
            mirror_heads.remove(&mirror);
        }
    }

    let mut released: Vec<String> = mirror_heads
        .iter()
        .filter(|name| {
            policy
                .mirrors
                .source_of(name)
                .is_none_or(|source| !source_rects.contains_key(source))
        })
        .cloned()
        .collect();
    released.sort();
    for name in released {
        mirror_heads.remove(&name);
        release_mirror_head(randr, policy, &name, mirror_heads, automatic_outputs);
    }
}

/// Turn a head that stopped mirroring back into an ordinary output: its own
/// policy, its preferred mode unless configured, and, without a configured
/// position, an automatic place right of the layout.
fn release_mirror_head(
    randr: &mut RandrConfigurator<'_>,
    policy: &MonitorPolicy,
    name: &str,
    mirror_heads: &HashSet<String>,
    automatic_outputs: &mut HashSet<String>,
) {
    let default = MonitorConfig::default();
    let config = policy.effective(name).unwrap_or(&default);
    let mut request = CrtcRequest::from_config(config);
    if !request.enable {
        automatic_outputs.remove(name);
        randr.configure(name, &request);
        return;
    }
    if config.resolution.is_none() {
        request.mode = ModeRequest::Preferred;
    }
    if request.position.is_none() {
        let position = position_after(
            randr
                .snapshot()
                .active()
                .filter(|(output, _)| output.name != name && !mirror_heads.contains(&output.name))
                .map(|(_, rect)| rect),
        );
        request.position = Some(MonitorPosition::Absolute(position));
        automatic_outputs.insert(name.to_string());
    }
    log::info!("output {name} stopped mirroring and becomes an independent output");
    randr.configure(name, &request);
}

/// Pure policy for one declared mirror whose source presents `source_rect`:
/// the configuration the mirror must run to show the source.
///
/// X11 cannot scale mirrored content: Xorg shares one framebuffer across
/// CRTCs and instantWM never touches RandR output transforms, so a mirror
/// CRTC always scans out a 1:1 pixel region of the framebuffer at its
/// position. The policy is therefore a best-effort ladder over the mirror's
/// advertised modes, compared against the source's pixel size:
///
/// - An explicit `enable = false` on the mirror's effective policy wins over
///   mirroring.
/// - An exact source-sized mode clones the source rectangle (the panel's own
///   scaler fills a non-native mode, so this is a true scaled mirror).
/// - Otherwise the same-aspect mode with the nearest pixel area that is no
///   larger than the source (tie: higher refresh) shows a centered 1:1 crop
///   of the framebuffer region the source occupies.
/// - Otherwise only larger same-aspect or different-aspect modes exist,
///   which would display source pixels plus neighboring framebuffer
///   garbage; the mirror head is disabled instead. The source is presenting,
///   so this cannot leave a headless desktop.
///
/// The mirror's own `resolution`, `refresh_rate` and `mirror_fit` select a
/// scanout mode and fit on Wayland; X11 has to derive the mode from the
/// source and ignores them.
fn mirror_policy_for(
    policy: &MonitorPolicy,
    mirror: &str,
    source: &str,
    source_rect: Rect,
    mirror_modes: &[OutputMode],
) -> CrtcRequest {
    let config = policy.effective(mirror);
    if config.is_some_and(|config| config.enable == Some(false)) {
        return CrtcRequest::DISABLE;
    }
    if config.is_some_and(|config| {
        config.resolution.is_some() || config.mirror_fit == Some(MirrorFit::Cover)
    }) {
        log::debug!(
            "mirror output {mirror} configures a mode or fit, which X11 ignores: RandR mirrors follow the source 1:1"
        );
    }

    let clone_region = |width: i32, height: i32, position: Point| CrtcRequest {
        enable: true,
        mode: ModeRequest::Match(MonitorModeRequest::size(width, height)),
        position: Some(MonitorPosition::Absolute(position)),
    };

    // Rung 1: an exact mode clones the source rectangle. The panel's scaler
    // fills a non-native mode, so this mirrors the source scaled.
    let rect = source_rect;
    let (source_width, source_height) = (rect.w, rect.h);
    if mirror_modes
        .iter()
        .any(|mode| mode.width == source_width && mode.height == source_height)
    {
        return clone_region(source_width, source_height, Point::new(rect.x, rect.y));
    }

    // Rung 2: X11 cannot scale, so the closest lossless approximation is a
    // centered 1:1 crop of the framebuffer region the source occupies.
    if let Some(crop_mode) = find_same_aspect_crop_mode(mirror_modes, source_width, source_height) {
        let (crop_width, crop_height) = (crop_mode.width, crop_mode.height);
        log::warn!(
            "mirror output {mirror} has no {source_width}x{source_height} mode for source {source}; running {crop_width}x{crop_height} as an unscaled center crop (X11 mirrors cannot scale)"
        );
        return clone_region(
            crop_width,
            crop_height,
            Point::new(
                rect.x + (source_width - crop_width) / 2,
                rect.y + (source_height - crop_height) / 2,
            ),
        );
    }

    // Rung 3: any remaining mode would show source pixels plus neighboring
    // framebuffer garbage, so switch the mirror head off instead.
    log::error!(
        "mirror output {mirror} has no mode compatible with source {source} ({source_width}x{source_height}); disabling it because X11 cannot scale a larger or different-aspect mode"
    );
    CrtcRequest::DISABLE
}

/// The [`OutputMode`] a mirror runs for a centered 1:1 crop: the same-aspect
/// mode with the pixel area nearest to the source's, among modes no larger
/// than the source, with the higher refresh breaking area ties.
///
/// The aspect test is an integer cross-multiplication on raw pixel sizes —
/// X11 has no transform or fractional-scale concept (scale is hardcoded to
/// 1.0), so unlike the Wayland ladder there is nothing transform-adjusted to
/// compare. Equal aspect plus an area no larger than the source's forces
/// both mode dimensions to be no larger than the source's, which keeps the
/// crop's centering offset non-negative.
fn find_same_aspect_crop_mode(
    mirror_modes: &[OutputMode],
    source_width: i32,
    source_height: i32,
) -> Option<&OutputMode> {
    let source_area = i64::from(source_width) * i64::from(source_height);
    mirror_modes
        .iter()
        .filter(|mode| {
            i64::from(mode.width) * i64::from(source_height)
                == i64::from(mode.height) * i64::from(source_width)
        })
        .filter(|mode| i64::from(mode.width) * i64::from(mode.height) <= source_area)
        .min_by_key(|mode| {
            (
                (i64::from(mode.width) * i64::from(mode.height)).abs_diff(source_area),
                std::cmp::Reverse(mode.refresh_millihertz),
            )
        })
}

/// Attempt automatic activation only for connectors that the runtime has
/// identified as physically new. Returns the newly active outputs whose
/// placement is owned by the automatic policy.
///
/// Declared mirrors of a connected source are left to [`apply_policy`],
/// which glues them onto their source instead of giving them a placement.
fn configure_new_outputs(
    randr: &mut RandrConfigurator<'_>,
    policy: &MonitorPolicy,
    candidates: &HashSet<String>,
) -> HashSet<String> {
    let snapshot = randr.snapshot();
    let gluable = gluable_mirrors(policy, &snapshot.connected_names());
    let inactive: Vec<String> = snapshot
        .connected()
        .filter(|output| {
            output.info.crtc == 0
                && candidates.contains(&output.name)
                && !gluable.contains(&output.name)
        })
        .map(|output| output.name.clone())
        .collect();
    let default = MonitorConfig::default();
    for name in inactive {
        let request = CrtcRequest::from_config(policy.effective(&name).unwrap_or(&default));
        if request.enable {
            randr.configure(&name, &request);
        }
    }

    let active = randr.snapshot().active_names();
    candidates
        .iter()
        .filter(|name| active.contains(*name) && !gluable.contains(*name))
        .filter(|name| {
            policy
                .effective(name)
                .is_none_or(|config| config.position.is_none())
        })
        .cloned()
        .collect()
}

fn new_auto_enable_candidates(
    previous_connected: &HashSet<String>,
    connected: &HashSet<String>,
    active: &HashSet<String>,
    policy: &MonitorPolicy,
) -> HashSet<String> {
    connected
        .difference(previous_connected)
        .filter(|name| !active.contains(*name) && !policy.is_explicitly_disabled(name))
        .cloned()
        .collect()
}

/// Close holes left by removed automatically positioned outputs. Outputs with
/// an explicit named or wildcard position anchor the layout and are never
/// moved by this policy. Returns whether any output moved.
fn compact_automatic_output_layout(
    randr: &mut RandrConfigurator<'_>,
    policy: &MonitorPolicy,
    automatic_outputs: &HashSet<String>,
    mirror_heads: &HashSet<String>,
) -> bool {
    let outputs = randr.snapshot().active_rects();
    let moves = planned_automatic_positions(&outputs, policy, automatic_outputs, mirror_heads);
    for (name, position) in &moves {
        randr.configure(name, &CrtcRequest::at(*position));
    }
    !moves.is_empty()
}

fn planned_automatic_positions(
    outputs: &[(String, Rect)],
    policy: &MonitorPolicy,
    automatic_outputs: &HashSet<String>,
    mirror_heads: &HashSet<String>,
) -> Vec<(String, Point)> {
    let mut placements: Vec<_> = outputs
        .iter()
        // A mirror head presents its source's region; planning it as a
        // placement would mark that region occupied and shift the source.
        .filter(|(name, _)| !mirror_heads.contains(name))
        .map(|(name, rect)| {
            let automatic = automatic_outputs.contains(name)
                && policy
                    .effective(name)
                    .is_none_or(|config| config.position.is_none());
            OutputPlacement {
                id: name.clone(),
                rect: *rect,
                source: if automatic {
                    OutputPositionSource::Automatic
                } else {
                    OutputPositionSource::ClientManaged
                },
            }
        })
        .collect();
    plan_automatic_output_positions(&mut placements)
}
#[cfg(test)]
mod refresh_tests {
    use super::{
        CrtcRequest, ModeRequest, crtc_shows, find_mode_by_resolution, find_same_aspect_crop_mode,
        mirror_policy_for, mode_refresh_millihertz, new_auto_enable_candidates,
        planned_automatic_positions, select_output_mode,
    };
    use crate::backend::output::{MonitorModeRequest, OutputMode};
    use crate::config::config_toml::{MirrorFit, MonitorConfig};
    use crate::output_mirror::MonitorPolicy;
    use crate::types::{MonitorPosition, Point, Rect};
    use std::collections::{HashMap, HashSet};

    #[test]
    fn calculates_standard_and_high_refresh_modes() {
        assert_eq!(
            mode_refresh_millihertz(&test_mode(1, 1920, 1080, 148_500_000, 2200, 1125)),
            Some(60_000)
        );
        assert_eq!(
            mode_refresh_millihertz(&test_mode(2, 2560, 1440, 585_953_280, 2720, 1496)),
            Some(144_000)
        );
    }

    #[test]
    fn adjusts_refresh_for_interlaced_and_doublescan_modes() {
        let mut mode = test_mode(1, 1920, 1080, 74_250_000, 2200, 1125);
        mode.mode_flags = x11rb::protocol::randr::ModeFlag::INTERLACE;
        assert_eq!(mode_refresh_millihertz(&mode), Some(60_000));

        mode.dot_clock = 148_500_000;
        mode.mode_flags = x11rb::protocol::randr::ModeFlag::DOUBLE_SCAN;
        assert_eq!(mode_refresh_millihertz(&mode), Some(30_000));
    }

    #[test]
    fn rejects_incomplete_mode_timings() {
        assert_eq!(
            mode_refresh_millihertz(&test_mode(1, 1920, 1080, 0, 2200, 1125)),
            None
        );
        assert_eq!(
            mode_refresh_millihertz(&test_mode(1, 1920, 1080, 148_500_000, 0, 1125)),
            None
        );
    }

    fn test_mode(
        id: u32,
        width: u16,
        height: u16,
        dot_clock: u32,
        htotal: u16,
        vtotal: u16,
    ) -> x11rb::protocol::randr::ModeInfo {
        x11rb::protocol::randr::ModeInfo {
            id,
            width,
            height,
            dot_clock,
            htotal,
            vtotal,
            ..Default::default()
        }
    }

    #[test]
    fn refresh_rate_request_selects_matching_mode() {
        // 1920x1080 timings sharing the same blanking: 148.5 MHz is 60 Hz,
        // 356.4 MHz is 144 Hz.
        let modes = vec![
            test_mode(1, 1920, 1080, 148_500_000, 2200, 1125),
            test_mode(2, 1920, 1080, 356_400_000, 2200, 1125),
        ];
        let output_info = x11rb::protocol::randr::GetOutputInfoReply {
            modes: vec![1, 2],
            ..Default::default()
        };
        assert_eq!(
            find_mode_by_resolution(
                &output_info,
                &modes,
                MonitorModeRequest::parse("1920x1080", Some(144.0)).unwrap()
            )
            .map(|mode| mode.id),
            Some(2)
        );
        assert_eq!(
            find_mode_by_resolution(
                &output_info,
                &modes,
                MonitorModeRequest::parse("1920x1080", Some(60.0)).unwrap()
            )
            .map(|mode| mode.id),
            Some(1)
        );
    }

    #[test]
    fn unset_refresh_rate_keeps_first_match_and_unknown_rate_falls_back() {
        let modes = vec![
            test_mode(1, 1920, 1080, 148_500_000, 2200, 1125),
            test_mode(2, 1920, 1080, 356_400_000, 2200, 1125),
        ];
        let output_info = x11rb::protocol::randr::GetOutputInfoReply {
            modes: vec![1, 2],
            ..Default::default()
        };
        assert_eq!(
            find_mode_by_resolution(
                &output_info,
                &modes,
                MonitorModeRequest::parse("1920x1080", None).unwrap()
            )
            .map(|mode| mode.id),
            Some(1)
        );
        assert_eq!(
            find_mode_by_resolution(
                &output_info,
                &modes,
                MonitorModeRequest::parse("1920x1080", Some(165.0)).unwrap()
            )
            .map(|mode| mode.id),
            None
        );
    }

    #[test]
    fn requested_mode_must_be_advertised_by_the_output() {
        let modes = vec![
            test_mode(1, 1920, 1080, 148_500_000, 2200, 1125),
            test_mode(2, 2560, 1440, 585_953_280, 2720, 1496),
        ];
        let output_info = x11rb::protocol::randr::GetOutputInfoReply {
            modes: vec![2],
            ..Default::default()
        };
        let request = MonitorModeRequest::parse("1920x1080", None).unwrap();
        assert!(find_mode_by_resolution(&output_info, &modes, request).is_none());
    }

    #[test]
    fn unavailable_requested_mode_preserves_current_mode() {
        let modes = vec![
            test_mode(1, 1920, 1080, 148_500_000, 2200, 1125),
            test_mode(2, 3840, 2160, 594_000_000, 4400, 2250),
        ];
        let output_info = x11rb::protocol::randr::GetOutputInfoReply {
            modes: vec![2, 1],
            ..Default::default()
        };
        let request = CrtcRequest::from_config(&MonitorConfig {
            resolution: Some("1920x1080".to_string()),
            refresh_rate: Some(165.0),
            ..Default::default()
        })
        .mode;

        assert_eq!(
            select_output_mode(&output_info, Some(1), request, &modes).map(|mode| mode.id),
            Some(1)
        );
        assert_eq!(
            select_output_mode(&output_info, None, request, &modes).map(|mode| mode.id),
            Some(2)
        );
        assert_eq!(
            select_output_mode(&output_info, Some(1), ModeRequest::Preferred, &modes)
                .map(|mode| mode.id),
            Some(2)
        );
    }

    #[test]
    fn refresh_rate_tolerance_matches_wayland_backend() {
        let sixty = test_mode(1, 1920, 1080, 148_500_000, 2200, 1125);
        let matches = |refresh| {
            MonitorModeRequest::parse("1920x1080", refresh)
                .unwrap()
                .matches(1920, 1080, mode_refresh_millihertz(&sixty))
        };
        assert!(matches(Some(60.0)));
        assert!(matches(Some(59.94)));
        assert!(!matches(Some(165.0)));
        assert!(matches(None));
        let broken = test_mode(2, 1920, 1080, 0, 2200, 1125);
        assert!(
            !MonitorModeRequest::parse("1920x1080", Some(60.0))
                .unwrap()
                .matches(1920, 1080, mode_refresh_millihertz(&broken))
        );
    }

    #[test]
    fn unchanged_crtc_configuration_is_a_noop() {
        let current = x11rb::protocol::randr::GetCrtcInfoReply {
            status: x11rb::protocol::randr::SetConfig::SUCCESS,
            sequence: 0,
            length: 0,
            timestamp: 0,
            x: 1920,
            y: 0,
            width: 1920,
            height: 1080,
            mode: 7,
            rotation: x11rb::protocol::randr::Rotation::ROTATE0,
            rotations: x11rb::protocol::randr::Rotation::ROTATE0,
            outputs: vec![9],
            possible: vec![9],
        };

        assert!(crtc_shows(&current, 1920, 0, 7, 9));
        assert!(!crtc_shows(&current, 0, 0, 7, 9));
        assert!(!crtc_shows(&current, 1920, 0, 8, 9));
        assert!(!crtc_shows(&current, 1920, 0, 7, 10));
    }

    #[test]
    fn automatic_layout_closes_holes_but_preserves_explicit_anchors() {
        let outputs = vec![
            output("DP-1", Rect::new(1920, 0, 1920, 1080)),
            output("HDMI-1", Rect::new(5000, 0, 1920, 1080)),
        ];
        let automatic: HashSet<_> = ["DP-1".to_string(), "HDMI-1".to_string()]
            .into_iter()
            .collect();
        assert_eq!(
            planned_automatic_positions(
                &outputs,
                &MonitorPolicy::default(),
                &automatic,
                &HashSet::new()
            ),
            vec![
                ("DP-1".to_string(), Point::new(0, 0)),
                ("HDMI-1".to_string(), Point::new(1920, 0)),
            ]
        );

        let mut configs = HashMap::new();
        configs.insert(
            "DP-1".to_string(),
            MonitorConfig {
                position: Some("1920,0".to_string()),
                ..MonitorConfig::default()
            },
        );
        assert_eq!(
            planned_automatic_positions(
                &outputs,
                &MonitorPolicy::new(&configs),
                &automatic,
                &HashSet::new()
            ),
            vec![("HDMI-1".to_string(), Point::new(3840, 0))]
        );
    }

    #[test]
    fn external_crtc_disable_is_not_a_new_connector() {
        let connected: HashSet<_> = ["DP-1".to_string()].into_iter().collect();
        assert!(
            new_auto_enable_candidates(
                &connected,
                &connected,
                &HashSet::new(),
                &MonitorPolicy::default()
            )
            .is_empty()
        );
        assert_eq!(
            new_auto_enable_candidates(
                &HashSet::new(),
                &connected,
                &HashSet::new(),
                &MonitorPolicy::default(),
            ),
            connected
        );
    }

    fn output(name: &str, rect: Rect) -> (String, Rect) {
        (name.to_string(), rect)
    }

    fn clone_region(width: i32, height: i32, x: i32, y: i32) -> CrtcRequest {
        CrtcRequest {
            enable: true,
            mode: ModeRequest::Match(MonitorModeRequest::size(width, height)),
            position: Some(MonitorPosition::Absolute(Point::new(x, y))),
        }
    }

    /// `DP-1` declares `mirror = "eDP-1"` unless overridden by the caller.
    fn mirror_configs(
        extra: impl IntoIterator<Item = (&'static str, MonitorConfig)>,
    ) -> MonitorPolicy {
        let mut configs: HashMap<String, MonitorConfig> = extra
            .into_iter()
            .map(|(name, config)| (name.to_string(), config))
            .collect();
        configs.entry("DP-1".to_string()).or_insert(MonitorConfig {
            mirror: Some("eDP-1".to_string()),
            ..MonitorConfig::default()
        });
        MonitorPolicy::new(&configs)
    }

    fn mode(width: i32, height: i32, refresh_millihertz: i32) -> OutputMode {
        OutputMode {
            width,
            height,
            refresh_millihertz,
        }
    }

    #[test]
    fn mirror_policy_explicit_disable_wins() {
        let configs = mirror_configs([(
            "DP-1",
            MonitorConfig {
                mirror: Some("eDP-1".to_string()),
                enable: Some(false),
                ..MonitorConfig::default()
            },
        )]);
        let policy = mirror_policy_for(
            &configs,
            "DP-1",
            "eDP-1",
            Rect::new(0, 0, 1920, 1080),
            &[mode(1920, 1080, 60_000)],
        );
        assert_eq!(policy, CrtcRequest::DISABLE);
    }

    #[test]
    fn mirror_policy_follows_the_source_rectangle() {
        let configs = mirror_configs([]);
        let modes = [mode(1920, 1080, 60_000), mode(2560, 1440, 144_000)];

        let policy = mirror_policy_for(
            &configs,
            "DP-1",
            "eDP-1",
            Rect::new(1920, 40, 2560, 1440),
            &modes,
        );
        assert_eq!(policy, clone_region(2560, 1440, 1920, 40));
    }

    #[test]
    fn mirror_policy_crops_centered_when_only_smaller_same_aspect_modes_exist() {
        let configs = mirror_configs([]);
        let modes = [mode(1280, 720, 144_000), mode(1920, 1080, 60_000)];

        // X11 cannot scale, so the nearest smaller 16:9 mode shows a
        // centered 1:1 crop of the framebuffer region the source occupies:
        // (1920, 40) + ((2560 - 1920) / 2, (1440 - 1080) / 2).
        let policy = mirror_policy_for(
            &configs,
            "DP-1",
            "eDP-1",
            Rect::new(1920, 40, 2560, 1440),
            &modes,
        );
        assert_eq!(policy, clone_region(1920, 1080, 2240, 220));
    }

    #[test]
    fn mirror_policy_disables_mirror_without_a_compatible_mode() {
        let configs = mirror_configs([]);

        // Only larger same-aspect modes: a 1:1 CRTC would also show
        // neighboring framebuffer content, so the head is switched off.
        let policy = mirror_policy_for(
            &configs,
            "DP-1",
            "eDP-1",
            Rect::new(0, 0, 1600, 900),
            &[mode(1920, 1080, 60_000)],
        );
        assert_eq!(policy, CrtcRequest::DISABLE);

        // No same-aspect mode at all: same outcome.
        let policy = mirror_policy_for(
            &configs,
            "DP-1",
            "eDP-1",
            Rect::new(0, 0, 2560, 1440),
            &[mode(1920, 1200, 60_000)],
        );
        assert_eq!(policy, CrtcRequest::DISABLE);
    }

    #[test]
    fn mirror_policy_ignores_mode_and_fit_on_x11() {
        // A mirror's own mode and fit apply on Wayland only; on X11 they
        // must neither change the ladder's outcome nor error.
        let configs = mirror_configs([(
            "DP-1",
            MonitorConfig {
                mirror: Some("eDP-1".to_string()),
                mirror_fit: Some(MirrorFit::Cover),
                resolution: Some("1280x720".to_string()),
                ..MonitorConfig::default()
            },
        )]);
        let source = Rect::new(0, 0, 2560, 1440);

        let policy = mirror_policy_for(
            &configs,
            "DP-1",
            "eDP-1",
            source,
            &[mode(1280, 720, 60_000), mode(2560, 1440, 60_000)],
        );
        assert_eq!(policy, clone_region(2560, 1440, 0, 0));

        let policy = mirror_policy_for(
            &configs,
            "DP-1",
            "eDP-1",
            source,
            &[mode(1280, 720, 60_000)],
        );
        assert_eq!(policy, clone_region(1280, 720, 640, 360));
    }

    #[test]
    fn crop_mode_picker_prefers_nearest_area_then_higher_refresh() {
        // The nearest area wins even against a smaller higher-refresh mode.
        let modes = vec![mode(1280, 720, 144_000), mode(1920, 1080, 60_000)];
        let picked = find_same_aspect_crop_mode(&modes, 2560, 1440).unwrap();
        assert_eq!((picked.width, picked.height), (1920, 1080));

        // Area ties (duplicate resolution at two refresh rates) resolve to
        // the higher refresh.
        let modes = vec![mode(1280, 720, 60_000), mode(1280, 720, 144_000)];
        let picked = find_same_aspect_crop_mode(&modes, 2560, 1440).unwrap();
        assert_eq!(picked.refresh_millihertz, 144_000);
    }

    #[test]
    fn planned_positions_exclude_mirror_heads_so_the_source_keeps_its_rect() {
        // The mirror shares its source's rectangle and sorts before it, so
        // planning it would mark the rectangle occupied and shift the source
        // right, displacing whatever sits under the cursor.
        let outputs = vec![
            output("DP-1", Rect::new(0, 0, 1920, 1080)),
            output("eDP-1", Rect::new(0, 0, 1920, 1080)),
            output("HDMI-1", Rect::new(3840, 0, 1920, 1080)),
        ];
        let automatic: HashSet<_> = ["DP-1", "eDP-1", "HDMI-1"]
            .iter()
            .map(|name| name.to_string())
            .collect();
        let mirror_heads: HashSet<_> = ["DP-1".to_string()].into_iter().collect();

        assert_eq!(
            planned_automatic_positions(
                &outputs,
                &MonitorPolicy::default(),
                &automatic,
                &mirror_heads
            ),
            vec![("HDMI-1".to_string(), Point::new(1920, 0))]
        );
    }

    #[test]
    fn a_released_mirror_takes_part_in_placement_again() {
        // Once released, a former mirror is an ordinary automatic output.
        let outputs = vec![
            output("eDP-1", Rect::new(0, 0, 1920, 1080)),
            output("DP-1", Rect::new(3840, 0, 1920, 1080)),
        ];
        let automatic: HashSet<_> = ["DP-1".to_string()].into_iter().collect();

        assert_eq!(
            planned_automatic_positions(
                &outputs,
                &MonitorPolicy::default(),
                &automatic,
                &HashSet::new()
            ),
            vec![("DP-1".to_string(), Point::new(1920, 0))]
        );
    }
}
