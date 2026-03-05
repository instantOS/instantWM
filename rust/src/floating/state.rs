//! Floating state transitions and geometry persistence.

use crate::animation::animate_client;
use crate::backend::BackendOps;
use crate::client::restore_border_width;
use crate::contexts::WmCtx;
use crate::layouts::arrange;
use crate::types::*;

pub fn set_floating_in_place(ctx: &mut WmCtx, win: WindowId) {
    match ctx {
        WmCtx::X11(x11) => {
            if let Some(client) = x11.core.g.clients.get_mut(&win) {
                client.isfloating = true;
            }

            restore_border_width(&mut x11.core, win);
            let restored_bw = x11
                .core
                .g
                .clients
                .get(&win)
                .map(|c| c.border_width)
                .unwrap_or(0);
            BackendOps::set_border_width(&x11.backend, win, restored_bw);

            if let Some(ref scheme) = x11.core.g.cfg.borderscheme {
                let pixel = scheme.float_focus.bg.color.pixel;
                let _ = x11rb::protocol::xproto::change_window_attributes(
                    x11.x11.conn,
                    win.into(),
                    &x11rb::protocol::xproto::ChangeWindowAttributesAux::new()
                        .border_pixel(Some(pixel as u32)),
                );
                let _ = x11.x11.conn.flush();
            }
        }
        WmCtx::Wayland(_) => {}
    }
}

pub fn save_floating_win(ctx: &mut WmCtx, win: WindowId) {
    match ctx {
        WmCtx::X11(x11) => {
            if let Some(client) = x11.core.g.clients.get_mut(&win) {
                client.float_geo = client.geo;
            }
        }
        WmCtx::Wayland(_) => {}
    }
}

pub fn restore_floating_win(ctx: &mut WmCtx, win: WindowId) {
    match ctx {
        WmCtx::X11(x11) => {
            let float_geo = x11.core.g.clients.get(&win).map(|c| c.float_geo);
            if let Some(rect) = float_geo {
                crate::client::resize(&mut x11.core, &x11.x11, win, &rect, false);
            }
        }
        WmCtx::Wayland(_) => {}
    }
}

pub fn apply_float_change(
    ctx: &mut WmCtx,
    win: WindowId,
    floating: bool,
    animate: bool,
    update_borders: bool,
) {
    match ctx {
        WmCtx::X11(x11) => {
            if floating {
                if let Some(client) = x11.core.g.clients.get_mut(&win) {
                    client.isfloating = true;
                }

                if update_borders {
                    restore_border_width(&mut x11.core, win);
                    let restored_bw = x11
                        .core
                        .g
                        .clients
                        .get(&win)
                        .map(|c| c.border_width)
                        .unwrap_or(0);
                    BackendOps::set_border_width(&x11.backend, win, restored_bw);

                    if let Some(ref scheme) = x11.core.g.cfg.borderscheme {
                        let pixel = scheme.float_focus.bg.color.pixel;
                        let _ = x11rb::protocol::xproto::change_window_attributes(
                            x11.x11.conn,
                            win.into(),
                            &x11rb::protocol::xproto::ChangeWindowAttributesAux::new()
                                .border_pixel(Some(pixel as u32)),
                        );
                        let _ = x11.x11.conn.flush();
                    }
                }

                let saved_geo = x11.core.g.clients.get(&win).map(|c| c.float_geo);
                let Some(saved_geo) = saved_geo else { return };

                if animate {
                    animate_client(
                        ctx,
                        win,
                        &Rect {
                            x: saved_geo.x,
                            y: saved_geo.y,
                            w: saved_geo.w,
                            h: saved_geo.h,
                        },
                        7,
                        0,
                    );
                } else {
                    crate::client::resize(&mut x11.core, &x11.x11, win, &saved_geo, false);
                }
            } else {
                let client_count = x11.core.g.clients.len();
                if let Some(client) = x11.core.g.clients.get_mut(&win) {
                    client.isfloating = false;
                    client.float_geo = client.geo;

                    if update_borders
                        && client_count <= 1
                        && client.snap_status == SnapPosition::None
                    {
                        if client.border_width != 0 {
                            client.old_border_width = client.border_width;
                        }
                        client.border_width = 0;
                    }
                }
            }
        }
        WmCtx::Wayland(_) => {}
    }
}

pub fn toggle_floating(ctx: &mut WmCtx) {
    let selected_window = match ctx {
        WmCtx::X11(x11) => {
            let mon = x11.core.g.selected_monitor();
            match mon.sel {
                Some(sel) if Some(sel) != mon.overlay => {
                    if let Some(c) = x11.core.g.clients.get(&sel) {
                        if c.is_true_fullscreen() {
                            return;
                        }
                    }
                    Some(sel)
                }
                _ => None,
            }
        }
        WmCtx::Wayland(_) => None,
    };

    let Some(win) = selected_window else { return };

    let (is_floating, is_fixed) = match ctx {
        WmCtx::X11(x11) => x11
            .core
            .g
            .clients
            .get(&win)
            .map(|c| (c.isfloating, c.isfixed))
            .unwrap_or((false, false)),
        WmCtx::Wayland(_) => (false, false),
    };

    let new_state = !is_floating || is_fixed;
    apply_float_change(ctx, win, new_state, true, true);
    arrange(
        ctx,
        Some(match ctx {
            WmCtx::X11(x11) => x11.core.g.selected_monitor_id(),
            WmCtx::Wayland(_) => 0,
        }),
    );
}

