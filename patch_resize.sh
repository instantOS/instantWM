#!/bin/bash
cat << 'DIFF' > patch.diff
<<<<<<< SEARCH
    let is_blocked = ctx
        .g()
        .clients
        .get(&win)
        .map(|c| c.is_true_fullscreen())
        .unwrap_or(false);
    if is_blocked {
        return;
    };

    match ctx {
        WmCtx::X11(ctx_x11) => {
            let dir = {
                let Some(c) = ctx_x11.core.g.clients.get(&win) else {
                    return;
                };
=======
    let is_blocked = match ctx.g().clients.get(&win) {
        Some(c) => c.is_true_fullscreen(),
        None => return,
    };
    if is_blocked {
        return;
    }

    match ctx {
        WmCtx::X11(ctx_x11) => {
            let dir = {
                let Some(c) = ctx_x11.core.g.clients.get(&win) else {
                    return;
                };
>>>>>>> REPLACE
<<<<<<< SEARCH
    let is_blocked = ctx
        .core
        .client(win)
        .map(|c| c.is_true_fullscreen())
        .unwrap_or(false);
    if is_blocked {
        return;
    };

    let (orig_left, orig_top, orig_right, orig_bottom, border_width) = {
        match ctx.core.client(win) {
            Some(c) => (
                c.geo.x,
                c.geo.y,
                c.geo.x + c.geo.w,
                c.geo.y + c.geo.h,
                c.border_width,
            ),
            None => return,
        }
    };
=======
    let (is_blocked, orig_left, orig_top, orig_right, orig_bottom, border_width) = match ctx.core.client(win) {
        Some(c) => (
            c.is_true_fullscreen(),
            c.geo.x,
            c.geo.y,
            c.geo.x + c.geo.w,
            c.geo.y + c.geo.h,
            c.border_width,
        ),
        None => return,
    };
    if is_blocked {
        return;
    }
>>>>>>> REPLACE
<<<<<<< SEARCH
            if should_toggle {
                with_wm_ctx_x11(ctx, |ctx| toggle_floating(ctx));
            } else {
                let is_floating = ctx.core.client(win).map(|c| c.isfloating).unwrap_or(false);
                let has_tiling = ctx.core.g.selected_monitor().is_tiling_layout();

                if !has_tiling || is_floating {
                    with_wm_ctx_x11(ctx, |ctx| {
=======
            if should_toggle {
                with_wm_ctx_x11(ctx, |ctx| toggle_floating(ctx));
            } else {
                let is_floating = match ctx.core.client(win) {
                    Some(c) => c.isfloating,
                    None => return,
                };
                let has_tiling = ctx.core.g.selected_monitor().is_tiling_layout();

                if !has_tiling || is_floating {
                    with_wm_ctx_x11(ctx, |ctx| {
>>>>>>> REPLACE
<<<<<<< SEARCH
    let is_fullscreen = ctx
        .core
        .g
        .clients
        .get(&win)
        .map(|c| c.is_fullscreen)
        .unwrap_or(false);
    if is_fullscreen {
        return;
    };

    let (orig_left, orig_top) = {
        match ctx.core.client(win) {
            Some(c) => (c.geo.x, c.geo.y),
            None => return,
        }
    };
=======
    let (is_fullscreen, orig_left, orig_top) = match ctx.core.g.clients.get(&win) {
        Some(c) => (c.is_fullscreen, c.geo.x, c.geo.y),
        None => return,
    };
    if is_fullscreen {
        return;
    }
>>>>>>> REPLACE
DIFF
patch -p1 rust/src/mouse/resize.rs < patch.diff
