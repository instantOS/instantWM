use std::process::Command;
use std::time::Duration;

use crate::config::init_config;
use crate::monitor::update_geom;
use crate::startup::common_wayland::{wayland_font_height_from_size, wayland_font_size_from_config};
use crate::types::{CLOSE_BUTTON_DETAIL, CLOSE_BUTTON_WIDTH};
use crate::wm::Wm;

pub(super) fn init_wayland_globals(wm: &mut Wm) {
    let cfg = init_config();
    wm.g.cfg.screen_width = 1280;
    wm.g.cfg.screen_height = 800;
    crate::globals::apply_config(&mut wm.g, &cfg);
    crate::globals::apply_tags_config(&mut wm.g, &cfg);
    wm.g.cfg.showbar = true;
    let font_size = wayland_font_size_from_config(&cfg.fonts);
    let font_height = wayland_font_height_from_size(font_size);
    wm.bar_painter.set_font_size(font_size);
    let min_bar_height = CLOSE_BUTTON_WIDTH + CLOSE_BUTTON_DETAIL + 2;
    wm.g.cfg.bar_height = (if cfg.barheight > 0 {
        font_height + cfg.barheight
    } else {
        font_height + 12
    })
    .max(min_bar_height);
    wm.g.cfg.horizontal_padding = font_height;
    wm.g.cfg.numlockmask = 0;
    update_geom(&mut wm.ctx());
}

pub(super) fn apply_wayland_session_env(socket_name: &str) {
    std::env::set_var("WAYLAND_DISPLAY", socket_name);
    std::env::set_var("XDG_SESSION_TYPE", "wayland");
    std::env::remove_var("DISPLAY");
    std::env::set_var("GDK_BACKEND", "wayland");
    std::env::set_var("QT_QPA_PLATFORM", "wayland");
    std::env::set_var("SDL_VIDEODRIVER", "wayland");
    std::env::set_var("CLUTTER_BACKEND", "wayland");
}

pub(super) fn spawn_wayland_smoke_window() {
    if std::env::var("INSTANTWM_WL_AUTOSPAWN").ok().as_deref() == Some("0") {
        return;
    }
    std::thread::spawn(|| {
        std::thread::sleep(Duration::from_millis(800));
        let _ = Command::new("sh")
            .arg("-lc")
            .arg("for app in gtk3-demo thunar xmessage; do command -v \"$app\" >/dev/null 2>&1 && exec \"$app\"; done; exit 0")
            .spawn();
    });
}

pub(super) fn sanitize_wayland_size(w: i32, h: i32) -> (i32, i32) {
    const WAYLAND_MIN_DIM: i32 = 64;
    (w.max(WAYLAND_MIN_DIM), h.max(WAYLAND_MIN_DIM))
}
