//! Server-side implementation of `wlr-foreign-toplevel-management-unstable-v1`.
//!
//! This protocol lets third-party taskbars, docks, and pagers enumerate every
//! managed toplevel and control it (activate, close, maximize, minimize,
//! fullscreen). It is the Wayland counterpart of the EWMH state X11 exposes
//! through `_NET_CLIENT_LIST` and friends.
//!
//! Smithay ships `ext-foreign-toplevel-list` (read-only inventory) but no
//! management handler, so — like `output_management` — the
//! `GlobalDispatch`/`Dispatch` impls live here, using the generated types from
//! `wayland-protocols-wlr`.
//!
//! # Sync model
//!
//! The compositor pushes snapshots; this module owns per-client resources and
//! does the diffing:
//!
//! ```text
//! WaylandState::refresh_foreign_toplevel(win)      (focus / state / title change)
//!   └─► ForeignToplevelManagementState::sync_toplevel
//!         ├─ unknown window  → advertise handle to every manager instance
//!         │                    (except instances that destroyed the handle)
//!         └─ known window    → send only changed events + done
//! ```

use smithay::output::Output;
use smithay::reexports::wayland_protocols_wlr::foreign_toplevel::v1::server::{
    zwlr_foreign_toplevel_handle_v1::{self, ZwlrForeignToplevelHandleV1},
    zwlr_foreign_toplevel_manager_v1::{self, ZwlrForeignToplevelManagerV1},
};
use smithay::reexports::wayland_server::{
    Client, DataInit, Dispatch, DisplayHandle, GlobalDispatch, New, Resource,
    backend::{ClientId, GlobalId},
    protocol::wl_output::WlOutput,
};

use crate::types::WindowId;

// ---------------------------------------------------------------------------
// Public state types
// ---------------------------------------------------------------------------

/// Protocol-visible presentation of one managed window. Computed by the
/// backend from model + space state; this module never reads WM state.
#[derive(Debug, Clone)]
pub struct ToplevelSnapshot {
    pub title: String,
    pub app_id: String,
    pub activated: bool,
    pub minimized: bool,
    pub maximized: bool,
    pub fullscreen: bool,
    /// Managed parent, as advertised by `xdg_toplevel.set_parent` or
    /// `WM_TRANSIENT_FOR`.
    pub parent: Option<WindowId>,
    /// Outputs the window's geometry currently intersects.
    pub outputs: Vec<Output>,
}

/// A control request issued by a managing client.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ForeignToplevelRequest {
    Activate,
    Close,
    SetMaximized(bool),
    SetMinimized(bool),
    SetFullscreen(bool),
}

/// Top-level state for wlr-foreign-toplevel-management.
pub struct ForeignToplevelManagementState {
    /// The Wayland global. Held so we can remove it later if we ever need to
    /// tear the protocol down cleanly.
    #[allow(dead_code)]
    global: GlobalId,
    dh: DisplayHandle,
    /// Per-client manager instances (one per bind).
    instances: Vec<ManagerInstance>,
    /// Windows currently managed through this protocol, in advertisement
    /// order. Kept independently of instances so a late-binding client can be
    /// replayed from [`ForeignToplevelHandler::foreign_toplevel_snapshot`].
    windows: Vec<WindowId>,
}

/// One bound `zwlr_foreign_toplevel_manager_v1`.
#[derive(Debug)]
struct ManagerInstance {
    obj: ZwlrForeignToplevelManagerV1,
    toplevels: Vec<ToplevelInstance>,
    /// Windows whose handle this client destroyed while the window lives.
    /// The protocol sends `toplevel` only for *new* toplevels, so a
    /// client-destroyed handle must never be re-advertised (wlroots behaves
    /// the same). Cleared when the window is unmanaged.
    suppressed: Vec<WindowId>,
}

