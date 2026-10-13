use super::*;

pub fn build_output_surfaces(
    output_manager: &mut ManagedDrmOutputManager,
    device: DrmDeviceId,
    renderer: &mut GlesRenderer,
    state: &mut WaylandNativeState,
) -> Vec<OutputSurfaceEntry> {
    let mut output_surfaces: Vec<OutputSurfaceEntry> = Vec::new();
    match probe_usable_connectors(output_manager) {
        Ok(connectors) => {
            reconcile_output_surfaces(
                output_manager,
                renderer,
                state,
                &mut output_surfaces,
                OutputTopology {
                    device,
                    connectors: &connectors,
                    in_flight: &[],
                    occupied_names: &[],
                },
            );
        }
        Err(error) => log::warn!("Initial DRM connector probe failed: {error}"),
    }
    output_surfaces
}

/// Runtime bookkeeping needed after physical output reconciliation.
#[derive(Default)]
pub struct OutputReconciliation {
    pub moved: Vec<(crtc::Handle, crtc::Handle)>,
    pub deferred: bool,
}

/// A single device's connector snapshot and constraints for reconciliation.
pub struct OutputTopology<'a> {
    pub device: DrmDeviceId,
    pub connectors: &'a [connector::Info],
    pub in_flight: &'a [crtc::Handle],
    pub occupied_names: &'a [String],
}

/// Discover heads, preserving assignments unless moving an existing head is
/// necessary to admit another. Retire in-flight flips before moving surfaces.
pub fn reconcile_output_surfaces(
    output_manager: &mut ManagedDrmOutputManager,
    renderer: &mut GlesRenderer,
    state: &mut WaylandNativeState,
    output_surfaces: &mut Vec<OutputSurfaceEntry>,
    topology: OutputTopology<'_>,
) -> OutputReconciliation {
    let OutputTopology {
        device,
        connectors,
        in_flight,
        occupied_names,
    } = topology;
    let mut output_x_offset = output_surfaces
        .iter()
        .filter(|entry| state.space.output_geometry(&entry.output).is_some())
        .map(|entry| entry.rect.x.saturating_add(entry.rect.w))
        .max()
        .unwrap_or(0);

    let res = match output_manager.device().resource_handles() {
        Ok(resources) => resources,
        Err(error) => {
            log::warn!("DRM output discovery failed: {error}");
            return OutputReconciliation::default();
        }
    };
    let existing_connectors: Vec<connector::Handle> = output_surfaces
        .iter()
        .map(|entry| entry.connector)
        .collect();
    let init_render_elements = DrmOutputRenderElements::<GlesRenderer, DrmOutputElement>::default();

    // Build every candidate from the same connector snapshot used for removal
    // and capability refresh. Incomplete encoder reads invalidate the plan.
    let probed = connectors
        .iter()
        .map(|info| drm_output_candidate(output_manager, &res, info))
        .collect::<std::io::Result<Vec<_>>>();
    let probed = match probed {
        Ok(probed) => probed.into_iter().flatten().collect::<Vec<_>>(),
        Err(error) => {
            log::warn!("DRM encoder discovery failed: {error}");
            return OutputReconciliation::default();
        }
    };
    let mut candidates = Vec::new();
    let mut preferred = Vec::new();
    for entry in output_surfaces.iter() {
        // Retain a represented head if the kernel temporarily exposes no
        // compatible encoder, rather than stealing its current controller.
        let compatible = probed
            .iter()
            .find(|candidate| candidate.connector == entry.connector)
            .map(|candidate| candidate.crtcs.clone())
            .unwrap_or_else(|| vec![entry.crtc]);
        candidates.push(compatible);
        preferred.push(Some(entry.crtc));
    }
    let pending: Vec<_> = probed
        .into_iter()
        .filter(|candidate| !existing_connectors.contains(&candidate.connector))
        .collect();
    candidates.extend(pending.iter().map(|candidate| candidate.crtcs.clone()));
    preferred.resize(candidates.len(), None);
    let assignments = crate::backend::output::assign_output_resources(&candidates, &preferred);
    if assignments[..output_surfaces.len()]
        .iter()
        .any(Option::is_none)
    {
        log::warn!(
            "DRM topology cannot retain all existing heads; keeping the current assignments"
        );
        return OutputReconciliation::default();
    }
    if output_surfaces
        .iter()
        .zip(&assignments)
        .any(|(entry, assigned)| *assigned != Some(entry.crtc) && in_flight.contains(&entry.crtc))
    {
        // Retire the real page flip before replacing a live surface. Otherwise
        // its delayed event could be mistaken for the new surface's first flip.
        return OutputReconciliation {
            moved: Vec::new(),
            deferred: true,
        };
    }
    let mut moved = Vec::new();
    // Release all moving surfaces before any new controller is initialized.
    for (entry, assignment) in output_surfaces.iter_mut().zip(&assignments) {
        if let Some(crtc) = assignment
            && entry.crtc != *crtc
        {
            log::info!(
                "Output {}: moving CRTC {:?} -> {:?}",
                entry.output.name(),
                entry.crtc,
                crtc
            );
            moved.push((entry.crtc, *crtc));
            entry.surface.take();
            entry.dmabuf_feedback = None;
            entry.crtc = *crtc;
        }
    }
    let migrations_ok = output_surfaces
        .iter_mut()
        .filter(|entry| moved.iter().any(|(_, new)| *new == entry.crtc))
        .all(|entry| {
            restore_output_surface(output_manager, renderer, entry, &init_render_elements)
        });
    if !migrations_ok {
        // A mathematical matching is not a KMS commit guarantee. Restore every
        // old assignment before admitting new heads if hardware rejects a move.
        for entry in output_surfaces.iter_mut() {
            if moved.iter().any(|(_, new)| *new == entry.crtc) {
                entry.surface.take();
            }
        }
        for entry in output_surfaces.iter_mut() {
            if let Some((old, _)) = moved.iter().find(|(_, new)| *new == entry.crtc) {
                entry.crtc = *old;
            }
        }
        for entry in output_surfaces.iter_mut() {
            if moved.iter().any(|(old, _)| *old == entry.crtc)
                && !restore_output_surface(output_manager, renderer, entry, &init_render_elements)
            {
                log::error!(
                    "Output {}: failed to restore controller after rejected migration",
                    entry.output.name()
                );
            }
        }
        // Even rollback recreates surfaces: retire the old runtime flip state.
        return OutputReconciliation {
            moved: moved.into_iter().map(|(old, _)| (old, old)).collect(),
            deferred: false,
        };
    }
    let existing_count = output_surfaces.len();
    for (candidate, assignment) in pending
        .into_iter()
        .zip(assignments.into_iter().skip(existing_count))
    {
        let Some(crtc) = assignment else {
            log::warn!(
                "Output {}: no available CRTC; compatible controllers: {:?}",
                candidate.name,
                candidate.crtcs
            );
            continue;
        };
        let mut spec = candidate.assign(crtc);
        // Preserve normal connector names, disambiguating only when another
        // device already advertises the same name (as niri does).
        let base = spec.name.clone();
        let mut suffix = 2;
        while occupied_names.contains(&spec.name)
            || output_surfaces
                .iter()
                .any(|entry| entry.output.name() == spec.name)
        {
            spec.name = format!("{base}-{suffix}");
            suffix += 1;
        }
        let Some(entry) = initialize_drm_output_surface(
            output_manager,
            device,
            renderer,
            state,
            &init_render_elements,
            spec,
            output_x_offset,
        ) else {
            continue;
        };
        output_x_offset += entry.rect.w;
        output_surfaces.push(entry);
    }
    OutputReconciliation {
        moved,
        deferred: false,
    }
}