pub fn change_floating_win(ctx: &mut WmCtx, win: WindowId) {
    let (is_fullscreen, is_fake_fullscreen, is_floating, is_fixed) = match ctx {
        WmCtx::X11(x11) => match x11.core.g.clients.get(&win) {
            Some(c) => (c.is_fullscreen, c.isfakefullscreen, c.isfloating, c.isfixed),
            None => return,
        },
        WmCtx::Wayland(_) => return,
    };

    if is_fake_fullscreen {
        return;
    }

    let new_state = !is_floating || is_fixed;
    apply_float_change(ctx, win, new_state, false, false);
    arrange(
        ctx,
        Some(match ctx {
            WmCtx::X11(x11) => x11.core.g.selected_monitor_id(),
            WmCtx::Wayland(_) => 0,
        }),
    );
}

pub fn set_floating(ctx: &mut WmCtx, win: WindowId, should_arrange: bool) {
    let (is_true_fullscreen, is_floating) = match ctx {
        WmCtx::X11(x11) => match x11.core.g.clients.get(&win) {
            Some(c) => (c.is_true_fullscreen(), c.isfloating),
            None => return,
        },
        WmCtx::Wayland(_) => return,
    };

    if is_true_fullscreen {
        return;
    }
    if is_floating {
        return;
    }

    apply_float_change(ctx, win, true, false, false);

    if should_arrange {
        arrange(
            ctx,
            Some(match ctx {
                WmCtx::X11(x11) => x11.core.g.selected_monitor_id(),
                WmCtx::Wayland(_) => 0,
            }),
        );
    }
}

pub fn set_tiled(ctx: &mut WmCtx, win: WindowId, should_arrange: bool) {
    let (is_true_fullscreen, is_floating, is_fixed) = match ctx {
        WmCtx::X11(x11) => match x11.core.g.clients.get(&win) {
            Some(c) => (c.is_true_fullscreen(), c.isfloating, c.isfixed),
            None => return,
        },
        WmCtx::Wayland(_) => return,
    };

    if is_true_fullscreen {
        return;
    }
    if !is_floating && !is_fixed {
        return;
    }

    apply_float_change(ctx, win, false, false, false);

    if should_arrange {
        arrange(
            ctx,
            Some(match ctx {
                WmCtx::X11(x11) => x11.core.g.selected_monitor_id(),
                WmCtx::Wayland(_) => 0,
            }),
        );
    }
}

pub fn temp_fullscreen(ctx: &mut WmCtx) {
    let (fullscreen_win, selected_window, animated) = match ctx {
        WmCtx::X11(x11) => {
            let mon = x11.core.g.selected_monitor();
            (mon.fullscreen, mon.sel, x11.core.g.animated)
        }
        WmCtx::Wayland(_) => (None, None, false),
    };

    if let Some(win) = fullscreen_win {
        let is_floating = match ctx {
            WmCtx::X11(x11) => x11
                .core
                .g
                .clients
                .get(&win)
                .map(|c| c.isfloating)
                .unwrap_or(false),
            WmCtx::Wayland(_) => false,
        };

        if is_floating
            || !super::helpers::has_tiling_layout(match ctx {
                WmCtx::X11(x11) => &x11.core,
                WmCtx::Wayland(_) => panic!("Wayland not supported"),
            })
        {
            restore_floating_win(ctx, win);
            match ctx {
                WmCtx::X11(x11) => super::helpers::apply_size(&mut x11.core, &x11.x11, win),
                WmCtx::Wayland(_) => {}
            }
        }

        match ctx {
            WmCtx::X11(x11) => x11.core.g.selected_monitor_mut().fullscreen = None,
            WmCtx::Wayland(_) => {}
        }
    } else {
        let Some(win) = selected_window else { return };

        match ctx {
            WmCtx::X11(x11) => x11.core.g.selected_monitor_mut().fullscreen = Some(win),
            WmCtx::Wayland(_) => {}
        }

        if super::helpers::check_floating(
            match ctx {
                WmCtx::X11(x11) => &x11.core,
                WmCtx::Wayland(_) => panic!("Wayland not supported"),
            },
            win,
        ) {
            save_floating_win(ctx, win);
        }
    }

    if animated {
        match ctx {
            WmCtx::X11(x11) => x11.core.g.animated = false,
            WmCtx::Wayland(_) => {}
        }
        arrange(
            ctx,
            Some(match ctx {
                WmCtx::X11(x11) => x11.core.g.selected_monitor_id(),
                WmCtx::Wayland(_) => 0,
            }),
        );
        match ctx {
            WmCtx::X11(x11) => x11.core.g.animated = true,
            WmCtx::Wayland(_) => {}
        }
    } else {
        arrange(
            ctx,
            Some(match ctx {
                WmCtx::X11(x11) => x11.core.g.selected_monitor_id(),
                WmCtx::Wayland(_) => 0,
            }),
        );
    }

    if let Some(win) = match ctx {
        WmCtx::X11(x11) => x11.core.g.selected_monitor().fullscreen,
        WmCtx::Wayland(_) => None,
    } {
        ctx.backend().raise_window(win);
    }
}