/// One advertised handle on one manager instance, with the last-sent values
/// so updates can diff instead of resending everything.
#[derive(Debug)]
struct ToplevelInstance {
    obj: ZwlrForeignToplevelHandleV1,
    window: WindowId,
    title: Option<String>,
    app_id: Option<String>,
    states_mask: Option<u32>,
    /// Last representable parent sent to the client. The outer option tracks
    /// whether the mandatory initial value has been sent.
    parent: Option<Option<WindowId>>,
    /// The client's own `wl_output` resources that were last announced with
    /// `output_enter` — what was actually *sent*, not the compositor-side
    /// output set. Diffing against the sent list means a client that binds
    /// `wl_output` late still receives its `output_enter` on the next
    /// refresh instead of both sides resolving to the same bindings.
    outputs: Vec<WlOutput>,
    /// `closed` makes the protocol object inert until the client sends its
    /// destructor request, as required by the protocol.
    closed: bool,
}

impl ToplevelInstance {
    const MAXIMIZED: u32 = 1 << 0;
    const MINIMIZED: u32 = 1 << 1;
    const ACTIVATED: u32 = 1 << 2;
    const FULLSCREEN: u32 = 1 << 3;
}

/// Data attached to the global.
pub struct ForeignToplevelGlobalData {
    /// Filter closure — currently always-true (all clients may manage).
    _filter: Box<dyn Fn(&Client) -> bool + Send + Sync>,
}

// ---------------------------------------------------------------------------
// Trait that WaylandState must implement
// ---------------------------------------------------------------------------

/// Handler trait bridging the protocol to instantWM's window management.
pub trait ForeignToplevelHandler {
    /// Access the mutable protocol state.
    fn foreign_toplevel_state(&mut self) -> &mut ForeignToplevelManagementState;

    /// Current protocol-visible presentation of a managed window, or `None`
    /// when the window is not (yet) part of the managed model.
    fn foreign_toplevel_snapshot(&self, window: WindowId) -> Option<ToplevelSnapshot>;

    /// A controlling client issued a request for this window.
    fn foreign_toplevel_request(&mut self, window: WindowId, request: ForeignToplevelRequest);
}

// ---------------------------------------------------------------------------
// ForeignToplevelManagementState implementation
// ---------------------------------------------------------------------------

impl ForeignToplevelManagementState {
    /// Create the state and register the global (version 3: full control +
    /// fullscreen requests).
    pub fn new<D>(dh: &DisplayHandle) -> Self
    where
        D: GlobalDispatch<ZwlrForeignToplevelManagerV1, ForeignToplevelGlobalData>
            + Dispatch<ZwlrForeignToplevelManagerV1, ()>
            + Dispatch<ZwlrForeignToplevelHandleV1, WindowId>
            + ForeignToplevelHandler
            + 'static,
    {
        let global = dh.create_global::<D, ZwlrForeignToplevelManagerV1, _>(
            3, // max version we support (fullscreen requests + parent event)
            ForeignToplevelGlobalData {
                _filter: Box::new(|_| true),
            },
        );

        ForeignToplevelManagementState {
            global,
            dh: dh.clone(),
            instances: Vec::new(),
            windows: Vec::new(),
        }
    }

    /// Advertise or refresh a window on every manager instance.
    ///
    /// Idempotent: an unknown window is added, a known one is diffed so only
    /// changed properties hit the wire. Call freely after any change that can
    /// affect the snapshot.
    pub fn sync_toplevel<D>(&mut self, window: WindowId, snapshot: &ToplevelSnapshot)
    where
        D: GlobalDispatch<ZwlrForeignToplevelManagerV1, ForeignToplevelGlobalData>
            + Dispatch<ZwlrForeignToplevelManagerV1, ()>
            + Dispatch<ZwlrForeignToplevelHandleV1, WindowId>
            + ForeignToplevelHandler
            + 'static,
    {
        if !self.windows.contains(&window) {
            self.windows.push(window);
        }

        for instance in &mut self.instances {
            if instance.suppressed.contains(&window) {
                continue;
            }
            let parent = parent_handle(instance, snapshot.parent);
            match instance
                .toplevels
                .iter_mut()
                .find(|t| t.window == window && !t.closed)
            {
                Some(entry) => update_entry(&self.dh, entry, snapshot, parent.as_ref()),
                None => create_entry::<D>(&self.dh, instance, window, snapshot),
            }
        }
    }