/// Probe a complete connector snapshot. A failed read invalidates the scan;
/// it is not evidence that an existing display disconnected.
pub fn probe_usable_connectors(
    output_manager: &ManagedDrmOutputManager,
) -> std::io::Result<Vec<connector::Info>> {
    let resources = output_manager.device().resource_handles()?;
    let mut usable = Vec::new();
    for &connector in resources.connectors() {
        // Startup and hotplug must refresh status, modes and EDID. Cached
        // connector information can retain the old dock's connected outputs
        // and reserve every CRTC before its replacement can be initialized.
        // This scan is event-driven: do not force-probe from the render loop.
        let info = match output_manager.device().get_connector(connector, true) {
            Ok(info) => info,
            // MST connectors can disappear between enumerating resources and
            // querying them. ENOENT is an actual removal, unlike other probe
            // failures, which must leave the previous topology intact.
            Err(error) if error.raw_os_error() == Some(libc::ENOENT) => continue,
            Err(error) => return Err(error),
        };
        if is_usable_connector(&info) {
            usable.push(info);
        }
    }
    Ok(usable)
}

/// Refresh capabilities even when the connector handle has not changed.
/// The policy transaction selects a supported replacement for a vanished mode;
/// the physical modeset still goes through the normal configuration path.
pub fn refresh_output_modes(entry: &mut OutputSurfaceEntry, info: &connector::Info) -> bool {
    let modes: Vec<_> = info
        .modes()
        .iter()
        .copied()
        .map(|mode| (OutputMode::from(mode), mode))
        .collect();
    if entry.modes == modes {
        return false;
    }
    log::info!(
        "Output {}: mode list changed ({} -> {} modes)",
        entry.output.name(),
        entry.modes.len(),
        modes.len()
    );
    entry.modes = modes;
    true
}

