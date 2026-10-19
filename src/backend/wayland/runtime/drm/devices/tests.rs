use super::*;
use std::os::unix::ffi::OsStrExt;
/// Opt-in hardware test: deliberately lights an otherwise unused EVDI
/// display. Never run against a compositor's primary KMS device.
/// INSTANTWM_TEST_RENDER=/dev/dri/renderD128
/// INSTANTWM_TEST_SCANOUT=/dev/dri/card0
#[test]
#[ignore = "requires an unused EVDI device and a hardware rendering node"]
fn evdi_scanout_from_primary_gpu() {
    use crate::backend::wayland::render::drm::{
        DrmFrameMetadata, DrmOutputElement, build_output_surfaces,
    };
    use smithay::backend::drm::DrmEvent;
    use smithay::backend::drm::compositor::{FrameFlags, PrimaryPlaneElement};
    use smithay::backend::egl::{EGLContext, EGLDisplay};
    use smithay::desktop::utils::OutputPresentationFeedback;
    use smithay::reexports::drm::Device;
    use std::fs::OpenOptions;
    use std::time::Duration;

    let render_path = std::env::var("INSTANTWM_TEST_RENDER").expect("set INSTANTWM_TEST_RENDER");
    let scanout_path = std::env::var("INSTANTWM_TEST_SCANOUT").expect("set INSTANTWM_TEST_SCANOUT");
    let open = |path| {
        DrmDeviceFd::new(DeviceFd::from(OwnedFd::from(
            OpenOptions::new()
                .read(true)
                .write(true)
                .open(path)
                .unwrap(),
        )))
    };
    let render_fd = open(&render_path);
    let scanout_fd = open(&scanout_path);
    assert_eq!(
        scanout_fd.get_driver().unwrap().name().as_bytes(),
        b"evdi",
        "this test must only take over an unused EVDI device"
    );
    let render_gbm = GbmDevice::new(render_fd).unwrap();
    let scanout_gbm = GbmDevice::new(scanout_fd.clone()).unwrap();
    let display = unsafe { EGLDisplay::new(render_gbm.clone()) }.unwrap();
    let context = EGLContext::new(&display).unwrap();
    let mut renderer = unsafe { GlesRenderer::new(context) }.unwrap();
    let (device, notifier) = DrmDevice::new(scanout_fd, true).unwrap();
    let mut manager = create_output_manager(device, &renderer, &render_gbm, &scanout_gbm);
    let (mut event_loop, mut state) = crate::test_support::new_compositor();
    let mut outputs = build_output_surfaces(
        &mut manager,
        DrmDeviceId(1),
        &mut renderer,
        &mut state.native,
    );
    assert!(!outputs.is_empty(), "no connected EVDI output initialized");
    let (tx, rx) = mpsc::channel();
    event_loop
        .handle()
        .insert_source(notifier, move |event, _, _| {
            if let DrmEvent::VBlank(crtc) = event {
                tx.send(crtc).unwrap();
            }
        })
        .unwrap();
    let entry = &mut outputs[0];
    let surface = entry.surface.as_mut().unwrap();
    let elements: &[DrmOutputElement] = &[];
    let deadline = std::time::Instant::now() + Duration::from_secs(3);
    loop {
        let frame = surface
            .render_frame(
                &mut renderer,
                elements,
                [0.1, 0.35, 0.65, 1.0],
                FrameFlags::empty(),
            )
            .unwrap();
        if let PrimaryPlaneElement::Swapchain(primary) = &frame.primary_element {
            primary.sync.wait().unwrap();
        } else {
            panic!("expected a rendered GPU swapchain buffer");
        }
        match surface.queue_frame(DrmFrameMetadata {
            presentation_feedback: OutputPresentationFeedback::new(&entry.output),
        }) {
            Ok(()) => break,
            Err(smithay::backend::drm::compositor::FrameError::DrmError(
                smithay::backend::drm::DrmError::Access(error),
            )) if error.source.raw_os_error() == Some(libc::EBUSY) => {
                // As in the runtime, let asynchronous KMS setup finish
                // before retrying a busy controller. Other failures fail.
                assert!(
                    std::time::Instant::now() < deadline,
                    "EVDI stayed busy: {error}"
                );
                event_loop
                    .dispatch(Duration::from_millis(50), &mut state)
                    .unwrap();
                for _ in rx.try_iter() {
                    surface.frame_submitted().unwrap();
                }
            }
            Err(error) => panic!("EVDI queue failed: {error:?}"),
        }
    }
    let deadline = std::time::Instant::now() + Duration::from_secs(3);
    loop {
        event_loop
            .dispatch(Duration::from_millis(50), &mut state)
            .unwrap();
        if let Ok(crtc) = rx.try_recv() {
            assert_eq!(crtc, entry.crtc);
            assert!(surface.frame_submitted().unwrap().is_some());
            break;
        }
        assert!(
            std::time::Instant::now() < deadline,
            "EVDI page flip did not complete"
        );
    }
    // Exercise the production scene/cursor path, DPMS and logical
    // disable/re-enable, not just the allocator's initial solid frame.
    state.native.space.map_output(&entry.output, (0, 0));
    let cursor = crate::backend::wayland::render::cursor_theme::CursorManager::new("default", 24);
    let render_elements = smithay::backend::drm::output::DrmOutputRenderElements::<
        GlesRenderer,
        DrmOutputElement,
    >::default();
    for phase in 0..4 {
        match phase {
            1 => entry
                .surface
                .as_ref()
                .unwrap()
                .with_compositor(|compositor| compositor.clear())
                .unwrap(),
            2 => {
                entry.surface.take();
                let mode = entry
                    .modes
                    .iter()
                    .find(|(mode, _)| Some(*mode) == entry.output.current_mode())
                    .unwrap()
                    .1;
                let initialized = manager
                    .lock()
                    .initialize_output(
                        entry.crtc,
                        mode,
                        &[entry.connector],
                        &entry.output,
                        None,
                        &mut renderer,
                        &render_elements,
                    )
                    .unwrap();
                entry.surface = Some(initialized);
            }
            3 => {
                if let Some((mode, native)) = entry
                    .modes
                    .iter()
                    .find(|(mode, _)| {
                        mode.size.w == 1920 && mode.size.h == 1080 && mode.refresh == 60000
                    })
                    .copied()
                {
                    entry
                        .surface
                        .as_mut()
                        .unwrap()
                        .use_mode(native, &mut renderer, &render_elements)
                        .unwrap();
                    entry
                        .output
                        .change_current_state(Some(mode), None, None, None);
                    entry.rect.w = mode.size.w;
                    entry.rect.h = mode.size.h;
                }
            }
            _ => {}
        }
        let policy = crate::backend::wayland::stacking::OutputScenePolicy::for_output(
            &state.wm.core.state,
            &entry.output,
        );
        let deadline = std::time::Instant::now() + Duration::from_secs(3);
        loop {
            let shared_scene = crate::backend::wayland::render::scene::build_shared_scene_elements(
                &mut state.wm,
                &mut state.native,
                &mut Default::default(),
            );
            let outcome = crate::backend::wayland::render::drm::render_drm_output(
                &mut state.native,
                &mut renderer,
                entry,
                &cursor,
                std::time::Instant::now(),
                Some(shared_scene),
                policy,
            );
            if outcome == crate::backend::wayland::render::drm::RenderOutcome::Submitted {
                break;
            }
            assert!(
                std::time::Instant::now() < deadline,
                "production EVDI render failed in phase {phase}: {outcome:?}"
            );
            event_loop
                .dispatch(Duration::from_millis(50), &mut state)
                .unwrap();
            for _ in rx.try_iter() {
                entry.surface.as_mut().unwrap().frame_submitted().unwrap();
            }
        }
        loop {
            event_loop
                .dispatch(Duration::from_millis(50), &mut state)
                .unwrap();
            if let Ok(crtc) = rx.try_recv() {
                assert_eq!(crtc, entry.crtc);
                assert!(
                    entry
                        .surface
                        .as_mut()
                        .unwrap()
                        .frame_submitted()
                        .unwrap()
                        .is_some()
                );
                break;
            }
            assert!(
                std::time::Instant::now() < deadline,
                "production EVDI page flip timed out in phase {phase}"
            );
        }
    }
    eprintln!(
        "Primary GPU -> EVDI import, KMS submission and page flip succeeded: {}",
        entry.output.name()
    );
}