    /// Stop advertising a window (it was unmanged/closed). Sends `closed`.
    pub fn remove_toplevel<D>(&mut self, window: WindowId)
    where
        D: GlobalDispatch<ZwlrForeignToplevelManagerV1, ForeignToplevelGlobalData>
            + Dispatch<ZwlrForeignToplevelManagerV1, ()>
            + Dispatch<ZwlrForeignToplevelHandleV1, WindowId>
            + ForeignToplevelHandler
            + 'static,
    {
        self.windows.retain(|&w| w != window);
        for instance in &mut self.instances {
            instance.suppressed.retain(|&w| w != window);
            for entry in &mut instance.toplevels {
                if entry.window == window && !entry.closed {
                    entry.obj.closed();
                    entry.closed = true;
                }
            }
        }
    }
}

/// Create a handle resource on one instance and send its initial state.
fn create_entry<D>(
    dh: &DisplayHandle,
    instance: &mut ManagerInstance,
    window: WindowId,
    snapshot: &ToplevelSnapshot,
) where
    D: Dispatch<ZwlrForeignToplevelHandleV1, WindowId> + 'static,
{
    let Ok(client) = dh.get_client(instance.obj.id()) else {
        return;
    };
    let Ok(obj) = client.create_resource::<ZwlrForeignToplevelHandleV1, _, D>(
        dh,
        instance.obj.version(),
        window,
    ) else {
        return;
    };

    instance.obj.toplevel(&obj);
    let parent = parent_handle(instance, snapshot.parent);
    let mut entry = ToplevelInstance {
        obj,
        window,
        title: None,
        app_id: None,
        states_mask: None,
        parent: None,
        outputs: Vec::new(),
        closed: false,
    };
    update_entry(dh, &mut entry, snapshot, parent.as_ref());
    instance.toplevels.push(entry);
}

fn parent_handle(
    instance: &ManagerInstance,
    parent: Option<WindowId>,
) -> Option<ZwlrForeignToplevelHandleV1> {
    let parent = parent?;
    instance
        .toplevels
        .iter()
        .find(|entry| entry.window == parent && !entry.closed)
        .map(|entry| entry.obj.clone())
}

