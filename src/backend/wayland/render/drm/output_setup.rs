use super::*;

pub fn build_output_surfaces(
    output_manager: &mut ManagedDrmOutputManager,
    renderer: &mut GlesRenderer,
    state: &mut WaylandState,
) -> Vec<OutputSurfaceEntry> {
    let mut output_surfaces: Vec<OutputSurfaceEntry> = Vec::new();
    add_new_output_surfaces(output_manager, renderer, state, &mut output_surfaces);
    output_surfaces
}

/// Discover and initialize connectors that are not already represented.
/// Existing entries are deliberately left intact so a hot-plug does not
/// modeset or reset unaffected outputs.
pub fn add_new_output_surfaces(
    output_manager: &mut ManagedDrmOutputManager,
    renderer: &mut GlesRenderer,
    state: &mut WaylandState,
    output_surfaces: &mut Vec<OutputSurfaceEntry>,
) {
    let mut output_x_offset = output_surfaces
        .iter()
        .filter(|entry| entry.enabled)
        .map(|entry| entry.rect.x.saturating_add(entry.rect.w))
        .max()
        .unwrap_or(0);

    let res = output_manager
        .device()
        .resource_handles()
        .expect("drm resource_handles");
    let mut used_crtcs: Vec<crtc::Handle> =
        output_surfaces.iter().map(|entry| entry.crtc).collect();
    let existing_connectors: Vec<connector::Handle> = output_surfaces
        .iter()
        .map(|entry| entry.connector)
        .collect();
    let init_render_elements = DrmOutputRenderElements::<GlesRenderer, DrmExtras>::default();

    let mut pending = Vec::new();
    for &conn_handle in res.connectors() {
        if existing_connectors.contains(&conn_handle) {
            continue;
        }
        let Some(candidate) = drm_output_candidate(output_manager, &res, conn_handle) else {
            continue;
        };
        pending.push(candidate);
    }

    let candidate_crtcs: Vec<Vec<_>> = pending
        .iter()
        .map(|candidate| candidate.crtcs.clone())
        .collect();
    let Some(assignments) = complete_crtc_assignment(&candidate_crtcs, &used_crtcs) else {
        log::warn!(
            "could not find a complete CRTC assignment for {} newly connected outputs",
            pending.len()
        );
        return;
    };

    for (candidate, crtc) in pending.into_iter().zip(assignments) {
        let spec = candidate.assign(crtc);
        used_crtcs.push(spec.crtc);
        let Some(entry) = initialize_drm_output_surface(
            output_manager,
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
}

/// Return the connector handles currently capable of producing an output.
pub fn usable_connector_handles(
    output_manager: &ManagedDrmOutputManager,
) -> std::io::Result<Vec<connector::Handle>> {
    let resources = output_manager.device().resource_handles()?;
    Ok(resources
        .connectors()
        .iter()
        .copied()
        .filter(|connector| {
            output_manager
                .device()
                .get_connector(*connector, false)
                .is_ok_and(|info| is_usable_connector(&info))
        })
        .collect())
}

struct DrmOutputSpec {
    connector: connector::Handle,
    crtc: crtc::Handle,
    mode: control::Mode,
    modes: Vec<control::Mode>,
    pixel_size: crate::types::Size,
    physical_size: crate::types::Size,
    name: String,
}

struct DrmOutputCandidate {
    connector: connector::Handle,
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
    connector: connector::Handle,
) -> Option<DrmOutputCandidate> {
    let conn_info = output_manager
        .device()
        .get_connector(connector, false)
        .ok()?;
    if !is_usable_connector(&conn_info) {
        return None;
    }

    let mode = best_connector_mode(conn_info.modes())?;
    let crtcs: Vec<_> = conn_info
        .encoders()
        .iter()
        .filter_map(|&encoder| output_manager.device().get_encoder(encoder).ok())
        .flat_map(|encoder| resources.filter_crtcs(encoder.possible_crtcs()))
        .collect::<std::collections::HashSet<_>>()
        .into_iter()
        .collect();
    if crtcs.is_empty() {
        return None;
    }
    let (width, height) = mode.size();
    let physical_size = conn_info.size().unwrap_or((0, 0));

    Some(DrmOutputCandidate {
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
    })
}

fn is_usable_connector(conn_info: &connector::Info) -> bool {
    // `Unknown` is not evidence of a live sink. Keeping such a connector
    // active reserves its CRTC and can prevent a newly enumerated MST head
    // from being assigned. The hotplug settling probes will add it once KMS
    // reports the authoritative Connected state.
    conn_info.state() == connector::State::Connected && !conn_info.modes().is_empty()
}

fn best_connector_mode(modes: &[control::Mode]) -> Option<control::Mode> {
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

/// Find a complete output-to-CRTC matching. Greedy allocation is incorrect for
/// MST docks because an early flexible connector can consume the only CRTC
/// available to a later constrained connector.
pub(super) fn complete_crtc_assignment<T>(
    candidates: &[Vec<T>],
    unavailable: &[T],
) -> Option<Vec<T>>
where
    T: Copy + Eq,
{
    fn search<T>(candidates: &[Vec<T>], assigned: &mut [Option<T>], used: &mut Vec<T>) -> bool
    where
        T: Copy + Eq,
    {
        let next = (0..candidates.len())
            .filter(|index| assigned[*index].is_none())
            .min_by_key(|index| {
                candidates[*index]
                    .iter()
                    .filter(|candidate| !used.contains(candidate))
                    .count()
            });
        let Some(index) = next else { return true };
        for candidate in candidates[index].iter().copied() {
            if used.contains(&candidate) {
                continue;
            }
            assigned[index] = Some(candidate);
            used.push(candidate);
            if search(candidates, assigned, used) {
                return true;
            }
            used.pop();
            assigned[index] = None;
        }
        false
    }

    let mut assigned = vec![None; candidates.len()];
    let mut used = unavailable.to_vec();
    search(candidates, &mut assigned, &mut used)
        .then(|| assigned.into_iter().map(Option::unwrap).collect())
}

fn initialize_drm_output_surface(
    output_manager: &mut ManagedDrmOutputManager,
    renderer: &mut GlesRenderer,
    state: &mut WaylandState,
    init_render_elements: &DrmOutputRenderElements<GlesRenderer, DrmExtras>,
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

    Some(OutputSurfaceEntry {
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
        position_source: crate::backend::output::OutputPositionSource::Automatic,
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

fn create_drm_wayland_output(state: &WaylandState, spec: &DrmOutputSpec, x_offset: i32) -> Output {
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
    state: &mut WaylandState,
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
    gbm_device: &GbmDevice<DrmDeviceFd>,
) -> ManagedDrmOutputManager {
    let allocator = GbmAllocator::new(
        gbm_device.clone(),
        GbmBufferFlags::RENDERING | GbmBufferFlags::SCANOUT,
    );
    let exporter = GbmFramebufferExporter::new(gbm_device.clone(), NodeFilter::None);
    let color_formats: [Fourcc; 2] = [Fourcc::Argb8888, Fourcc::Xrgb8888];
    let renderer_formats: Vec<_> = renderer.dmabuf_formats().into_iter().collect();

    ManagedDrmOutputManager::new(
        drm_device,
        allocator,
        exporter,
        Some(gbm_device.clone()),
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
