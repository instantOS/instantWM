use crate::config::{
    get_bordercolors, get_closebuttoncolors, get_statusbarcolors, get_tagcolors, get_tags,
    get_windowcolors,
};
use crate::globals::{get_globals, get_globals_mut, get_x11};
use crate::types::*;
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

#[derive(Debug, Clone, Copy, PartialEq)]
pub enum ResourceType {
    String,
    Integer,
    Float,
}

#[derive(Debug, Clone)]
pub struct ResourcePref {
    pub name: &'static str,
    pub rtype: ResourceType,
}

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

pub fn resource_load(name: &str, rtype: ResourceType, dst: &mut [u8]) {
    let x11 = get_x11();
    let Some(ref conn) = x11.conn else { return };

    let fullname = format!("instantwm.{}", name);

    let Ok(cookie) = conn.get_property(
        false,
        get_globals().root,
        x11rb::protocol::xproto::AtomEnum::RESOURCE_MANAGER,
        x11rb::protocol::xproto::AtomEnum::STRING,
        0,
        1024,
    ) else {
        return;
    };

    let Ok(reply) = cookie.reply() else { return };

    let resource_str = String::from_utf8_lossy(&reply.value);

    for line in resource_str.lines() {
        let line = line.trim();
        if let Some(colon_pos) = line.find(':') {
            let key = line[..colon_pos].trim();
            let value = line[colon_pos + 1..].trim();

            if key == fullname {
                match rtype {
                    ResourceType::String => {
                        let bytes = value.as_bytes();
                        let len = bytes.len().min(dst.len() - 1);
                        dst[..len].copy_from_slice(&bytes[..len]);
                        dst[len] = 0;
                    }
                    ResourceType::Integer => {
                        if let Ok(val) = value.parse::<u32>() {
                            let val_bytes = val.to_ne_bytes();
                            dst[..4].copy_from_slice(&val_bytes);
                        }
                    }
                    ResourceType::Float => {
                        if let Ok(val) = value.parse::<f32>() {
                            let val_bytes = val.to_ne_bytes();
                            dst[..4].copy_from_slice(&val_bytes);
                        }
                    }
                }
                return;
            }
        }
    }
}

pub fn load_xresources() {
    let x11 = get_x11();
    let Some(ref conn) = x11.conn else { return };

    let Ok(res_cookie) = conn.get_property(
        false,
        get_globals().root,
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

    load_color_resources(&resource_str);
    load_tag_resources(&resource_str);
}

fn load_color_resources(resource_str: &str) {
    let mut globals = get_globals_mut();

    for i in 0..NUM_SCHEMEHOVERTYPES {
        for q in 0..NUM_SCHEMECOLORTYPES {
            for u in 0..NUM_SCHEMEWINDOWTYPES {
                let propname = format!(
                    "{}.{}.win.{}",
                    SCHEME_HOVER_TYPES[i], SCHEME_WINDOW_TYPES[u], SCHEME_COLOR_TYPES[q]
                );

                if let Some(value) = find_resource(resource_str, &propname) {
                    if i < globals.windowcolors.len()
                        && u < globals.windowcolors[i].len()
                        && q < globals.windowcolors[i][u].len()
                    {
                        globals.windowcolors[i][u][q] = Box::leak(value.into_boxed_str());
                    }
                }
            }

            for u in 0..NUM_SCHEMETAGTYPES {
                let propname = format!(
                    "{}.{}.tag.{}",
                    SCHEME_HOVER_TYPES[i], SCHEME_TAG_TYPES[u], SCHEME_COLOR_TYPES[q]
                );

                if let Some(value) = find_resource(resource_str, &propname) {
                    if i < globals.tags.colors.len()
                        && u < globals.tags.colors[i].len()
                        && q < globals.tags.colors[i][u].len()
                    {
                        globals.tags.colors[i][u][q] = Box::leak(value.into_boxed_str());
                    }
                }
            }

            for u in 0..NUM_SCHEMECLOSETYPES {
                let propname = format!(
                    "{}.{}.close.{}",
                    SCHEME_HOVER_TYPES[i], SCHEME_CLOSE_TYPES[u], SCHEME_COLOR_TYPES[q]
                );

                if let Some(value) = find_resource(resource_str, &propname) {
                    if i < globals.closebuttoncolors.len()
                        && u < globals.closebuttoncolors[i].len()
                        && q < globals.closebuttoncolors[i][u].len()
                    {
                        globals.closebuttoncolors[i][u][q] = Box::leak(value.into_boxed_str());
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
            if i < globals.bordercolors.len() {
                globals.bordercolors[i] = Box::leak(value.into_boxed_str());
            }
        }
    }

    let status_names = ["status.fg", "status.bg", "status.detail"];
    for (i, name) in status_names.iter().enumerate() {
        if let Some(value) = find_resource(resource_str, name) {
            if i < globals.statusbarcolors.len() {
                globals.statusbarcolors[i] = Box::leak(value.into_boxed_str());
            }
        }
    }
}

fn load_tag_resources(resource_str: &str) {
    let mut globals = get_globals_mut();

    for i in 0..9 {
        let propname = format!("tag.{}", i + 1);
        if let Some(value) = find_resource(resource_str, &propname) {
            let bytes = value.as_bytes();
            let len = bytes.len().min(15);
            globals.tags.names[i][..len].copy_from_slice(&bytes[..len]);
            globals.tags.names[i][len] = 0;
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

pub fn verify_tags_xres() {
    let mut globals = get_globals_mut();

    for i in 0..9 {
        let len = globals.tags.names[i]
            .iter()
            .position(|&b| b == 0)
            .unwrap_or(globals.tags.names[i].len());
        if len > MAX_TAGLEN - 1 || len == 0 {
            let err = b"Xres err";
            globals.tags.names[i][..err.len()].copy_from_slice(err);
            globals.tags.names[i][err.len()] = 0;
        }
    }
}

pub fn get_resources() -> Vec<ResourcePref> {
    vec![
        ResourcePref {
            name: "borderpx",
            rtype: ResourceType::Integer,
        },
        ResourcePref {
            name: "snap",
            rtype: ResourceType::Integer,
        },
        ResourcePref {
            name: "showbar",
            rtype: ResourceType::Integer,
        },
        ResourcePref {
            name: "topbar",
            rtype: ResourceType::Integer,
        },
        ResourcePref {
            name: "nmaster",
            rtype: ResourceType::Integer,
        },
        ResourcePref {
            name: "mfact",
            rtype: ResourceType::Float,
        },
        ResourcePref {
            name: "focusfollowsmouse",
            rtype: ResourceType::Integer,
        },
        ResourcePref {
            name: "focusfollowsfloatmouse",
            rtype: ResourceType::Integer,
        },
        ResourcePref {
            name: "animated",
            rtype: ResourceType::Integer,
        },
        ResourcePref {
            name: "barheight",
            rtype: ResourceType::Integer,
        },
        ResourcePref {
            name: "systraypinning",
            rtype: ResourceType::Integer,
        },
    ]
}