#[test]
fn reused_card_path_is_retired_before_its_replacement_is_opened() {
    let known = [
        (PRIMARY_DEVICE, Path::new("/dev/dri/card1"), 1),
        (DrmDeviceId(1), Path::new("/dev/dri/card0"), 2),
    ];
    let paths = known
        .iter()
        .map(|(_, path, _)| path.to_path_buf())
        .collect();
    let changes = plan_device_changes(known.into_iter(), paths, &[2].into());
    assert_eq!(changes.removed, vec![DrmDeviceId(1)]);
    assert_eq!(changes.added, vec![PathBuf::from("/dev/dri/card0")]);
}

#[test]
fn connector_hotplug_does_not_reopen_devices() {
    let known = [
        (PRIMARY_DEVICE, Path::new("/dev/dri/card1"), 1),
        (DrmDeviceId(1), Path::new("/dev/dri/card0"), 2),
    ];
    let paths = known
        .iter()
        .map(|(_, path, _)| path.to_path_buf())
        .collect();
    let changes = plan_device_changes(known.into_iter(), paths, &Default::default());
    assert!(changes.removed.is_empty());
    assert!(changes.added.is_empty());
}

#[test]
fn secondary_device_removal_preserves_the_primary() {
    let known = [
        (PRIMARY_DEVICE, Path::new("/dev/dri/card1"), 1),
        (DrmDeviceId(1), Path::new("/dev/dri/card0"), 2),
    ];
    let changes = plan_device_changes(
        known.into_iter(),
        vec!["/dev/dri/card1".into()],
        &Default::default(),
    );
    assert_eq!(changes.removed, vec![DrmDeviceId(1)]);
    assert!(changes.added.is_empty());
}
