use crate::contexts::WmCtx;
use x11rb::protocol::xproto::ConnectionExt;

const NUM_SCHEMEHOVERTYPES: usize = 2;
const NUM_SCHEMECOLORTYPES: usize = 3;
const NUM_SCHEMEWINDOWTYPES: usize = 7;
const NUM_SCHEMETAGTYPES: usize = 5;
const NUM_SCHEMECLOSETYPES: usize = 3;
const MAX_TAGLEN: usize = 16;

const SCHEME_HOVER_TYPES: [&str; NUM_SCHEMEHOVERTYPES] = ["normal", "hover"];
const SCHEME_COLOR_TYPES: [&str; NUM_SCHEMECOLORTYPES] = ["fg", "bg", "detail"];
const SCHEME_WINDOW_TYPES: [&str; NUM_SCHEMEWINDOWTYPES] = [
    "focus",
    "normal",
    "minimized",
    "sticky",
    "stickyfocus",
    "overlay",
    "overlayfocus",
];
const SCHEME_TAG_TYPES: [&str; NUM_SCHEMETAGTYPES] =
    ["inactive", "filled", "focus", "nofocus", "empty"];
const SCHEME_CLOSE_TYPES: [&str; NUM_SCHEMECLOSETYPES] = ["normal", "locked", "fullscreen"];

pub fn list_xresources() {
    for i in 0..NUM_SCHEMEHOVERTYPES {
        for q in 0..NUM_SCHEMECOLORTYPES {
            for u in 0..NUM_SCHEMEWINDOWTYPES {
                let propname = format!(
                    "{}.{}.win.{}",
                    SCHEME_HOVER_TYPES[i], SCHEME_WINDOW_TYPES[u], SCHEME_COLOR_TYPES[q]
                );
                println!("instantwm.{}", propname);
            }

            for u in 0..NUM_SCHEMETAGTYPES {
                let propname = format!(
                    "{}.{}.tag.{}",
                    SCHEME_HOVER_TYPES[i], SCHEME_TAG_TYPES[u], SCHEME_COLOR_TYPES[q]
                );
                println!("instantwm.{}", propname);
            }

            for u in 0..NUM_SCHEMECLOSETYPES {
                let propname = format!(
                    "{}.{}.close.{}",
                    SCHEME_HOVER_TYPES[i], SCHEME_CLOSE_TYPES[u], SCHEME_COLOR_TYPES[q]
                );
                println!("instantwm.{}", propname);
            }
        }
    }

    println!("normal.border");
    println!("focus.tile.border");
    println!("focus.float.border");
    println!("snap.border");
    println!("status.fg");
    println!("status.bg");
    println!("status.detail");
}

/// Load xresources and update the runtime configuration
/// This should be called after init_globals but before setup
pub fn load_xresources(ctx: &mut WmCtx) {
    let Some(conn) = ctx.x11_conn().map(|x11| x11.conn) else {
        return;
    };

    let Ok(res_cookie) = conn.get_property(
        false,
        ctx.g.cfg.root,
        x11rb::protocol::xproto::AtomEnum::RESOURCE_MANAGER,
        x11rb::protocol::xproto::AtomEnum::STRING,
        0,
        65536,
    ) else {
        return;
    };

    let Ok(res_reply) = res_cookie.reply() else {
        return;
    };

    let resource_str = String::from_utf8_lossy(&res_reply.value);

    load_color_resources(ctx, &resource_str);
    load_tag_resources(ctx, &resource_str);
}