fn restore_output_surface(
    manager: &mut ManagedDrmOutputManager,
    renderer: &mut GlesRenderer,
    entry: &mut OutputSurfaceEntry,
    elements: &DrmOutputRenderElements<GlesRenderer, DrmOutputElement>,
) -> bool {
    if !entry.enabled {
        return true;
    }
    let Some(mode) = entry
        .modes
        .iter()
        .find_map(|(mode, drm)| (Some(*mode) == entry.output.current_mode()).then_some(*drm))
    else {
        return false;
    };
    let initialized = manager.lock().initialize_output(
        entry.crtc,
        mode,
        &[entry.connector],
        &entry.output,
        None,
        renderer,
        elements,
    );
    match initialized {
        Ok(surface) => {
            entry.dmabuf_feedback = build_output_dmabuf_feedback(renderer, &surface);
            if let Err(error) =
                surface.with_compositor(|compositor| compositor.use_vrr(entry.vrr_enabled))
            {
                log::warn!(
                    "Output {}: failed to restore adaptive sync: {error}",
                    entry.output.name()
                );
                return false;
            }
            if !entry.powered
                && let Err(error) = surface.with_compositor(|compositor| compositor.clear())
            {
                log::warn!(
                    "Output {}: failed to restore powered-off state: {error}",
                    entry.output.name()
                );
                return false;
            }
            entry.surface = Some(surface);
            true
        }
        Err(error) => {
            log::warn!(
                "Output {}: CRTC {:?} initialization failed: {error:?}",
                entry.output.name(),
                entry.crtc
            );
            false
        }
    }
}

struct DrmOutputSpec {
    connector: connector::Handle,
    internal: bool,
    crtc: crtc::Handle,
    mode: control::Mode,
    modes: Vec<control::Mode>,
    pixel_size: crate::types::Size,
    physical_size: crate::types::Size,
    name: String,
}

struct DrmOutputCandidate {
    connector: connector::Handle,
    internal: bool,
    crtcs: Vec<crtc::Handle>,
    mode: control::Mode,
    modes: Vec<control::Mode>,
    pixel_size: crate::types::Size,
    physical_size: crate::types::Size,
    name: String,
}

impl DrmOutputCandidate {
    fn assign(self, crtc: crtc::Handle) -> DrmOutputSpec {
        DrmOutputSpec {
            connector: self.connector,
            internal: self.internal,
            crtc,
            mode: self.mode,
            modes: self.modes,
            pixel_size: self.pixel_size,
            physical_size: self.physical_size,
            name: self.name,
        }
    }
}

fn drm_output_candidate(
    output_manager: &ManagedDrmOutputManager,
    resources: &control::ResourceHandles,
    conn_info: &connector::Info,
) -> std::io::Result<Option<DrmOutputCandidate>> {
    let connector = conn_info.handle();
    let Some(mode) = best_connector_mode(conn_info.modes()) else {
        return Ok(None);
    };
    // Use DRM resource order, never randomized HashSet iteration.
    let encoders: Vec<_> = conn_info
        .encoders()
        .iter()
        .map(|&encoder| output_manager.device().get_encoder(encoder))
        .collect::<std::io::Result<Vec<_>>>()?;
    let crtcs: Vec<_> = resources
        .crtcs()
        .iter()
        .copied()
        .filter(|crtc| {
            encoders.iter().any(|encoder| {
                resources
                    .filter_crtcs(encoder.possible_crtcs())
                    .contains(crtc)
            })
        })
        .collect();
    if crtcs.is_empty() {
        log::warn!(
            "Connector {:?}: no compatible display controllers",
            connector
        );
        return Ok(None);
    }
    let (width, height) = mode.size();
    let physical_size = conn_info.size().unwrap_or((0, 0));

    Ok(Some(DrmOutputCandidate {
        connector,
        crtcs,
        mode,
        modes: conn_info.modes().to_vec(),
        pixel_size: crate::types::Size::new(width as i32, height as i32),
        physical_size: crate::types::Size::new(physical_size.0 as i32, physical_size.1 as i32),
        name: format!(
            "{}-{}",
            connector_type_name(conn_info.interface()),
            conn_info.interface_id()
        ),
        internal: matches!(
            conn_info.interface(),
            connector::Interface::EmbeddedDisplayPort
                | connector::Interface::LVDS
                | connector::Interface::DSI
        ),
    }))
}