/// Diff `snapshot` against what was last sent and emit only changed events.
///
/// Always ends with `done` when anything changed, per protocol contract.
fn update_entry(
    dh: &DisplayHandle,
    entry: &mut ToplevelInstance,
    snapshot: &ToplevelSnapshot,
    parent: Option<&ZwlrForeignToplevelHandleV1>,
) {
    let mut changed = false;

    if entry.title.as_ref() != Some(&snapshot.title) {
        entry.obj.title(snapshot.title.clone());
        entry.title = Some(snapshot.title.clone());
        changed = true;
    }
    if entry.app_id.as_ref() != Some(&snapshot.app_id) {
        entry.obj.app_id(snapshot.app_id.clone());
        entry.app_id = Some(snapshot.app_id.clone());
        changed = true;
    }

    let mut mask = 0u32;
    if snapshot.maximized {
        mask |= ToplevelInstance::MAXIMIZED;
    }
    if snapshot.minimized {
        mask |= ToplevelInstance::MINIMIZED;
    }
    if snapshot.activated {
        mask |= ToplevelInstance::ACTIVATED;
    }
    // The fullscreen state (enum value 3) only exists from protocol
    // version 2; sending it to a version-1 handle would desynchronize the
    // client's wl_array parsing.
    let fullscreen_advertisable = entry.obj.version() >= 2;
    if snapshot.fullscreen && fullscreen_advertisable {
        mask |= ToplevelInstance::FULLSCREEN;
    }
    if entry.states_mask != Some(mask) {
        let mut states = Vec::new();
        // Wire order matches the spec enum: maximized=0, minimized=1,
        // activated=2, fullscreen=3. wl_array contents are host-endian.
        if snapshot.maximized {
            states.extend(0u32.to_ne_bytes());
        }
        if snapshot.minimized {
            states.extend(1u32.to_ne_bytes());
        }
        if snapshot.activated {
            states.extend(2u32.to_ne_bytes());
        }
        if snapshot.fullscreen && fullscreen_advertisable {
            states.extend(3u32.to_ne_bytes());
        }
        entry.obj.state(states);
        entry.states_mask = Some(mask);
        changed = true;
    }

    if entry.obj.version() >= 3 {
        // A client that destroyed the parent's handle must not receive a
        // parent update solely because that resource disappeared. Defer the
        // event until either the relationship is cleared or a live handle is
        // available for the new parent.
        let advertised_parent = match (snapshot.parent, parent) {
            (None, _) => Some(None),
            (Some(parent), Some(_)) => Some(Some(parent)),
            (Some(_), None) => None,
        };
        if let Some(advertised_parent) = advertised_parent
            && entry.parent != Some(advertised_parent)
        {
            entry.obj.parent(parent);
            entry.parent = Some(advertised_parent);
            changed = true;
        }
    }

    // Resolve membership against the client's *current* wl_output bindings
    // and diff against what was actually announced, so a late-bound
    // wl_output still receives its output_enter on the next refresh. A
    // client that has not bound an output simply sees no enter/leave.
    let client = dh.get_client(entry.obj.id()).ok();
    let new_wl_outputs: Vec<WlOutput> = match client.as_ref() {
        Some(client) => snapshot
            .outputs
            .iter()
            .flat_map(|output| output.client_outputs(client))
            .collect(),
        None => Vec::new(),
    };

    for wl_output in &new_wl_outputs {
        if !entry.outputs.contains(wl_output) {
            entry.obj.output_enter(wl_output);
        }
    }
    for wl_output in &entry.outputs {
        // A wl_output the client destroyed cannot receive events anymore.
        if wl_output.is_alive() && !new_wl_outputs.contains(wl_output) {
            entry.obj.output_leave(wl_output);
        }
    }
    if entry.outputs != new_wl_outputs {
        entry.outputs = new_wl_outputs;
        changed = true;
    }

    if changed {
        entry.obj.done();
    }
}

// ---------------------------------------------------------------------------
// GlobalDispatch / Dispatch implementations
// ---------------------------------------------------------------------------

impl<D> GlobalDispatch<ZwlrForeignToplevelManagerV1, ForeignToplevelGlobalData, D>
    for ForeignToplevelManagementState
where
    D: GlobalDispatch<ZwlrForeignToplevelManagerV1, ForeignToplevelGlobalData>
        + Dispatch<ZwlrForeignToplevelManagerV1, ()>
        + Dispatch<ZwlrForeignToplevelHandleV1, WindowId>
        + ForeignToplevelHandler
        + 'static,
{
    fn bind(
        state: &mut D,
        dh: &DisplayHandle,
        _client: &Client,
        resource: New<ZwlrForeignToplevelManagerV1>,
        _global_data: &ForeignToplevelGlobalData,
        data_init: &mut DataInit<'_, D>,
    ) {
        let mut instance = ManagerInstance {
            obj: data_init.init(resource, ()),
            toplevels: Vec::new(),
            suppressed: Vec::new(),
        };

        // Late-binding clients (panels started after the compositor) get the
        // full current inventory immediately.
        let windows = state.foreign_toplevel_state().windows.clone();
        for window in windows {
            if let Some(snapshot) = state.foreign_toplevel_snapshot(window) {
                create_entry::<D>(dh, &mut instance, window, &snapshot);
            }
        }

        state.foreign_toplevel_state().instances.push(instance);
    }

    fn can_view(client: Client, global_data: &ForeignToplevelGlobalData) -> bool {
        (global_data._filter)(&client)
    }
}

