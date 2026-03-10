use smithay::backend::allocator::gbm::{GbmAllocator, GbmBufferFlags, GbmDevice};
use smithay::backend::allocator::Fourcc;
use smithay::backend::drm::{DrmDevice, DrmDeviceFd, GbmBufferedSurface};
use smithay::backend::renderer::damage::OutputDamageTracker;
use smithay::backend::renderer::gles::GlesRenderer;
use smithay::backend::renderer::ImportDma;
use smithay::output::{Mode as OutputMode, Output, PhysicalProperties, Scale, Subpixel};
use smithay::reexports::drm::control::{connector, crtc, Device as ControlDevice};

use crate::backend::wayland::compositor::WaylandState;

use super::state::OutputSurfaceEntry;

pub fn build_output_surfaces(
    drm_device: &mut DrmDevice,
    renderer: &mut GlesRenderer,
    state: &WaylandState,
    gbm_device: &GbmDevice<DrmDeviceFd>,
) -> Vec<OutputSurfaceEntry> {
    let gbm_allocator = GbmAllocator::new(
        gbm_device.clone(),
        GbmBufferFlags::RENDERING | GbmBufferFlags::SCANOUT,
    );
    let color_formats: &[Fourcc] = &[Fourcc::Argb8888, Fourcc::Xrgb8888];
    let renderer_formats: Vec<_> = renderer.dmabuf_formats().into_iter().collect();

    let mut output_surfaces: Vec<OutputSurfaceEntry> = Vec::new();
    let mut output_x_offset: i32 = 0;

    let res = drm_device.resource_handles().expect("drm resource_handles");
    let mut used_crtcs: Vec<crtc::Handle> = Vec::new();

    for &conn_handle in res.connectors() {
        let Ok(conn_info) = drm_device.get_connector(conn_handle, false) else {
            continue;
        };
        if conn_info.state() != connector::State::Connected
            && conn_info.state() != connector::State::Unknown
        {
            continue;
        }
        let modes = conn_info.modes();
        if modes.is_empty() {
            continue;
        }

        let mut sorted_modes = modes.to_vec();
        sorted_modes.sort_by(|a, b| {
            let (aw, ah) = a.size();
            let (bw, bh) = b.size();
            (bw as u64 * bh as u64)
                .cmp(&(aw as u64 * ah as u64))
                .then_with(|| b.vrefresh().cmp(&a.vrefresh()))
        });
        let mode = sorted_modes[0];

        let encoder_crtcs: Vec<crtc::Handle> = conn_info
            .encoders()
            .iter()
            .filter_map(|&enc_h| drm_device.get_encoder(enc_h).ok())
            .flat_map(|enc| res.filter_crtcs(enc.possible_crtcs()))
            .collect();

        let Some(&picked_crtc) = encoder_crtcs.iter().find(|c| !used_crtcs.contains(c)) else {
            continue;
        };
        used_crtcs.push(picked_crtc);

        let drm_surface = drm_device
            .create_surface(picked_crtc, mode, &[conn_handle])
            .expect("create_surface");
        let gbm_surface = GbmBufferedSurface::new(
            drm_surface,
            gbm_allocator.clone(),
            color_formats,
            renderer_formats.iter().cloned(),
        )
        .expect("GbmBufferedSurface::new");

        let (mode_w, mode_h) = mode.size();
        let (mode_w, mode_h) = (mode_w as i32, mode_h as i32);
        let output_name = format!(
            "{}-{}",
            connector_type_name(conn_info.interface()),
            conn_info.interface_id()
        );
        log::info!(
            "Output {output_name}: {mode_w}x{mode_h}@{}Hz on CRTC {:?}",
            mode.vrefresh(),
            picked_crtc
        );

        let output = Output::new(
            output_name,
            PhysicalProperties {
                size: {
                    let (mm_w, mm_h) = conn_info.size().unwrap_or((0, 0));
                    (mm_w as i32, mm_h as i32).into()
                },
                subpixel: Subpixel::Unknown,
                make: "instantOS".into(),
                model: "instantWM".into(),
            },
        );
        let out_mode = OutputMode {
            size: (mode_w, mode_h).into(),
            refresh: (mode.vrefresh() as i32) * 1000,
        };
        output.change_current_state(
            Some(out_mode),
            Some(smithay::utils::Transform::Normal),
            Some(Scale::Integer(1)),
            Some((output_x_offset, 0).into()),
        );
        output.set_preferred(out_mode);
        let _global = output.create_global::<WaylandState>(&state.display_handle);

        let damage_tracker = OutputDamageTracker::from_output(&output);

        output_surfaces.push(OutputSurfaceEntry {
            crtc: picked_crtc,
            surface: gbm_surface,
            output: output.clone(),
            damage_tracker,
            x_offset: output_x_offset,
            width: mode_w,
            height: mode_h,
        });
        output_x_offset += mode_w;
    }

    output_surfaces
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