fn is_usable_connector(conn_info: &connector::Info) -> bool {
    // `Unknown` is not evidence of a live sink. Keeping such a connector
    // active reserves its CRTC and can prevent a newly enumerated MST head
    // from being assigned. The hotplug settling probes will add it once KMS
    // reports the authoritative Connected state.
    conn_info.state() == connector::State::Connected && !conn_info.modes().is_empty()
}

pub(crate) fn best_connector_mode(modes: &[control::Mode]) -> Option<control::Mode> {
    modes.iter().copied().max_by(|a, b| {
        let (aw, ah) = a.size();
        let (bw, bh) = b.size();
        a.mode_type()
            .contains(control::ModeTypeFlags::PREFERRED)
            .cmp(&b.mode_type().contains(control::ModeTypeFlags::PREFERRED))
            .then_with(|| (aw as u64 * ah as u64).cmp(&(bw as u64 * bh as u64)))
            .then_with(|| a.vrefresh().cmp(&b.vrefresh()))
    })
}

fn initialize_drm_output_surface(
    output_manager: &mut ManagedDrmOutputManager,
    device: DrmDeviceId,
    renderer: &mut GlesRenderer,
    state: &mut WaylandNativeState,
    init_render_elements: &DrmOutputRenderElements<GlesRenderer, DrmOutputElement>,
    spec: DrmOutputSpec,
    x_offset: i32,
) -> Option<OutputSurfaceEntry> {
    log::info!(
        "Output {}: {}x{}@{}Hz on CRTC {:?}",
        spec.name,
        spec.pixel_size.w,
        spec.pixel_size.h,
        spec.mode.vrefresh(),
        spec.crtc
    );

    let output = create_drm_wayland_output(state, &spec, x_offset);
    let surface = match output_manager.lock().initialize_output(
        spec.crtc,
        spec.mode,
        &[spec.connector],
        &output,
        None,
        renderer,
        init_render_elements,
    ) {
        Ok(surface) => surface,
        Err(error) => {
            log::warn!("Output {}: failed to initialize: {error:?}", spec.name);
            state.set_output_global_enabled(&output, false);
            return None;
        }
    };
    let (vrr_support, configured_vrr_mode) =
        configure_drm_output_vrr(state, &spec.name, spec.connector, &surface);
    let dmabuf_feedback = build_output_dmabuf_feedback(renderer, &surface);
    if dmabuf_feedback.is_none() {
        log::warn!(
            "Output {}: could not build DMA-BUF scanout feedback; clients will receive render-only feedback",
            spec.name
        );
    }
    state.runtime.output_power_modes.insert(
        spec.name.clone(),
        crate::backend::output::OutputPowerMode::On,
    );

    state
        .runtime
        .lid_output_policy
        .set_internal(spec.name.clone().into(), spec.internal);
    log::info!(
        "Output {} created: device={device:?}, internal={}, enabled=true",
        spec.name,
        spec.internal
    );

    Some(OutputSurfaceEntry {
        device,
        crtc: spec.crtc,
        surface: Some(surface),
        connector: spec.connector,
        modes: spec
            .modes
            .iter()
            .copied()
            .map(|mode| (OutputMode::from(mode), mode))
            .collect(),
        output: output.clone(),
        dmabuf_feedback,
        rect: crate::types::Rect::from_position_and_size(
            crate::types::Point::new(x_offset, 0),
            spec.pixel_size,
        ),
        vrr_support,
        configured_vrr_mode,
        vrr_enabled: false,
        enabled: true,
        powered: true,
        pending_power_on: None,
    })
}

pub(crate) fn build_output_dmabuf_feedback(
    renderer: &GlesRenderer,
    output: &ManagedDrmOutput,
) -> Option<OutputDmabufFeedback> {
    use smithay::backend::allocator::format::FormatSet;
    use smithay::backend::egl::EGLDevice;

    let render_node = EGLDevice::device_for_display(renderer.egl_context().display())
        .ok()?
        .try_get_render_node()
        .ok()??;
    let render_formats = renderer.dmabuf_formats();
    let scanout_formats: FormatSet = output.with_compositor(|compositor| {
        compositor
            .surface()
            .plane_info()
            .formats
            .intersection(&render_formats)
            .copied()
            .collect()
    });
    let scanout_device = output
        .with_compositor(|compositor| compositor.surface().device_fd().dev_id())
        .ok()?;

    let builder = DmabufFeedbackBuilder::new(render_node.dev_id(), render_formats.clone());
    let render = builder.clone().build().ok()?;
    let scanout = builder
        .add_preference_tranche(
            scanout_device,
            zwp_linux_dmabuf_feedback_v1::TrancheFlags::Scanout,
            scanout_formats,
            4u32..=6,
        )
        .build()
        .ok()?;
    Some(OutputDmabufFeedback { render, scanout })
}