fn load_color_resources(ctx: &mut WmCtx, resource_str: &str) {
    for i in 0..NUM_SCHEMEHOVERTYPES {
        for q in 0..NUM_SCHEMECOLORTYPES {
            for u in 0..NUM_SCHEMEWINDOWTYPES {
                let propname = format!(
                    "{}.{}.win.{}",
                    SCHEME_HOVER_TYPES[i], SCHEME_WINDOW_TYPES[u], SCHEME_COLOR_TYPES[q]
                );

                if let Some(value) = find_resource(resource_str, &propname) {
                    if i < ctx.g.cfg.windowcolors.len()
                        && u < ctx.g.cfg.windowcolors[i].len()
                        && q < ctx.g.cfg.windowcolors[i][u].len()
                    {
                        ctx.g.cfg.windowcolors[i][u][q] = Box::leak(value.into_boxed_str());
                    }
                }
            }

            for u in 0..NUM_SCHEMETAGTYPES {
                let propname = format!(
                    "{}.{}.tag.{}",
                    SCHEME_HOVER_TYPES[i], SCHEME_TAG_TYPES[u], SCHEME_COLOR_TYPES[q]
                );

                if let Some(value) = find_resource(resource_str, &propname) {
                    if i < ctx.g.tags.colors.len()
                        && u < ctx.g.tags.colors[i].len()
                        && q < ctx.g.tags.colors[i][u].len()
                    {
                        ctx.g.tags.colors[i][u][q] = Box::leak(value.into_boxed_str());
                    }
                }
            }

            for u in 0..NUM_SCHEMECLOSETYPES {
                let propname = format!(
                    "{}.{}.close.{}",
                    SCHEME_HOVER_TYPES[i], SCHEME_CLOSE_TYPES[u], SCHEME_COLOR_TYPES[q]
                );

                if let Some(value) = find_resource(resource_str, &propname) {
                    if i < ctx.g.cfg.closebuttoncolors.len()
                        && u < ctx.g.cfg.closebuttoncolors[i].len()
                        && q < ctx.g.cfg.closebuttoncolors[i][u].len()
                    {
                        ctx.g.cfg.closebuttoncolors[i][u][q] = Box::leak(value.into_boxed_str());
                    }
                }
            }
        }
    }

    let border_names = [
        "normal.border",
        "focus.tile.border",
        "focus.float.border",
        "snap.border",
    ];
    for (i, name) in border_names.iter().enumerate() {
        if let Some(value) = find_resource(resource_str, name) {
            if i < ctx.g.cfg.bordercolors.len() {
                ctx.g.cfg.bordercolors[i] = Box::leak(value.into_boxed_str());
            }
        }
    }

    let status_names = ["status.fg", "status.bg", "status.detail"];
    for (i, name) in status_names.iter().enumerate() {
        if let Some(value) = find_resource(resource_str, name) {
            if i < ctx.g.cfg.statusbarcolors.len() {
                ctx.g.cfg.statusbarcolors[i] = Box::leak(value.into_boxed_str());
            }
        }
    }
}

fn load_tag_resources(ctx: &mut WmCtx, resource_str: &str) {
    let mon_count = ctx.g.monitors.len();
    for i in 0..9 {
        let propname = format!("tag.{}", i + 1);
        if let Some(value) = find_resource(resource_str, &propname) {
            for mon_idx in 0..mon_count {
                if let Some(mon) = ctx.g.monitors.get_mut(mon_idx) {
                    if i < mon.tags.len() {
                        mon.tags[i].name = value.clone();
                    }
                }
            }
        }
    }
}

fn find_resource(resource_str: &str, name: &str) -> Option<String> {
    let full_name = format!("instantwm.{}", name);

    for line in resource_str.lines() {
        let line = line.trim();
        if let Some(colon_pos) = line.find(':') {
            let key = line[..colon_pos].trim();
            let value = line[colon_pos + 1..].trim();

            if key == full_name {
                return Some(value.to_string());
            }
        }
    }

    None
}

pub fn verify_tags_xres(ctx: &mut WmCtx) {
    let mon_count = ctx.g.monitors.len();
    for mon_idx in 0..mon_count {
        for i in 0..9 {
            if let Some(mon) = ctx.g.monitors.get_mut(mon_idx) {
                if i < mon.tags.len() {
                    let len = mon.tags[i].name.len();
                    if len > MAX_TAGLEN - 1 || len == 0 {
                        mon.tags[i].name = "Xres err".to_string();
                    }
                }
            }
        }
    }
}