impl<D> Dispatch<ZwlrForeignToplevelManagerV1, (), D> for ForeignToplevelManagementState
where
    D: GlobalDispatch<ZwlrForeignToplevelManagerV1, ForeignToplevelGlobalData>
        + Dispatch<ZwlrForeignToplevelManagerV1, ()>
        + Dispatch<ZwlrForeignToplevelHandleV1, WindowId>
        + ForeignToplevelHandler
        + 'static,
{
    fn request(
        state: &mut D,
        _client: &Client,
        obj: &ZwlrForeignToplevelManagerV1,
        request: zwlr_foreign_toplevel_manager_v1::Request,
        _data: &(),
        _dh: &DisplayHandle,
        _data_init: &mut DataInit<'_, D>,
    ) {
        if let zwlr_foreign_toplevel_manager_v1::Request::Stop = request {
            let ft_state = state.foreign_toplevel_state();
            ft_state.instances.retain(|i| i.obj != *obj);
            obj.finished();
        }
    }

    fn destroyed(state: &mut D, _client: ClientId, obj: &ZwlrForeignToplevelManagerV1, _data: &()) {
        let ft_state = state.foreign_toplevel_state();
        ft_state.instances.retain(|i| i.obj != *obj);
    }
}

impl<D> Dispatch<ZwlrForeignToplevelHandleV1, WindowId, D> for ForeignToplevelManagementState
where
    D: GlobalDispatch<ZwlrForeignToplevelManagerV1, ForeignToplevelGlobalData>
        + Dispatch<ZwlrForeignToplevelManagerV1, ()>
        + Dispatch<ZwlrForeignToplevelHandleV1, WindowId>
        + ForeignToplevelHandler
        + 'static,
{
    fn request(
        state: &mut D,
        _client: &Client,
        obj: &ZwlrForeignToplevelHandleV1,
        request: zwlr_foreign_toplevel_handle_v1::Request,
        data: &WindowId,
        _dh: &DisplayHandle,
        _data_init: &mut DataInit<'_, D>,
    ) {
        use zwlr_foreign_toplevel_handle_v1::Request;
        let window = *data;
        if matches!(request, Request::Destroy) {
            let ft_state = state.foreign_toplevel_state();
            forget_handle(ft_state, obj, window);
            return;
        }

        // A handle becomes inert after `closed`; all requests except the
        // destructor must then be ignored. It can also be absent because its
        // manager was stopped or destroyed.
        let active = state
            .foreign_toplevel_state()
            .instances
            .iter()
            .any(|instance| {
                instance
                    .toplevels
                    .iter()
                    .any(|entry| entry.obj == *obj && !entry.closed)
            });
        if !active {
            return;
        }

        match request {
            Request::Activate { .. } => {
                state.foreign_toplevel_request(window, ForeignToplevelRequest::Activate)
            }
            Request::Close => state.foreign_toplevel_request(window, ForeignToplevelRequest::Close),
            Request::SetMaximized => {
                state.foreign_toplevel_request(window, ForeignToplevelRequest::SetMaximized(true))
            }
            Request::UnsetMaximized => {
                state.foreign_toplevel_request(window, ForeignToplevelRequest::SetMaximized(false))
            }
            Request::SetMinimized => {
                state.foreign_toplevel_request(window, ForeignToplevelRequest::SetMinimized(true))
            }
            Request::UnsetMinimized => {
                state.foreign_toplevel_request(window, ForeignToplevelRequest::SetMinimized(false))
            }
            Request::SetFullscreen { .. } => {
                state.foreign_toplevel_request(window, ForeignToplevelRequest::SetFullscreen(true))
            }
            Request::UnsetFullscreen => {
                state.foreign_toplevel_request(window, ForeignToplevelRequest::SetFullscreen(false))
            }
            // Geometry hint for click-to-raise heuristics; instantWM resolves
            // focus itself, so the rectangle is accepted but unused.
            Request::SetRectangle { .. } => {}
            _ => {}
        }
    }

    fn destroyed(
        state: &mut D,
        _client: ClientId,
        obj: &ZwlrForeignToplevelHandleV1,
        data: &WindowId,
    ) {
        let ft_state = state.foreign_toplevel_state();
        forget_handle(ft_state, obj, *data);
    }
}