fn create_drm_wayland_output(
    state: &WaylandNativeState,
    spec: &DrmOutputSpec,
    x_offset: i32,
) -> Output {
    let out_mode = OutputMode::from(spec.mode);
    let output = state.create_output_global(
        spec.name.clone(),
        PhysicalProperties {
            size: (spec.physical_size.w, spec.physical_size.h).into(),
            subpixel: Subpixel::Unknown,
            make: "instantOS".into(),
            model: "instantWM".into(),
            serial_number: "Unknown".into(),
        },
        out_mode,
        crate::types::Point::new(x_offset, 0),
    );
    for mode in &spec.modes {
        output.add_mode(OutputMode::from(*mode));
    }
    output
}

fn configure_drm_output_vrr(
    state: &mut WaylandNativeState,
    output_name: &str,
    connector: connector::Handle,
    surface: &state::ManagedDrmOutput,
) -> (BackendVrrSupport, VrrMode) {
    let vrr_support = drm_surface_vrr_support(surface, connector);
    state.set_output_vrr_support(output_name, vrr_support);
    let configured_vrr_mode = state
        .output_vrr_metadata(output_name)
        .map(|m| m.vrr_mode)
        .unwrap_or_default();
    state.set_output_vrr_mode(output_name, configured_vrr_mode);
    state.set_output_vrr_enabled(output_name, false);
    log::info!("Output {output_name}: VRR support = {:?}", vrr_support);
    (vrr_support, configured_vrr_mode)
}

fn drm_surface_vrr_support(
    surface: &state::ManagedDrmOutput,
    connector: connector::Handle,
) -> BackendVrrSupport {
    match surface.with_compositor(|compositor| compositor.vrr_supported(connector)) {
        Ok(VrrSupport::Supported) => BackendVrrSupport::Supported,
        Ok(VrrSupport::RequiresModeset) => BackendVrrSupport::RequiresModeset,
        Ok(VrrSupport::NotSupported) | Err(_) => BackendVrrSupport::Unsupported,
    }
}

pub fn create_output_manager(
    drm_device: DrmDevice,
    renderer: &GlesRenderer,
    allocator_device: &GbmDevice<DrmDeviceFd>,
    scanout_device: &GbmDevice<DrmDeviceFd>,
) -> ManagedDrmOutputManager {
    let cross_device = allocator_device.dev_id().ok() != Some(drm_device.device_id());
    let allocator = GbmAllocator::new(
        allocator_device.clone(),
        GbmBufferFlags::RENDERING | GbmBufferFlags::SCANOUT,
    );
    let exporter = GbmFramebufferExporter::new(scanout_device.clone(), NodeFilter::None);
    let color_formats: [Fourcc; 2] = [Fourcc::Argb8888, Fourcc::Xrgb8888];
    // Display-only devices cannot consume GPU-specific tiling/compression.
    // Like niri, render on the primary GPU into linear buffers and let
    // Smithay's exporter import their DMA-BUFs into the scanout device.
    let renderer_formats: Vec<_> = if cross_device {
        renderer
            .egl_context()
            .dmabuf_render_formats()
            .iter()
            .copied()
            .filter(|format| format.modifier == smithay::backend::allocator::Modifier::Linear)
            .collect()
    } else {
        renderer.dmabuf_formats().into_iter().collect()
    };

    ManagedDrmOutputManager::new(
        drm_device,
        allocator,
        exporter,
        Some(scanout_device.clone()),
        color_formats,
        renderer_formats,
    )
}

fn connector_type_name(interface: connector::Interface) -> &'static str {
    match interface {
        connector::Interface::DVII => "DVI-I",
        connector::Interface::DVID => "DVI-D",
        connector::Interface::DVIA => "DVI-A",
        connector::Interface::SVideo => "S-Video",
        connector::Interface::DisplayPort => "DP",
        connector::Interface::HDMIA => "HDMI-A",
        connector::Interface::HDMIB => "HDMI-B",
        connector::Interface::EmbeddedDisplayPort => "eDP",
        connector::Interface::VGA => "VGA",
        connector::Interface::LVDS => "LVDS",
        connector::Interface::DSI => "DSI",
        connector::Interface::DPI => "DPI",
        connector::Interface::Composite => "Composite",
        _ => "Unknown",
    }
}