/// Drop a handle the client destroyed and remember the opt-out.
///
/// Without the record, the next [`ForeignToplevelManagementState::sync_toplevel`]
/// would treat the still-managed window as unknown and re-advertise it with a
/// brand-new handle — violating the event's "new toplevel is created"
/// semantics and resurrecting taskbar entries the client deliberately closed.
/// Recording happens on both the explicit `destroy` request and server-side
/// resource destruction; the latter also covers clients killed mid-request.
fn forget_handle(
    state: &mut ForeignToplevelManagementState,
    obj: &ZwlrForeignToplevelHandleV1,
    window: WindowId,
) {
    let window_is_managed = state.windows.contains(&window);
    for instance in &mut state.instances {
        let was_active = instance
            .toplevels
            .iter()
            .any(|entry| entry.obj == *obj && !entry.closed);
        if instance.toplevels.iter().any(|entry| entry.obj == *obj) {
            instance.toplevels.retain(|entry| entry.obj != *obj);
            if was_active && window_is_managed && !instance.suppressed.contains(&window) {
                instance.suppressed.push(window);
            }
        }
    }
}

// ---------------------------------------------------------------------------
// Delegate macro (mirrors delegate_output_management!)
// ---------------------------------------------------------------------------

// `#[macro_export]` puts the macro at the crate root so the absolute paths
// below resolve no matter where the macro is invoked from. This matches the
// `delegate_output_management!` pattern.

#[macro_export]
macro_rules! delegate_foreign_toplevel_management {
    ($(@< $( $lt:tt $( : $clt:tt $(+ $dlt:tt )* )? ),+ >)? $ty: ty) => {
        smithay::reexports::wayland_server::delegate_global_dispatch!($(@< $( $lt $( : $clt $(+ $dlt )* )? ),+ >)? $ty: [
            smithay::reexports::wayland_protocols_wlr::foreign_toplevel::v1::server::zwlr_foreign_toplevel_manager_v1::ZwlrForeignToplevelManagerV1: $crate::backend::wayland::compositor::protocols::foreign_toplevel::ForeignToplevelGlobalData
        ] => $crate::backend::wayland::compositor::protocols::foreign_toplevel::ForeignToplevelManagementState);
        smithay::reexports::wayland_server::delegate_dispatch!($(@< $( $lt $( : $clt $(+ $dlt )* )? ),+ >)? $ty: [
            smithay::reexports::wayland_protocols_wlr::foreign_toplevel::v1::server::zwlr_foreign_toplevel_manager_v1::ZwlrForeignToplevelManagerV1: ()
        ] => $crate::backend::wayland::compositor::protocols::foreign_toplevel::ForeignToplevelManagementState);
        smithay::reexports::wayland_server::delegate_dispatch!($(@< $( $lt $( : $clt $(+ $dlt )* )? ),+ >)? $ty: [
            smithay::reexports::wayland_protocols_wlr::foreign_toplevel::v1::server::zwlr_foreign_toplevel_handle_v1::ZwlrForeignToplevelHandleV1: $crate::types::WindowId
        ] => $crate::backend::wayland::compositor::protocols::foreign_toplevel::ForeignToplevelManagementState);
    };
}
