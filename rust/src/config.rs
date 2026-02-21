use crate::client::close_win;
use crate::layouts::*;
use crate::mouse::{
    force_resize_mouse, gesture_mouse, move_mouse, resize_aspect_mouse, resize_mouse,
    window_title_mouse_handler, window_title_mouse_handler_right,
};
use crate::toggles::{hide_window, unhide_all};
use crate::types::*;

use crate::animation::{anim_left, anim_right, down_scale_client, up_scale_client};
use crate::bar::toggle_bar;
use crate::client::{kill_client, shut_kill, toggle_fake_fullscreen, zoom};
use crate::commands::{command_prefix, set_special_next};
use crate::floating::{
    center_window, distribute_clients, temp_fullscreen, toggle_floating,
    toggle_fullscreen_overview, toggle_overview,
};
use crate::focus::{direction_focus, focus_last_client, focus_stack, warp_to_focus};
use crate::keyboard::{
    down_key, down_press, focus_nmon, key_resize, space_toggle, up_key, up_press,
};
use crate::layouts::command_layout;
use crate::monitor::{focus_mon, follow_mon};
use crate::mouse::{drag_tag, draw_window, move_resize};
use crate::overlay::{create_overlay, hide_overlay, set_overlay, show_overlay};
use crate::push::{push_down, push_up};
use crate::scratchpad::{
    scratchpad_hide, scratchpad_make, scratchpad_show, scratchpad_status, scratchpad_toggle,
    scratchpad_unmake,
};
use crate::tags::{
    desktop_set, follow_tag, follow_view, last_view, move_left, move_right, name_tag, quit,
    reset_name_tag, shift_view, swap_tags, tag, tag_mon, tag_to_left, tag_to_right, toggle_tag,
    toggle_view, view, view_to_left, view_to_right, win_view,
};
use crate::toggles::{
    alt_tab_free, redraw_win, set_border_width, toggle_alt_tag, toggle_animated,
    toggle_double_draw, toggle_focus_follows_float_mouse, toggle_focus_follows_mouse,
    toggle_locked, toggle_prefix, toggle_show_tags, toggle_sticky,
};
use crate::util::spawn;

pub const BORDERPX: u32 = 3;
pub const MAX_TAGLEN: usize = 16;

pub const MODKEY: u32 = 1 << 6; // Mod4Mask (Super/Windows key)

pub const TAGMASK: u32 = (1 << MAX_TAGS) - 1;

mod colors {
    pub const COL_BG: &str = "#121212";
    pub const COL_TEXT: &str = "#DFDFDF";
    pub const COL_BLACK: &str = "#000000";

    pub const COL_BG_ACCENT: &str = "#384252";
    pub const COL_BG_ACCENT_HOVER: &str = "#4C5564";
    pub const COL_BG_HOVER: &str = "#1C1C1C";

    pub const COL_LIGHT_BLUE: &str = "#89B3F7";
    pub const COL_LIGHT_BLUE_HOVER: &str = "#a1c2f9";
    pub const COL_BLUE: &str = "#536DFE";
    pub const COL_BLUE_HOVER: &str = "#758afe";

    pub const COL_LIGHT_GREEN: &str = "#81c995";
    pub const COL_LIGHT_GREEN_HOVER: &str = "#99d3aa";
    pub const COL_GREEN: &str = "#1e8e3e";
    pub const COL_GREEN_HOVER: &str = "#4ba465";

    pub const COL_LIGHT_YELLOW: &str = "#fdd663";
    pub const COL_LIGHT_YELLOW_HOVER: &str = "#fddd82";
    pub const COL_YELLOW: &str = "#f9ab00";
    pub const COL_YELLOW_HOVER: &str = "#f9bb33";

    pub const COL_LIGHT_RED: &str = "#f28b82";
    pub const COL_LIGHT_RED_HOVER: &str = "#f4a19a";
    pub const COL_RED: &str = "#d93025";
    pub const COL_RED_HOVER: &str = "#e05951";
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SchemeHover {
    NoHover = 0,
    Hover = 1,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SchemeTag {
    Inactive = 0,
    Filled = 1,
    Focus = 2,
    NoFocus = 3,
    Empty = 4,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SchemeWin {
    Focus = 0,
    Normal = 1,
    Minimized = 2,
    Sticky = 3,
    StickyFocus = 4,
    Overlay = 5,
    OverlayFocus = 6,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SchemeClose {
    Normal = 0,
    Locked = 1,
    Fullscreen = 2,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SchemeBorder {
    Normal = 0,
    TileFocus = 1,
    FloatFocus = 2,
    Snap = 3,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ColIndex {
    Fg = 0,
    Bg = 1,
    Detail = 2,
}

pub fn get_tagcolors() -> Vec<Vec<Vec<&'static str>>> {
    use colors::*;
    vec![
        vec![
            vec![COL_TEXT, COL_BG, COL_BG],
            vec![COL_TEXT, COL_BG_ACCENT, COL_LIGHT_BLUE],
            vec![COL_BLACK, COL_LIGHT_GREEN, COL_GREEN],
            vec![COL_BLACK, COL_LIGHT_YELLOW, COL_YELLOW],
            vec![COL_BLACK, COL_LIGHT_RED, COL_RED],
        ],
        vec![
            vec![COL_TEXT, COL_BG_HOVER, COL_BG],
            vec![COL_TEXT, COL_BG_ACCENT_HOVER, COL_LIGHT_BLUE_HOVER],
            vec![COL_BLACK, COL_LIGHT_GREEN_HOVER, COL_GREEN_HOVER],
            vec![COL_BLACK, COL_LIGHT_YELLOW_HOVER, COL_YELLOW_HOVER],
            vec![COL_BLACK, COL_LIGHT_RED_HOVER, COL_RED_HOVER],
        ],
    ]
}

pub fn get_windowcolors() -> Vec<Vec<Vec<&'static str>>> {
    use colors::*;
    vec![
        vec![
            vec![COL_TEXT, COL_BG_ACCENT, COL_LIGHT_BLUE],
            vec![COL_TEXT, COL_BG, COL_BG],
            vec![COL_BG_ACCENT, COL_BG, COL_BG],
            vec![COL_BLACK, COL_LIGHT_YELLOW, COL_YELLOW],
            vec![COL_BLACK, COL_LIGHT_GREEN, COL_GREEN],
            vec![COL_BLACK, COL_LIGHT_YELLOW, COL_YELLOW],
            vec![COL_BLACK, COL_LIGHT_GREEN, COL_GREEN],
        ],
        vec![
            vec![COL_TEXT, COL_BG_ACCENT_HOVER, COL_LIGHT_BLUE_HOVER],
            vec![COL_TEXT, COL_BG_HOVER, COL_BG_HOVER],
            vec![COL_BG_ACCENT_HOVER, COL_BG, COL_BG],
            vec![COL_BLACK, COL_LIGHT_YELLOW_HOVER, COL_YELLOW_HOVER],
            vec![COL_BLACK, COL_LIGHT_GREEN_HOVER, COL_GREEN_HOVER],
            vec![COL_BLACK, COL_LIGHT_YELLOW_HOVER, COL_YELLOW_HOVER],
            vec![COL_BLACK, COL_LIGHT_GREEN_HOVER, COL_GREEN_HOVER],
        ],
    ]
}

pub fn get_closebuttoncolors() -> Vec<Vec<Vec<&'static str>>> {
    use colors::*;
    vec![
        vec![
            vec![COL_TEXT, COL_LIGHT_RED, COL_RED],
            vec![COL_TEXT, COL_LIGHT_YELLOW, COL_YELLOW],
            vec![COL_TEXT, COL_LIGHT_RED, COL_RED],
        ],
        vec![
            vec![COL_TEXT, COL_LIGHT_RED_HOVER, COL_RED_HOVER],
            vec![COL_TEXT, COL_LIGHT_YELLOW_HOVER, COL_YELLOW_HOVER],
            vec![COL_TEXT, COL_LIGHT_RED_HOVER, COL_RED_HOVER],
        ],
    ]
}

pub fn get_bordercolors() -> Vec<&'static str> {
    use colors::*;
    vec![
        COL_BG_ACCENT,
        COL_LIGHT_BLUE,
        COL_LIGHT_GREEN,
        COL_LIGHT_YELLOW,
    ]
}

pub fn get_statusbarcolors() -> Vec<&'static str> {
    use colors::*;
    vec![COL_TEXT, COL_BG, COL_BG]
}

pub fn get_tags_default() -> [&'static str; MAX_TAGS] {
    [
        "1", "2", "3", "4", "5", "6", "7", "8", "9", "10", "11", "12", "13", "14", "15", "16",
        "17", "18", "19", "20", "s",
    ]
}

pub fn get_tags() -> [[u8; MAX_TAGLEN]; MAX_TAGS] {
    let default = get_tags_default();
    let mut tags = [[0u8; MAX_TAGLEN]; MAX_TAGS];
    for (i, &tag) in default.iter().enumerate() {
        let bytes = tag.as_bytes();
        let len = bytes.len().min(MAX_TAGLEN);
        tags[i][..len].copy_from_slice(&bytes[..len]);
    }
    tags
}

pub fn get_tagsalt() -> Vec<&'static str> {
    vec!["", "{}", "$", "", "", "", "", "", ""]
}

pub fn get_fonts() -> Vec<&'static str> {
    vec!["Inter-Regular:size=12", "Fira Code Nerd Font:size=12"]
}

pub fn get_layouts() -> Vec<&'static dyn Layout> {
    vec![
        &TILE_LAYOUT,
        &GRID_LAYOUT,
        &FLOATING_LAYOUT,
        &MONOCLE_LAYOUT,
        &VERT_LAYOUT,
        &DECK_LAYOUT,
        &OVERVIEW_LAYOUT,
        &BSTACK_LAYOUT,
        &HORIZ_LAYOUT,
    ]
}

pub fn get_rules() -> Vec<Rule> {
    const SCRATCHPAD_CLASS: &str = "scratchpad_default";
    vec![
        Rule {
            class: Some("Pavucontrol"),
            instance: None,
            title: None,
            tags: 0,
            isfloating: RuleFloat::Float,
            monitor: -1,
        },
        Rule {
            class: Some("Onboard"),
            instance: None,
            title: None,
            tags: 0,
            isfloating: RuleFloat::Float,
            monitor: -1,
        },
        Rule {
            class: Some("floatmenu"),
            instance: None,
            title: None,
            tags: 0,
            isfloating: RuleFloat::Float,
            monitor: -1,
        },
        Rule {
            class: Some("Welcome.py"),
            instance: None,
            title: None,
            tags: 0,
            isfloating: RuleFloat::Float,
            monitor: -1,
        },
        Rule {
            class: Some("Pamac-installer"),
            instance: None,
            title: None,
            tags: 0,
            isfloating: RuleFloat::Float,
            monitor: -1,
        },
        Rule {
            class: Some("xpad"),
            instance: None,
            title: None,
            tags: 0,
            isfloating: RuleFloat::Float,
            monitor: -1,
        },
        Rule {
            class: Some("Guake"),
            instance: None,
            title: None,
            tags: 0,
            isfloating: RuleFloat::Float,
            monitor: -1,
        },
        Rule {
            class: Some("instantfloat"),
            instance: None,
            title: None,
            tags: 0,
            isfloating: RuleFloat::FloatCenter,
            monitor: -1,
        },
        Rule {
            class: Some(SCRATCHPAD_CLASS),
            instance: None,
            title: None,
            tags: 0,
            isfloating: RuleFloat::Scratchpad,
            monitor: -1,
        },
        Rule {
            class: Some("kdeconnect.daemon"),
            instance: None,
            title: None,
            tags: 0,
            isfloating: RuleFloat::FloatFullscreen,
            monitor: -1,
        },
        Rule {
            class: Some("Panther"),
            instance: None,
            title: None,
            tags: 0,
            isfloating: RuleFloat::FloatFullscreen,
            monitor: -1,
        },
        Rule {
            class: Some("org-wellkord-globonote-Main"),
            instance: None,
            title: None,
            tags: 0,
            isfloating: RuleFloat::Float,
            monitor: -1,
        },
        Rule {
            class: Some("Peek"),
            instance: None,
            title: None,
            tags: 0,
            isfloating: RuleFloat::Float,
            monitor: -1,
        },
    ]
}

pub const SCRATCHPAD_CLASS: &str = "scratchpad_default";

pub mod commands {
    pub const INSTANTMENU_CMD: &[&str] = &["instantmenu_run"];
    pub const CLIPMENU_CMD: &[&str] = &["instantclipmenu"];
    pub const SMART_CMD: &[&str] = &["instantmenu_smartrun"];
    pub const INSTANTMENU_ST_CMD: &[&str] = &["instantmenu_run_st"];
    pub const TERM_CMD: &[&str] = &[".config/instantos/default/terminal"];
    pub const TERM_SCRATCH_CMD: &[&str] = &[
        ".config/instantos/default/terminal",
        "-c",
        super::SCRATCHPAD_CLASS,
    ];
    pub const QUICKMENU_CMD: &[&str] = &["quickmenu"];
    pub const INSTANTASSIST_CMD: &[&str] = &["instantassist"];
    pub const INSTANTREPEAT_CMD: &[&str] = &["instantrepeat"];
    pub const INSTANTPACMAN_CMD: &[&str] = &["instantpacman"];
    pub const INSTANTSHARE_CMD: &[&str] = &["instantshare", "snap"];
    pub const NAUTILUS_CMD: &[&str] = &[".config/instantos/default/filemanager"];
    pub const SLOCK_CMD: &[&str] = &[".config/instantos/default/lockscreen"];
    pub const ONEKEYLOCK_CMD: &[&str] = &["ilock", "-o"];
    pub const LANGSWITCH_CMD: &[&str] = &["ilayout"];
    pub const OSLOCK_CMD: &[&str] = &["instantlock", "-o"];
    pub const HELP_CMD: &[&str] = &["instanthotkeys", "gui"];
    pub const SEARCH_CMD: &[&str] = &["instantsearch"];
    pub const KEYLAYOUTSWITCH_CMD: &[&str] = &["instantkeyswitch"];
    pub const ISWITCH_CMD: &[&str] = &["iswitch"];
    pub const INSTANTSWITCH_CMD: &[&str] = &[
        "rofi",
        "-show",
        "window",
        "-kb-row-down",
        "Alt+Tab,Down",
        "-kb-row-up",
        "Alt+Ctrl+Tab,Up",
        "-kb-accept-entry",
        "!Alt_L,!Alt+Tab,Return",
        "-me-select-entry",
        "",
        "-me-accept-entry",
        "MousePrimary",
    ];
    pub const CARETINSTANTSWITCH_CMD: &[&str] = &[
        "rofi",
        "-show",
        "window",
        "-kb-row-down",
        "Alt+Tab,Down",
        "-kb-row-up",
        "Alt+Ctrl+Tab,Up",
        "-kb-accept-entry",
        "!Alt_L,!Alt+Tab,Return",
        "-me-select-entry",
        "",
        "-me-accept-entry",
        "MousePrimary",
        "-theme",
        "/usr/share/instantdotfiles/rootconfig/rofi/appmenu.rasi",
    ];
    pub const INSTANTSKIPPY_CMD: &[&str] = &["instantskippy"];
    pub const ONBOARD_CMD: &[&str] = &["onboard"];
    pub const INSTANTSHUTDOWN_CMD: &[&str] = &["instantshutdown"];
    pub const SYSTEMMONITOR_CMD: &[&str] = &[".config/instantos/default/systemmonitor"];
    pub const NOTIFY_CMD: &[&str] = &["instantnotify"];
    pub const YAZI_CMD: &[&str] = &[".config/instantos/default/termfilemanager"];
    pub const PANTHER_CMD: &[&str] = &[".config/instantos/default/appmenu"];
    pub const CONTROLCENTER_CMD: &[&str] = &["ins settings --gui"];
    pub const DISPLAY_CMD: &[&str] = &["instantdisper"];
    pub const PAVUCONTROL_CMD: &[&str] = &["pavucontrol"];
    pub const INSTANTSETTINGS_CMD: &[&str] = &["ins", "settings", "--gui"];
    pub const CODE_CMD: &[&str] = &["instantutils open graphicaleditor"];
    pub const STARTMENU_CMD: &[&str] = &["instantstartmenu"];
    pub const SCROT_CMD: &[&str] = &["/usr/share/instantassist/assists/s/s.sh"];
    pub const FSCROT_CMD: &[&str] = &["/usr/share/instantassist/assists/s/m.sh"];
    pub const CLIPSCROT_CMD: &[&str] = &["/usr/share/instantassist/assists/s/c.sh"];
    pub const FCLIPSCROT_CMD: &[&str] = &["/usr/share/instantassist/assists/s/f.sh"];
    pub const FIREFOX_CMD: &[&str] = &[".config/instantos/default/browser"];
    pub const EDITOR_CMD: &[&str] = &[".config/instantos/default/editor"];
    pub const PLAYER_NEXT_CMD: &[&str] = &["playerctl", "next"];
    pub const PLAYER_PREVIOUS_CMD: &[&str] = &["playerctl", "previous"];
    pub const PLAYER_PAUSE_CMD: &[&str] = &["playerctl", "play-pause"];
    pub const SPOTICLI_CMD: &[&str] = &["spoticli", "m"];
    pub const UPVOL_CMD: &[&str] = &["/usr/share/instantassist/utils/p.sh", "+"];
    pub const DOWNVOL_CMD: &[&str] = &["/usr/share/instantassist/utils/p.sh", "-"];
    pub const MUTEVOL_CMD: &[&str] = &["/usr/share/instantassist/utils/p.sh", "m"];
    pub const UPBRIGHT_CMD: &[&str] = &["/usr/share/instantassist/utils/b.sh", "+"];
    pub const DOWNBRIGHT_CMD: &[&str] = &["/usr/share/instantassist/utils/b.sh", "-"];
}

pub use commands::*;

pub fn get_external_commands() -> ExternalCommands {
    ExternalCommands {
        instantmenucmd: commands::INSTANTMENU_CMD.to_vec(),
        clipmenucmd: commands::CLIPMENU_CMD.to_vec(),
        smartcmd: commands::SMART_CMD.to_vec(),
        instantmenustcmd: commands::INSTANTMENU_ST_CMD.to_vec(),
        termcmd: commands::TERM_CMD.to_vec(),
        termscratchcmd: commands::TERM_SCRATCH_CMD.to_vec(),
        quickmenucmd: commands::QUICKMENU_CMD.to_vec(),
        instantassistcmd: commands::INSTANTASSIST_CMD.to_vec(),
        instantrepeatcmd: commands::INSTANTREPEAT_CMD.to_vec(),
        instantpacmancmd: commands::INSTANTPACMAN_CMD.to_vec(),
        instantsharecmd: commands::INSTANTSHARE_CMD.to_vec(),
        nautiluscmd: commands::NAUTILUS_CMD.to_vec(),
        slockcmd: commands::SLOCK_CMD.to_vec(),
        onekeylock: commands::ONEKEYLOCK_CMD.to_vec(),
        langswitchcmd: commands::LANGSWITCH_CMD.to_vec(),
        oslockcmd: commands::OSLOCK_CMD.to_vec(),
        helpcmd: commands::HELP_CMD.to_vec(),
        searchcmd: commands::SEARCH_CMD.to_vec(),
        keylayoutswitchcmd: commands::KEYLAYOUTSWITCH_CMD.to_vec(),
        iswitchcmd: commands::ISWITCH_CMD.to_vec(),
        instantswitchcmd: commands::INSTANTSWITCH_CMD.to_vec(),
        caretinstantswitchcmd: commands::CARETINSTANTSWITCH_CMD.to_vec(),
        instantskippycmd: commands::INSTANTSKIPPY_CMD.to_vec(),
        onboardcmd: commands::ONBOARD_CMD.to_vec(),
        instantshutdowncmd: commands::INSTANTSHUTDOWN_CMD.to_vec(),
        systemmonitorcmd: commands::SYSTEMMONITOR_CMD.to_vec(),
        notifycmd: commands::NOTIFY_CMD.to_vec(),
        yazicmd: commands::YAZI_CMD.to_vec(),
        panther: commands::PANTHER_CMD.to_vec(),
        controlcentercmd: commands::CONTROLCENTER_CMD.to_vec(),
        displaycmd: commands::DISPLAY_CMD.to_vec(),
        pavucontrol: commands::PAVUCONTROL_CMD.to_vec(),
        instantsettings: commands::INSTANTSETTINGS_CMD.to_vec(),
        codecmd: commands::CODE_CMD.to_vec(),
        startmenucmd: commands::STARTMENU_CMD.to_vec(),
        scrotcmd: commands::SCROT_CMD.to_vec(),
        fscrotcmd: commands::FSCROT_CMD.to_vec(),
        clipscrotcmd: commands::CLIPSCROT_CMD.to_vec(),
        fclipscrotcmd: commands::FCLIPSCROT_CMD.to_vec(),
        firefoxcmd: commands::FIREFOX_CMD.to_vec(),
        editorcmd: commands::EDITOR_CMD.to_vec(),
        playernext: commands::PLAYER_NEXT_CMD.to_vec(),
        playerprevious: commands::PLAYER_PREVIOUS_CMD.to_vec(),
        playerpause: commands::PLAYER_PAUSE_CMD.to_vec(),
        spoticli: commands::SPOTICLI_CMD.to_vec(),
        upvol: commands::UPVOL_CMD.to_vec(),
        downvol: commands::DOWNVOL_CMD.to_vec(),
        mutevol: commands::MUTEVOL_CMD.to_vec(),
        upbright: commands::UPBRIGHT_CMD.to_vec(),
        downbright: commands::DOWNBRIGHT_CMD.to_vec(),
    }
}

#[derive(Debug, Clone)]
pub struct ExternalCommands {
    pub instantmenucmd: Vec<&'static str>,
    pub clipmenucmd: Vec<&'static str>,
    pub smartcmd: Vec<&'static str>,
    pub instantmenustcmd: Vec<&'static str>,
    pub termcmd: Vec<&'static str>,
    pub termscratchcmd: Vec<&'static str>,
    pub quickmenucmd: Vec<&'static str>,
    pub instantassistcmd: Vec<&'static str>,
    pub instantrepeatcmd: Vec<&'static str>,
    pub instantpacmancmd: Vec<&'static str>,
    pub instantsharecmd: Vec<&'static str>,
    pub nautiluscmd: Vec<&'static str>,
    pub slockcmd: Vec<&'static str>,
    pub onekeylock: Vec<&'static str>,
    pub langswitchcmd: Vec<&'static str>,
    pub oslockcmd: Vec<&'static str>,
    pub helpcmd: Vec<&'static str>,
    pub searchcmd: Vec<&'static str>,
    pub keylayoutswitchcmd: Vec<&'static str>,
    pub iswitchcmd: Vec<&'static str>,
    pub instantswitchcmd: Vec<&'static str>,
    pub caretinstantswitchcmd: Vec<&'static str>,
    pub instantskippycmd: Vec<&'static str>,
    pub onboardcmd: Vec<&'static str>,
    pub instantshutdowncmd: Vec<&'static str>,
    pub systemmonitorcmd: Vec<&'static str>,
    pub notifycmd: Vec<&'static str>,
    pub yazicmd: Vec<&'static str>,
    pub panther: Vec<&'static str>,
    pub controlcentercmd: Vec<&'static str>,
    pub displaycmd: Vec<&'static str>,
    pub pavucontrol: Vec<&'static str>,
    pub instantsettings: Vec<&'static str>,
    pub codecmd: Vec<&'static str>,
    pub startmenucmd: Vec<&'static str>,
    pub scrotcmd: Vec<&'static str>,
    pub fscrotcmd: Vec<&'static str>,
    pub clipscrotcmd: Vec<&'static str>,
    pub fclipscrotcmd: Vec<&'static str>,
    pub firefoxcmd: Vec<&'static str>,
    pub editorcmd: Vec<&'static str>,
    pub playernext: Vec<&'static str>,
    pub playerprevious: Vec<&'static str>,
    pub playerpause: Vec<&'static str>,
    pub spoticli: Vec<&'static str>,
    pub upvol: Vec<&'static str>,
    pub downvol: Vec<&'static str>,
    pub mutevol: Vec<&'static str>,
    pub upbright: Vec<&'static str>,
    pub downbright: Vec<&'static str>,
}

pub mod xk {
    pub const XK_BackSpace: u32 = 0xFF08;
    pub const XK_Tab: u32 = 0xFF09;
    pub const XK_Return: u32 = 0xFF0D;
    pub const XK_Escape: u32 = 0xFF1B;
    pub const XK_Delete: u32 = 0xFFFF;
    pub const XK_Home: u32 = 0xFF50;
    pub const XK_Left: u32 = 0xFF51;
    pub const XK_Up: u32 = 0xFF52;
    pub const XK_Right: u32 = 0xFF53;
    pub const XK_Down: u32 = 0xFF54;
    pub const XK_Page_Up: u32 = 0xFF55;
    pub const XK_Page_Down: u32 = 0xFF56;
    pub const XK_End: u32 = 0xFF57;
    pub const XK_Insert: u32 = 0xFF63;
    pub const XK_F1: u32 = 0xFFBE;
    pub const XK_F2: u32 = 0xFFBF;
    pub const XK_F3: u32 = 0xFFC0;
    pub const XK_F4: u32 = 0xFFC1;
    pub const XK_F5: u32 = 0xFFC2;
    pub const XK_F6: u32 = 0xFFC3;
    pub const XK_F7: u32 = 0xFFC4;
    pub const XK_F8: u32 = 0xFFC5;
    pub const XK_F9: u32 = 0xFFC6;
    pub const XK_F10: u32 = 0xFFC7;
    pub const XK_F11: u32 = 0xFFC8;
    pub const XK_F12: u32 = 0xFFC9;
    pub const XK_space: u32 = 0x0020;
    pub const XK_exclam: u32 = 0x0021;
    pub const XK_quotedbl: u32 = 0x0022;
    pub const XK_numbersign: u32 = 0x0023;
    pub const XK_dollar: u32 = 0x0024;
    pub const XK_percent: u32 = 0x0025;
    pub const XK_ampersand: u32 = 0x0026;
    pub const XK_apostrophe: u32 = 0x0027;
    pub const XK_parenleft: u32 = 0x0028;
    pub const XK_parenright: u32 = 0x0029;
    pub const XK_asterisk: u32 = 0x002a;
    pub const XK_plus: u32 = 0x002b;
    pub const XK_comma: u32 = 0x002c;
    pub const XK_minus: u32 = 0x002d;
    pub const XK_period: u32 = 0x002e;
    pub const XK_slash: u32 = 0x002f;
    pub const XK_0: u32 = 0x0030;
    pub const XK_1: u32 = 0x0031;
    pub const XK_2: u32 = 0x0032;
    pub const XK_3: u32 = 0x0033;
    pub const XK_4: u32 = 0x0034;
    pub const XK_5: u32 = 0x0035;
    pub const XK_6: u32 = 0x0036;
    pub const XK_7: u32 = 0x0037;
    pub const XK_8: u32 = 0x0038;
    pub const XK_9: u32 = 0x0039;
    pub const XK_colon: u32 = 0x003a;
    pub const XK_semicolon: u32 = 0x003b;
    pub const XK_less: u32 = 0x003c;
    pub const XK_equal: u32 = 0x003d;
    pub const XK_greater: u32 = 0x003e;
    pub const XK_question: u32 = 0x003f;
    pub const XK_at: u32 = 0x0040;
    pub const XK_A: u32 = 0x0041;
    pub const XK_B: u32 = 0x0042;
    pub const XK_C: u32 = 0x0043;
    pub const XK_D: u32 = 0x0044;
    pub const XK_E: u32 = 0x0045;
    pub const XK_F: u32 = 0x0046;
    pub const XK_G: u32 = 0x0047;
    pub const XK_H: u32 = 0x0048;
    pub const XK_I: u32 = 0x0049;
    pub const XK_J: u32 = 0x004a;
    pub const XK_K: u32 = 0x004b;
    pub const XK_L: u32 = 0x004c;
    pub const XK_M: u32 = 0x004d;
    pub const XK_N: u32 = 0x004e;
    pub const XK_O: u32 = 0x004f;
    pub const XK_P: u32 = 0x0050;
    pub const XK_Q: u32 = 0x0051;
    pub const XK_R: u32 = 0x0052;
    pub const XK_S: u32 = 0x0053;
    pub const XK_T: u32 = 0x0054;
    pub const XK_U: u32 = 0x0055;
    pub const XK_V: u32 = 0x0056;
    pub const XK_W: u32 = 0x0057;
    pub const XK_X: u32 = 0x0058;
    pub const XK_Y: u32 = 0x0059;
    pub const XK_Z: u32 = 0x005a;
    pub const XK_bracketleft: u32 = 0x005b;
    pub const XK_backslash: u32 = 0x005c;
    pub const XK_bracketright: u32 = 0x005d;
    pub const XK_asciicircum: u32 = 0x005e;
    pub const XK_underscore: u32 = 0x005f;
    pub const XK_grave: u32 = 0x0060;
    pub const XK_a: u32 = 0x0061;
    pub const XK_b: u32 = 0x0062;
    pub const XK_c: u32 = 0x0063;
    pub const XK_d: u32 = 0x0064;
    pub const XK_e: u32 = 0x0065;
    pub const XK_f: u32 = 0x0066;
    pub const XK_g: u32 = 0x0067;
    pub const XK_h: u32 = 0x0068;
    pub const XK_i: u32 = 0x0069;
    pub const XK_j: u32 = 0x006a;
    pub const XK_k: u32 = 0x006b;
    pub const XK_l: u32 = 0x006c;
    pub const XK_m: u32 = 0x006d;
    pub const XK_n: u32 = 0x006e;
    pub const XK_o: u32 = 0x006f;
    pub const XK_p: u32 = 0x0070;
    pub const XK_q: u32 = 0x0071;
    pub const XK_r: u32 = 0x0072;
    pub const XK_s: u32 = 0x0073;
    pub const XK_t: u32 = 0x0074;
    pub const XK_u: u32 = 0x0075;
    pub const XK_v: u32 = 0x0076;
    pub const XK_w: u32 = 0x0077;
    pub const XK_x: u32 = 0x0078;
    pub const XK_y: u32 = 0x0079;
    pub const XK_z: u32 = 0x007a;
    pub const XK_Print: u32 = 0xFF61;
    pub const XK_dead_circumflex: u32 = 0xFE52;

    pub const XF86XK_MonBrightnessUp: u32 = 0x1008FF02;
    pub const XF86XK_MonBrightnessDown: u32 = 0x1008FF03;
    pub const XF86XK_AudioLowerVolume: u32 = 0x1008FF11;
    pub const XF86XK_AudioMute: u32 = 0x1008FF12;
    pub const XF86XK_AudioRaiseVolume: u32 = 0x1008FF13;
    pub const XF86XK_AudioPlay: u32 = 0x1008FF14;
    pub const XF86XK_AudioPause: u32 = 0x1008FF15;
    pub const XF86XK_AudioNext: u32 = 0x1008FF17;
    pub const XF86XK_AudioPrev: u32 = 0x1008FF16;
}

const CONTROL: u32 = 1 << 2;
const SHIFT: u32 = 1 << 0;
const MOD1: u32 = 1 << 3;

fn tagkeys(keysym: u32, tag_idx: usize) -> [Key; 6] {
    [
        Key {
            mod_mask: MODKEY,
            keysym: keysym,
            func: Some(view),
            arg: Arg {
                ui: 1 << tag_idx,
                ..Default::default()
            },
        },
        Key {
            mod_mask: MODKEY | CONTROL,
            keysym: keysym,
            func: Some(toggle_view),
            arg: Arg {
                ui: 1 << tag_idx,
                ..Default::default()
            },
        },
        Key {
            mod_mask: MODKEY | SHIFT,
            keysym: keysym,
            func: Some(tag),
            arg: Arg {
                ui: 1 << tag_idx,
                ..Default::default()
            },
        },
        Key {
            mod_mask: MODKEY | MOD1,
            keysym: keysym,
            func: Some(follow_tag),
            arg: Arg {
                ui: 1 << tag_idx,
                ..Default::default()
            },
        },
        Key {
            mod_mask: MODKEY | CONTROL | SHIFT,
            keysym: keysym,
            func: Some(toggle_tag),
            arg: Arg {
                ui: 1 << tag_idx,
                ..Default::default()
            },
        },
        Key {
            mod_mask: MODKEY | MOD1 | SHIFT,
            keysym: keysym,
            func: Some(swap_tags),
            arg: Arg {
                ui: 1 << tag_idx,
                ..Default::default()
            },
        },
    ]
}

pub fn get_keys() -> Vec<Key> {
    use xk::*;
    let mut keys = Vec::new();

    keys.extend_from_slice(&[
        Key {
            mod_mask: MODKEY | MOD1,
            keysym: XK_j,
            func: Some(key_resize),
            arg: Arg {
                i: 0,
                ..Default::default()
            },
        },
        Key {
            mod_mask: MODKEY | MOD1,
            keysym: XK_k,
            func: Some(key_resize),
            arg: Arg {
                i: 1,
                ..Default::default()
            },
        },
        Key {
            mod_mask: MODKEY | MOD1,
            keysym: XK_l,
            func: Some(key_resize),
            arg: Arg {
                i: 2,
                ..Default::default()
            },
        },
        Key {
            mod_mask: MODKEY | MOD1,
            keysym: XK_h,
            func: Some(key_resize),
            arg: Arg {
                i: 3,
                ..Default::default()
            },
        },
        Key {
            mod_mask: MODKEY | CONTROL,
            keysym: XK_d,
            func: Some(distribute_clients),
            arg: Arg::default(),
        },
        Key {
            mod_mask: MODKEY | SHIFT,
            keysym: XK_d,
            func: Some(draw_window),
            arg: Arg::default(),
        },
        Key {
            mod_mask: MODKEY | SHIFT,
            keysym: XK_Escape,
            func: Some(spawn),
            arg: Arg {
                v: Some(CMD_SYSTEMMONITOR),
                ..Default::default()
            },
        },
        Key {
            mod_mask: MODKEY,
            keysym: XK_r,
            func: Some(spawn),
            arg: Arg {
                v: Some(CMD_YAZI),
                ..Default::default()
            },
        },
        Key {
            mod_mask: MODKEY | CONTROL | MOD1,
            keysym: XK_r,
            func: Some(redraw_win),
            arg: Arg::default(),
        },
        Key {
            mod_mask: MODKEY,
            keysym: XK_n,
            func: Some(spawn),
            arg: Arg {
                v: Some(CMD_NAUTILUS),
                ..Default::default()
            },
        },
        Key {
            mod_mask: MODKEY | CONTROL,
            keysym: XK_q,
            func: Some(spawn),
            arg: Arg {
                v: Some(CMD_INSTANTSHUTDOWN),
                ..Default::default()
            },
        },
        Key {
            mod_mask: MODKEY,
            keysym: XK_y,
            func: Some(spawn),
            arg: Arg {
                v: Some(CMD_PANTHER),
                ..Default::default()
            },
        },
        Key {
            mod_mask: MODKEY,
            keysym: XK_a,
            func: Some(spawn),
            arg: Arg {
                v: Some(CMD_INSTANTASSIST),
                ..Default::default()
            },
        },
        Key {
            mod_mask: MODKEY | SHIFT,
            keysym: XK_a,
            func: Some(spawn),
            arg: Arg {
                v: Some(CMD_INSTANTREPEAT),
                ..Default::default()
            },
        },
        Key {
            mod_mask: MODKEY | CONTROL,
            keysym: XK_i,
            func: Some(spawn),
            arg: Arg {
                v: Some(CMD_INSTANTPACMAN),
                ..Default::default()
            },
        },
        Key {
            mod_mask: MODKEY | SHIFT,
            keysym: XK_i,
            func: Some(spawn),
            arg: Arg {
                v: Some(CMD_INSTANTSHARE),
                ..Default::default()
            },
        },
        Key {
            mod_mask: MODKEY,
            keysym: XK_w,
            func: Some(set_overlay),
            arg: Arg::default(),
        },
        Key {
            mod_mask: MODKEY | CONTROL,
            keysym: XK_w,
            func: Some(create_overlay),
            arg: Arg::default(),
        },
        Key {
            mod_mask: MODKEY,
            keysym: XK_g,
            func: Some(spawn),
            arg: Arg {
                v: Some(CMD_NOTIFY),
                ..Default::default()
            },
        },
        Key {
            mod_mask: MODKEY | CONTROL,
            keysym: XK_space,
            func: Some(spawn),
            arg: Arg {
                v: Some(CMD_INSTANTMENU),
                ..Default::default()
            },
        },
        Key {
            mod_mask: MODKEY | SHIFT,
            keysym: XK_v,
            func: Some(spawn),
            arg: Arg {
                v: Some(CMD_CLIPMENU),
                ..Default::default()
            },
        },
        Key {
            mod_mask: MODKEY,
            keysym: XK_space,
            func: Some(spawn),
            arg: Arg {
                v: Some(CMD_SMART),
                ..Default::default()
            },
        },
        Key {
            mod_mask: MODKEY,
            keysym: XK_minus,
            func: Some(spawn),
            arg: Arg {
                v: Some(CMD_INSTANTMENU_ST),
                ..Default::default()
            },
        },
        Key {
            mod_mask: MODKEY,
            keysym: XK_x,
            func: Some(spawn),
            arg: Arg {
                v: Some(CMD_INSTANTSWITCH),
                ..Default::default()
            },
        },
        Key {
            mod_mask: MOD1,
            keysym: XK_Tab,
            func: Some(spawn),
            arg: Arg {
                v: Some(CMD_ISWITCH),
                ..Default::default()
            },
        },
        Key {
            mod_mask: MODKEY | MOD1 | CONTROL | SHIFT,
            keysym: XK_Tab,
            func: Some(alt_tab_free),
            arg: Arg::default(),
        },
        Key {
            mod_mask: MODKEY,
            keysym: XK_dead_circumflex,
            func: Some(spawn),
            arg: Arg {
                v: Some(CMD_CARETINSTANTSWITCH),
                ..Default::default()
            },
        },
        Key {
            mod_mask: MODKEY | CONTROL,
            keysym: XK_l,
            func: Some(spawn),
            arg: Arg {
                v: Some(CMD_SLOCK),
                ..Default::default()
            },
        },
        Key {
            mod_mask: MODKEY | CONTROL | SHIFT,
            keysym: XK_l,
            func: Some(spawn),
            arg: Arg {
                v: Some(CMD_ONEKEYLOCK),
                ..Default::default()
            },
        },
        Key {
            mod_mask: MODKEY | CONTROL,
            keysym: XK_h,
            func: Some(hide_window),
            arg: Arg::default(),
        },
        Key {
            mod_mask: MODKEY | MOD1 | CONTROL,
            keysym: XK_h,
            func: Some(unhide_all),
            arg: Arg::default(),
        },
        Key {
            mod_mask: MODKEY | MOD1 | CONTROL,
            keysym: XK_l,
            func: Some(spawn),
            arg: Arg {
                v: Some(CMD_LANGSWITCH),
                ..Default::default()
            },
        },
        Key {
            mod_mask: MODKEY,
            keysym: XK_Return,
            func: Some(spawn),
            arg: Arg {
                v: Some(CMD_TERM),
                ..Default::default()
            },
        },
        Key {
            mod_mask: MODKEY,
            keysym: XK_v,
            func: Some(spawn),
            arg: Arg {
                v: Some(CMD_QUICKMENU),
                ..Default::default()
            },
        },
        Key {
            mod_mask: MODKEY,
            keysym: XK_b,
            func: Some(toggle_bar),
            arg: Arg::default(),
        },
        Key {
            mod_mask: MODKEY,
            keysym: XK_j,
            func: Some(focus_stack),
            arg: Arg {
                i: 1,
                ..Default::default()
            },
        },
        Key {
            mod_mask: MODKEY,
            keysym: XK_Down,
            func: Some(down_key),
            arg: Arg {
                i: 1,
                ..Default::default()
            },
        },
        Key {
            mod_mask: MODKEY | SHIFT,
            keysym: XK_Down,
            func: Some(down_press),
            arg: Arg::default(),
        },
        Key {
            mod_mask: MODKEY,
            keysym: XK_k,
            func: Some(focus_stack),
            arg: Arg {
                i: -1,
                ..Default::default()
            },
        },
        Key {
            mod_mask: MODKEY,
            keysym: XK_Up,
            func: Some(up_key),
            arg: Arg {
                i: -1,
                ..Default::default()
            },
        },
        Key {
            mod_mask: MODKEY | SHIFT,
            keysym: XK_Up,
            func: Some(up_press),
            arg: Arg::default(),
        },
        Key {
            mod_mask: MODKEY | CONTROL,
            keysym: XK_j,
            func: Some(push_down),
            arg: Arg::default(),
        },
        Key {
            mod_mask: MODKEY | CONTROL,
            keysym: XK_k,
            func: Some(push_up),
            arg: Arg::default(),
        },
        Key {
            mod_mask: MODKEY | MOD1,
            keysym: XK_s,
            func: Some(toggle_alt_tag),
            arg: Arg {
                ui: 2,
                ..Default::default()
            },
        },
        Key {
            mod_mask: MODKEY | SHIFT | MOD1,
            keysym: XK_s,
            func: Some(toggle_animated),
            arg: Arg {
                ui: 2,
                ..Default::default()
            },
        },
        Key {
            mod_mask: MODKEY | CONTROL,
            keysym: XK_s,
            func: Some(toggle_sticky),
            arg: Arg::default(),
        },
        Key {
            mod_mask: MODKEY | SHIFT,
            keysym: XK_s,
            func: Some(scratchpad_make),
            arg: Arg {
                v: Some(CMD_DEFAULT),
                ..Default::default()
            },
        },
        Key {
            mod_mask: MODKEY,
            keysym: XK_s,
            func: Some(scratchpad_toggle),
            arg: Arg {
                v: Some(CMD_DEFAULT),
                ..Default::default()
            },
        },
        Key {
            mod_mask: MODKEY | SHIFT,
            keysym: XK_f,
            func: Some(toggle_fake_fullscreen),
            arg: Arg::default(),
        },
        Key {
            mod_mask: MODKEY | CONTROL,
            keysym: XK_f,
            func: Some(temp_fullscreen),
            arg: Arg::default(),
        },
        Key {
            mod_mask: MODKEY | MOD1,
            keysym: XK_f,
            func: Some(spawn),
            arg: Arg {
                v: Some(CMD_SEARCH),
                ..Default::default()
            },
        },
        Key {
            mod_mask: MODKEY | MOD1,
            keysym: XK_space,
            func: Some(spawn),
            arg: Arg {
                v: Some(CMD_KEYLAYOUTSWITCH),
                ..Default::default()
            },
        },
        Key {
            mod_mask: MODKEY | SHIFT | MOD1,
            keysym: XK_d,
            func: Some(toggle_double_draw),
            arg: Arg::default(),
        },
        Key {
            mod_mask: MODKEY | SHIFT,
            keysym: XK_w,
            func: Some(warp_to_focus),
            arg: Arg::default(),
        },
        Key {
            mod_mask: MODKEY | MOD1,
            keysym: XK_w,
            func: Some(center_window),
            arg: Arg::default(),
        },
        Key {
            mod_mask: MODKEY | SHIFT | CONTROL,
            keysym: XK_s,
            func: Some(toggle_show_tags),
            arg: Arg {
                ui: 2,
                ..Default::default()
            },
        },
        Key {
            mod_mask: MODKEY,
            keysym: XK_i,
            func: Some(inc_nmaster),
            arg: Arg {
                i: 1,
                ..Default::default()
            },
        },
        Key {
            mod_mask: MODKEY,
            keysym: XK_d,
            func: Some(inc_nmaster),
            arg: Arg {
                i: -1,
                ..Default::default()
            },
        },
        Key {
            mod_mask: MODKEY,
            keysym: XK_h,
            func: Some(set_mfact),
            arg: Arg {
                f: -0.05,
                ..Default::default()
            },
        },
        Key {
            mod_mask: MODKEY,
            keysym: XK_l,
            func: Some(set_mfact),
            arg: Arg {
                f: 0.05,
                ..Default::default()
            },
        },
        Key {
            mod_mask: MODKEY | SHIFT,
            keysym: XK_Return,
            func: Some(zoom),
            arg: Arg::default(),
        },
        Key {
            mod_mask: MODKEY,
            keysym: XK_Tab,
            func: Some(last_view),
            arg: Arg::default(),
        },
        Key {
            mod_mask: MODKEY | SHIFT,
            keysym: XK_Tab,
            func: Some(focus_last_client),
            arg: Arg::default(),
        },
        Key {
            mod_mask: MODKEY | MOD1,
            keysym: XK_Tab,
            func: Some(follow_view),
            arg: Arg::default(),
        },
        Key {
            mod_mask: MODKEY,
            keysym: XK_q,
            func: Some(shut_kill),
            arg: Arg::default(),
        },
        Key {
            mod_mask: MOD1,
            keysym: XK_F4,
            func: Some(kill_client),
            arg: Arg::default(),
        },
        Key {
            mod_mask: MODKEY,
            keysym: XK_F1,
            func: Some(spawn),
            arg: Arg {
                v: Some(CMD_HELP),
                ..Default::default()
            },
        },
        Key {
            mod_mask: MODKEY,
            keysym: XK_F2,
            func: Some(toggle_prefix),
            arg: Arg::default(),
        },
        Key {
            mod_mask: MODKEY,
            keysym: XK_t,
            func: Some(set_layout),
            arg: Arg {
                v: Some(0),
                ..Default::default()
            },
        },
        Key {
            mod_mask: MODKEY,
            keysym: XK_f,
            func: Some(set_layout),
            arg: Arg {
                v: Some(2),
                ..Default::default()
            },
        },
        Key {
            mod_mask: MODKEY,
            keysym: XK_m,
            func: Some(set_layout),
            arg: Arg {
                v: Some(3),
                ..Default::default()
            },
        },
        Key {
            mod_mask: MODKEY | SHIFT,
            keysym: XK_m,
            func: Some(move_mouse),
            arg: Arg::default(),
        },
        Key {
            mod_mask: MODKEY | MOD1,
            keysym: XK_m,
            func: Some(resize_mouse),
            arg: Arg::default(),
        },
        Key {
            mod_mask: MODKEY,
            keysym: XK_c,
            func: Some(set_layout),
            arg: Arg {
                v: Some(1),
                ..Default::default()
            },
        },
        Key {
            mod_mask: MODKEY | CONTROL,
            keysym: XK_c,
            func: Some(spawn),
            arg: Arg {
                v: Some(CMD_CONTROLCENTER),
                ..Default::default()
            },
        },
        Key {
            mod_mask: MODKEY,
            keysym: XK_Left,
            func: Some(anim_left),
            arg: Arg::default(),
        },
        Key {
            mod_mask: MODKEY,
            keysym: XK_Right,
            func: Some(anim_right),
            arg: Arg::default(),
        },
        Key {
            mod_mask: MODKEY,
            keysym: XK_e,
            func: Some(toggle_overview),
            arg: Arg {
                ui: !0,
                ..Default::default()
            },
        },
        Key {
            mod_mask: MODKEY | SHIFT,
            keysym: XK_e,
            func: Some(toggle_fullscreen_overview),
            arg: Arg {
                ui: !0,
                ..Default::default()
            },
        },
        Key {
            mod_mask: MODKEY | CONTROL,
            keysym: XK_e,
            func: Some(spawn),
            arg: Arg {
                v: Some(CMD_INSTANTSKIPPY),
                ..Default::default()
            },
        },
        Key {
            mod_mask: MODKEY | CONTROL,
            keysym: XK_Left,
            func: Some(direction_focus),
            arg: Arg {
                ui: 3,
                ..Default::default()
            },
        },
        Key {
            mod_mask: MODKEY | CONTROL,
            keysym: XK_Right,
            func: Some(direction_focus),
            arg: Arg {
                ui: 1,
                ..Default::default()
            },
        },
        Key {
            mod_mask: MODKEY | CONTROL,
            keysym: XK_Up,
            func: Some(direction_focus),
            arg: Arg {
                ui: 0,
                ..Default::default()
            },
        },
        Key {
            mod_mask: MODKEY | CONTROL,
            keysym: XK_Down,
            func: Some(direction_focus),
            arg: Arg {
                ui: 2,
                ..Default::default()
            },
        },
        Key {
            mod_mask: MODKEY | SHIFT | CONTROL,
            keysym: XK_Right,
            func: Some(shift_view),
            arg: Arg {
                i: 1,
                ..Default::default()
            },
        },
        Key {
            mod_mask: MODKEY | SHIFT | CONTROL,
            keysym: XK_Left,
            func: Some(shift_view),
            arg: Arg {
                i: -1,
                ..Default::default()
            },
        },
        Key {
            mod_mask: MODKEY | MOD1,
            keysym: XK_Left,
            func: Some(move_left),
            arg: Arg::default(),
        },
        Key {
            mod_mask: MODKEY | MOD1,
            keysym: XK_Right,
            func: Some(move_right),
            arg: Arg::default(),
        },
        Key {
            mod_mask: MODKEY | SHIFT,
            keysym: XK_Left,
            func: Some(tag_to_left),
            arg: Arg::default(),
        },
        Key {
            mod_mask: MODKEY | SHIFT,
            keysym: XK_Right,
            func: Some(tag_to_right),
            arg: Arg::default(),
        },
        Key {
            mod_mask: MODKEY | SHIFT,
            keysym: XK_j,
            func: Some(move_resize),
            arg: Arg {
                i: 0,
                ..Default::default()
            },
        },
        Key {
            mod_mask: MODKEY | SHIFT,
            keysym: XK_k,
            func: Some(move_resize),
            arg: Arg {
                i: 1,
                ..Default::default()
            },
        },
        Key {
            mod_mask: MODKEY | SHIFT,
            keysym: XK_l,
            func: Some(move_resize),
            arg: Arg {
                i: 2,
                ..Default::default()
            },
        },
        Key {
            mod_mask: MODKEY | SHIFT,
            keysym: XK_h,
            func: Some(move_resize),
            arg: Arg {
                i: 3,
                ..Default::default()
            },
        },
        Key {
            mod_mask: MODKEY | CONTROL,
            keysym: XK_comma,
            func: Some(cycle_layout),
            arg: Arg {
                i: -1,
                ..Default::default()
            },
        },
        Key {
            mod_mask: MODKEY | CONTROL,
            keysym: XK_period,
            func: Some(cycle_layout),
            arg: Arg {
                i: 1,
                ..Default::default()
            },
        },
        Key {
            mod_mask: MODKEY,
            keysym: XK_p,
            func: Some(set_layout),
            arg: Arg::default(),
        },
        Key {
            mod_mask: MODKEY | SHIFT,
            keysym: XK_p,
            func: Some(spawn),
            arg: Arg {
                v: Some(CMD_DISPLAY),
                ..Default::default()
            },
        },
        Key {
            mod_mask: MODKEY | SHIFT,
            keysym: XK_space,
            func: Some(space_toggle),
            arg: Arg::default(),
        },
        Key {
            mod_mask: MODKEY,
            keysym: XK_0,
            func: Some(view),
            arg: Arg {
                ui: !0,
                ..Default::default()
            },
        },
        Key {
            mod_mask: MODKEY | SHIFT,
            keysym: XK_0,
            func: Some(tag),
            arg: Arg {
                ui: !0,
                ..Default::default()
            },
        },
        Key {
            mod_mask: MODKEY,
            keysym: XK_comma,
            func: Some(focus_mon),
            arg: Arg {
                i: -1,
                ..Default::default()
            },
        },
        Key {
            mod_mask: MODKEY,
            keysym: XK_period,
            func: Some(focus_mon),
            arg: Arg {
                i: 1,
                ..Default::default()
            },
        },
        Key {
            mod_mask: MODKEY | SHIFT,
            keysym: XK_comma,
            func: Some(tag_mon),
            arg: Arg {
                i: -1,
                ..Default::default()
            },
        },
        Key {
            mod_mask: MODKEY | SHIFT,
            keysym: XK_period,
            func: Some(tag_mon),
            arg: Arg {
                i: 1,
                ..Default::default()
            },
        },
        Key {
            mod_mask: MODKEY | MOD1,
            keysym: XK_comma,
            func: Some(follow_mon),
            arg: Arg {
                i: -1,
                ..Default::default()
            },
        },
        Key {
            mod_mask: MODKEY | MOD1,
            keysym: XK_period,
            func: Some(follow_mon),
            arg: Arg {
                i: 1,
                ..Default::default()
            },
        },
        Key {
            mod_mask: MODKEY | SHIFT | CONTROL | MOD1,
            keysym: XK_period,
            func: Some(desktop_set),
            arg: Arg::default(),
        },
    ]);

    for tag in 0..9 {
        keys.extend_from_slice(&tagkeys(XK_1 + tag as u32, tag));
    }

    keys.extend_from_slice(&[
        Key {
            mod_mask: MODKEY | SHIFT | CONTROL,
            keysym: XK_q,
            func: Some(quit),
            arg: Arg::default(),
        },
        Key {
            mod_mask: 0,
            keysym: XF86XK_MonBrightnessUp,
            func: Some(spawn),
            arg: Arg {
                v: Some(CMD_UPBRIGHT),
                ..Default::default()
            },
        },
        Key {
            mod_mask: 0,
            keysym: XF86XK_MonBrightnessDown,
            func: Some(spawn),
            arg: Arg {
                v: Some(CMD_DOWNBRIGHT),
                ..Default::default()
            },
        },
        Key {
            mod_mask: 0,
            keysym: XF86XK_AudioLowerVolume,
            func: Some(spawn),
            arg: Arg {
                v: Some(CMD_DOWNVOL),
                ..Default::default()
            },
        },
        Key {
            mod_mask: 0,
            keysym: XF86XK_AudioMute,
            func: Some(spawn),
            arg: Arg {
                v: Some(CMD_MUTEVOL),
                ..Default::default()
            },
        },
        Key {
            mod_mask: 0,
            keysym: XF86XK_AudioRaiseVolume,
            func: Some(spawn),
            arg: Arg {
                v: Some(CMD_UPVOL),
                ..Default::default()
            },
        },
        Key {
            mod_mask: 0,
            keysym: XF86XK_AudioPlay,
            func: Some(spawn),
            arg: Arg {
                v: Some(CMD_PLAYERPAUSE),
                ..Default::default()
            },
        },
        Key {
            mod_mask: 0,
            keysym: XF86XK_AudioPause,
            func: Some(spawn),
            arg: Arg {
                v: Some(CMD_PLAYERPAUSE),
                ..Default::default()
            },
        },
        Key {
            mod_mask: 0,
            keysym: XF86XK_AudioNext,
            func: Some(spawn),
            arg: Arg {
                v: Some(CMD_PLAYERNEXT),
                ..Default::default()
            },
        },
        Key {
            mod_mask: 0,
            keysym: XF86XK_AudioPrev,
            func: Some(spawn),
            arg: Arg {
                v: Some(CMD_PLAYERPREVIOUS),
                ..Default::default()
            },
        },
        Key {
            mod_mask: MODKEY | SHIFT,
            keysym: XK_Print,
            func: Some(spawn),
            arg: Arg {
                v: Some(CMD_FSCROT),
                ..Default::default()
            },
        },
        Key {
            mod_mask: MODKEY,
            keysym: XK_Print,
            func: Some(spawn),
            arg: Arg {
                v: Some(CMD_SCROT),
                ..Default::default()
            },
        },
        Key {
            mod_mask: MODKEY | CONTROL,
            keysym: XK_Print,
            func: Some(spawn),
            arg: Arg {
                v: Some(CMD_CLIPSCROT),
                ..Default::default()
            },
        },
        Key {
            mod_mask: MODKEY | MOD1,
            keysym: XK_Print,
            func: Some(spawn),
            arg: Arg {
                v: Some(CMD_FCLIPSCROT),
                ..Default::default()
            },
        },
        Key {
            mod_mask: MODKEY,
            keysym: XK_o,
            func: Some(win_view),
            arg: Arg::default(),
        },
    ]);

    keys
}

pub fn get_dkeys() -> Vec<Key> {
    use xk::*;
    vec![
        Key {
            mod_mask: 0,
            keysym: XK_r,
            func: Some(spawn),
            arg: Arg {
                v: Some(CMD_YAZI),
                ..Default::default()
            },
        },
        Key {
            mod_mask: 0,
            keysym: XK_e,
            func: Some(spawn),
            arg: Arg {
                v: Some(CMD_EDITOR),
                ..Default::default()
            },
        },
        Key {
            mod_mask: 0,
            keysym: XK_n,
            func: Some(spawn),
            arg: Arg {
                v: Some(CMD_NAUTILUS),
                ..Default::default()
            },
        },
        Key {
            mod_mask: 0,
            keysym: XK_space,
            func: Some(spawn),
            arg: Arg {
                v: Some(CMD_PANTHER),
                ..Default::default()
            },
        },
        Key {
            mod_mask: 0,
            keysym: XK_f,
            func: Some(spawn),
            arg: Arg {
                v: Some(CMD_FIREFOX),
                ..Default::default()
            },
        },
        Key {
            mod_mask: 0,
            keysym: XK_a,
            func: Some(spawn),
            arg: Arg {
                v: Some(CMD_INSTANTASSIST),
                ..Default::default()
            },
        },
        Key {
            mod_mask: 0,
            keysym: XK_F1,
            func: Some(spawn),
            arg: Arg {
                v: Some(CMD_HELP),
                ..Default::default()
            },
        },
        Key {
            mod_mask: 0,
            keysym: XK_m,
            func: Some(spawn),
            arg: Arg {
                v: Some(CMD_SPOTICLI),
                ..Default::default()
            },
        },
        Key {
            mod_mask: 0,
            keysym: XK_Return,
            func: Some(spawn),
            arg: Arg {
                v: Some(CMD_TERM),
                ..Default::default()
            },
        },
        Key {
            mod_mask: 0,
            keysym: XK_plus,
            func: Some(spawn),
            arg: Arg {
                v: Some(CMD_UPVOL),
                ..Default::default()
            },
        },
        Key {
            mod_mask: 0,
            keysym: XK_minus,
            func: Some(spawn),
            arg: Arg {
                v: Some(CMD_DOWNVOL),
                ..Default::default()
            },
        },
        Key {
            mod_mask: 0,
            keysym: XK_Tab,
            func: Some(spawn),
            arg: Arg {
                v: Some(CMD_CARETINSTANTSWITCH),
                ..Default::default()
            },
        },
        Key {
            mod_mask: 0,
            keysym: XK_c,
            func: Some(spawn),
            arg: Arg {
                v: Some(CMD_CODE),
                ..Default::default()
            },
        },
        Key {
            mod_mask: 0,
            keysym: XK_y,
            func: Some(spawn),
            arg: Arg {
                v: Some(CMD_SMART),
                ..Default::default()
            },
        },
        Key {
            mod_mask: 0,
            keysym: XK_v,
            func: Some(spawn),
            arg: Arg {
                v: Some(CMD_QUICKMENU),
                ..Default::default()
            },
        },
        Key {
            mod_mask: 0,
            keysym: XK_h,
            func: Some(view_to_left),
            arg: Arg::default(),
        },
        Key {
            mod_mask: 0,
            keysym: XK_l,
            func: Some(view_to_right),
            arg: Arg::default(),
        },
        Key {
            mod_mask: 0,
            keysym: XK_k,
            func: Some(shift_view),
            arg: Arg {
                i: 1,
                ..Default::default()
            },
        },
        Key {
            mod_mask: 0,
            keysym: XK_j,
            func: Some(shift_view),
            arg: Arg {
                i: -1,
                ..Default::default()
            },
        },
        Key {
            mod_mask: 0,
            keysym: XK_Left,
            func: Some(view_to_left),
            arg: Arg::default(),
        },
        Key {
            mod_mask: 0,
            keysym: XK_Right,
            func: Some(view_to_right),
            arg: Arg::default(),
        },
        Key {
            mod_mask: 0,
            keysym: XK_Up,
            func: Some(shift_view),
            arg: Arg {
                i: 1,
                ..Default::default()
            },
        },
        Key {
            mod_mask: 0,
            keysym: XK_Down,
            func: Some(shift_view),
            arg: Arg {
                i: -1,
                ..Default::default()
            },
        },
        Key {
            mod_mask: 0,
            keysym: XK_1,
            func: Some(view),
            arg: Arg {
                ui: 1 << 0,
                ..Default::default()
            },
        },
        Key {
            mod_mask: 0,
            keysym: XK_2,
            func: Some(view),
            arg: Arg {
                ui: 1 << 1,
                ..Default::default()
            },
        },
        Key {
            mod_mask: 0,
            keysym: XK_3,
            func: Some(view),
            arg: Arg {
                ui: 1 << 2,
                ..Default::default()
            },
        },
        Key {
            mod_mask: 0,
            keysym: XK_4,
            func: Some(view),
            arg: Arg {
                ui: 1 << 3,
                ..Default::default()
            },
        },
        Key {
            mod_mask: 0,
            keysym: XK_5,
            func: Some(view),
            arg: Arg {
                ui: 1 << 4,
                ..Default::default()
            },
        },
        Key {
            mod_mask: 0,
            keysym: XK_6,
            func: Some(view),
            arg: Arg {
                ui: 1 << 5,
                ..Default::default()
            },
        },
        Key {
            mod_mask: 0,
            keysym: XK_7,
            func: Some(view),
            arg: Arg {
                ui: 1 << 6,
                ..Default::default()
            },
        },
        Key {
            mod_mask: 0,
            keysym: XK_8,
            func: Some(view),
            arg: Arg {
                ui: 1 << 7,
                ..Default::default()
            },
        },
        Key {
            mod_mask: 0,
            keysym: XK_9,
            func: Some(view),
            arg: Arg {
                ui: 1 << 8,
                ..Default::default()
            },
        },
    ]
}

pub fn get_buttons() -> Vec<Button> {
    use crate::types::Click::*;
    vec![
        Button {
            click: LtSymbol,
            mask: 0,
            button: 1,
            func: Some(cycle_layout),
            arg: Arg {
                i: -1,
                ..Default::default()
            },
        },
        Button {
            click: LtSymbol,
            mask: 0,
            button: 3,
            func: Some(cycle_layout),
            arg: Arg {
                i: 1,
                ..Default::default()
            },
        },
        Button {
            click: LtSymbol,
            mask: MODKEY,
            button: 1,
            func: Some(create_overlay),
            arg: Arg::default(),
        },
        Button {
            click: LtSymbol,
            mask: 0,
            button: 2,
            func: Some(set_layout),
            arg: Arg {
                v: Some(0),
                ..Default::default()
            },
        },
        Button {
            click: WinTitle,
            mask: 0,
            button: 1,
            func: Some(window_title_mouse_handler),
            arg: Arg::default(),
        },
        Button {
            click: WinTitle,
            mask: MODKEY,
            button: 1,
            func: Some(set_overlay),
            arg: Arg::default(),
        },
        Button {
            click: WinTitle,
            mask: MODKEY,
            button: 3,
            func: Some(spawn),
            arg: Arg {
                v: Some(CMD_NOTIFY),
                ..Default::default()
            },
        },
        Button {
            click: StatusText,
            mask: 0,
            button: 3,
            func: Some(spawn),
            arg: Arg {
                v: Some(CMD_CARETINSTANTSWITCH),
                ..Default::default()
            },
        },
        Button {
            click: WinTitle,
            mask: 0,
            button: 2,
            func: Some(close_win),
            arg: Arg::default(),
        },
        Button {
            click: CloseButton,
            mask: 0,
            button: 1,
            func: Some(kill_client),
            arg: Arg::default(),
        },
        Button {
            click: CloseButton,
            mask: 0,
            button: 3,
            func: Some(toggle_locked),
            arg: Arg::default(),
        },
        Button {
            click: ResizeWidget,
            mask: 0,
            button: 1,
            func: Some(draw_window),
            arg: Arg::default(),
        },
        Button {
            click: WinTitle,
            mask: 0,
            button: 3,
            func: Some(window_title_mouse_handler_right),
            arg: Arg::default(),
        },
        Button {
            click: WinTitle,
            mask: 0,
            button: 5,
            func: Some(focus_stack),
            arg: Arg {
                i: 1,
                ..Default::default()
            },
        },
        Button {
            click: WinTitle,
            mask: 0,
            button: 4,
            func: Some(focus_stack),
            arg: Arg {
                i: -1,
                ..Default::default()
            },
        },
        Button {
            click: WinTitle,
            mask: SHIFT,
            button: 5,
            func: Some(push_down),
            arg: Arg::default(),
        },
        Button {
            click: WinTitle,
            mask: SHIFT,
            button: 4,
            func: Some(push_up),
            arg: Arg::default(),
        },
        Button {
            click: WinTitle,
            mask: CONTROL,
            button: 5,
            func: Some(down_scale_client),
            arg: Arg::default(),
        },
        Button {
            click: WinTitle,
            mask: CONTROL,
            button: 4,
            func: Some(up_scale_client),
            arg: Arg::default(),
        },
        Button {
            click: StatusText,
            mask: 0,
            button: 2,
            func: Some(spawn),
            arg: Arg {
                v: Some(CMD_TERM),
                ..Default::default()
            },
        },
        Button {
            click: StatusText,
            mask: 0,
            button: 4,
            func: Some(spawn),
            arg: Arg {
                v: Some(CMD_UPVOL),
                ..Default::default()
            },
        },
        Button {
            click: StatusText,
            mask: 0,
            button: 5,
            func: Some(spawn),
            arg: Arg {
                v: Some(CMD_DOWNVOL),
                ..Default::default()
            },
        },
        Button {
            click: StatusText,
            mask: MODKEY,
            button: 2,
            func: Some(spawn),
            arg: Arg {
                v: Some(CMD_MUTEVOL),
                ..Default::default()
            },
        },
        Button {
            click: StatusText,
            mask: 0,
            button: 1,
            func: Some(spawn),
            arg: Arg {
                v: Some(CMD_PANTHER),
                ..Default::default()
            },
        },
        Button {
            click: StatusText,
            mask: MODKEY | SHIFT,
            button: 1,
            func: Some(spawn),
            arg: Arg {
                v: Some(CMD_PAVUCONTROL),
                ..Default::default()
            },
        },
        Button {
            click: StatusText,
            mask: MODKEY | CONTROL,
            button: 1,
            func: Some(spawn),
            arg: Arg {
                v: Some(CMD_NOTIFY),
                ..Default::default()
            },
        },
        Button {
            click: StatusText,
            mask: MODKEY,
            button: 1,
            func: Some(spawn),
            arg: Arg {
                v: Some(CMD_INSTANTSETTINGS),
                ..Default::default()
            },
        },
        Button {
            click: StatusText,
            mask: MODKEY,
            button: 3,
            func: Some(spawn),
            arg: Arg {
                v: Some(CMD_SPOTICLI),
                ..Default::default()
            },
        },
        Button {
            click: StatusText,
            mask: MODKEY,
            button: 4,
            func: Some(spawn),
            arg: Arg {
                v: Some(CMD_UPBRIGHT),
                ..Default::default()
            },
        },
        Button {
            click: StatusText,
            mask: MODKEY,
            button: 5,
            func: Some(spawn),
            arg: Arg {
                v: Some(CMD_DOWNBRIGHT),
                ..Default::default()
            },
        },
        Button {
            click: RootWin,
            mask: MODKEY,
            button: 3,
            func: Some(spawn),
            arg: Arg {
                v: Some(CMD_NOTIFY),
                ..Default::default()
            },
        },
        Button {
            click: RootWin,
            mask: 0,
            button: 1,
            func: Some(spawn),
            arg: Arg {
                v: Some(CMD_PANTHER),
                ..Default::default()
            },
        },
        Button {
            click: RootWin,
            mask: MODKEY,
            button: 1,
            func: Some(set_overlay),
            arg: Arg::default(),
        },
        Button {
            click: RootWin,
            mask: 0,
            button: 3,
            func: Some(spawn),
            arg: Arg {
                v: Some(CMD_SMART),
                ..Default::default()
            },
        },
        Button {
            click: RootWin,
            mask: 0,
            button: 5,
            func: Some(show_overlay),
            arg: Arg::default(),
        },
        Button {
            click: RootWin,
            mask: 0,
            button: 4,
            func: Some(hide_overlay),
            arg: Arg::default(),
        },
        Button {
            click: RootWin,
            mask: 0,
            button: 2,
            func: Some(spawn),
            arg: Arg {
                v: Some(CMD_INSTANTMENU),
                ..Default::default()
            },
        },
        Button {
            click: ClientWin,
            mask: MODKEY,
            button: 1,
            func: Some(move_mouse),
            arg: Arg::default(),
        },
        Button {
            click: ClientWin,
            mask: MODKEY,
            button: 2,
            func: Some(toggle_floating),
            arg: Arg::default(),
        },
        Button {
            click: ClientWin,
            mask: MODKEY,
            button: 3,
            func: Some(resize_mouse),
            arg: Arg::default(),
        },
        Button {
            click: ClientWin,
            mask: MODKEY | MOD1,
            button: 3,
            func: Some(force_resize_mouse),
            arg: Arg::default(),
        },
        Button {
            click: ClientWin,
            mask: MODKEY | SHIFT,
            button: 3,
            func: Some(resize_aspect_mouse),
            arg: Arg::default(),
        },
        Button {
            click: TagBar,
            mask: 0,
            button: 1,
            func: Some(drag_tag),
            arg: Arg::default(),
        },
        Button {
            click: TagBar,
            mask: 0,
            button: 5,
            func: Some(view_to_right),
            arg: Arg::default(),
        },
        Button {
            click: TagBar,
            mask: MODKEY,
            button: 4,
            func: Some(shift_view),
            arg: Arg {
                i: -1,
                ..Default::default()
            },
        },
        Button {
            click: TagBar,
            mask: MODKEY,
            button: 5,
            func: Some(shift_view),
            arg: Arg {
                i: 1,
                ..Default::default()
            },
        },
        Button {
            click: TagBar,
            mask: 0,
            button: 4,
            func: Some(view_to_left),
            arg: Arg::default(),
        },
        Button {
            click: TagBar,
            mask: 0,
            button: 3,
            func: Some(toggle_view),
            arg: Arg::default(),
        },
        Button {
            click: TagBar,
            mask: MODKEY,
            button: 1,
            func: Some(tag),
            arg: Arg::default(),
        },
        Button {
            click: TagBar,
            mask: MOD1,
            button: 1,
            func: Some(follow_tag),
            arg: Arg::default(),
        },
        Button {
            click: TagBar,
            mask: MODKEY,
            button: 3,
            func: Some(toggle_tag),
            arg: Arg::default(),
        },
        Button {
            click: ShutDown,
            mask: 0,
            button: 1,
            func: Some(spawn),
            arg: Arg {
                v: Some(CMD_INSTANTSHUTDOWN),
                ..Default::default()
            },
        },
        Button {
            click: ShutDown,
            mask: 0,
            button: 3,
            func: Some(spawn),
            arg: Arg {
                v: Some(CMD_SLOCK),
                ..Default::default()
            },
        },
        Button {
            click: ShutDown,
            mask: 0,
            button: 2,
            func: Some(spawn),
            arg: Arg {
                v: Some(CMD_OSLOCK),
                ..Default::default()
            },
        },
        Button {
            click: SideBar,
            mask: 0,
            button: 1,
            func: Some(gesture_mouse),
            arg: Arg::default(),
        },
        Button {
            click: StartMenu,
            mask: 0,
            button: 1,
            func: Some(spawn),
            arg: Arg {
                v: Some(CMD_STARTMENU),
                ..Default::default()
            },
        },
        Button {
            click: StartMenu,
            mask: SHIFT,
            button: 1,
            func: Some(toggle_prefix),
            arg: Arg::default(),
        },
        Button {
            click: StartMenu,
            mask: 0,
            button: 3,
            func: Some(spawn),
            arg: Arg {
                v: Some(CMD_QUICKMENU),
                ..Default::default()
            },
        },
    ]
}

pub fn get_commands() -> Vec<XCommand> {
    vec![
        XCommand {
            cmd: "overlay",
            func: Some(set_overlay),
            arg: Arg {
                ui: 0,
                ..Default::default()
            },
            cmd_type: 0,
        },
        XCommand {
            cmd: "warpfocus",
            func: Some(warp_to_focus),
            arg: Arg {
                ui: 0,
                ..Default::default()
            },
            cmd_type: 0,
        },
        XCommand {
            cmd: "tag",
            func: Some(view),
            arg: Arg {
                ui: 2,
                ..Default::default()
            },
            cmd_type: 3,
        },
        XCommand {
            cmd: "animated",
            func: Some(toggle_animated),
            arg: Arg {
                ui: 2,
                ..Default::default()
            },
            cmd_type: 1,
        },
        XCommand {
            cmd: "border",
            func: Some(set_border_width),
            arg: Arg {
                i: BORDERPX as i32,
                ..Default::default()
            },
            cmd_type: 5,
        },
        XCommand {
            cmd: "focusfollowsmouse",
            func: Some(toggle_focus_follows_mouse),
            arg: Arg {
                ui: 2,
                ..Default::default()
            },
            cmd_type: 1,
        },
        XCommand {
            cmd: "focusfollowsfloatmouse",
            func: Some(toggle_focus_follows_float_mouse),
            arg: Arg {
                ui: 2,
                ..Default::default()
            },
            cmd_type: 1,
        },
        XCommand {
            cmd: "alttab",
            func: Some(alt_tab_free),
            arg: Arg {
                ui: 2,
                ..Default::default()
            },
            cmd_type: 1,
        },
        XCommand {
            cmd: "layout",
            func: Some(command_layout),
            arg: Arg {
                ui: 0,
                ..Default::default()
            },
            cmd_type: 1,
        },
        XCommand {
            cmd: "prefix",
            func: Some(command_prefix),
            arg: Arg {
                ui: 1,
                ..Default::default()
            },
            cmd_type: 1,
        },
        XCommand {
            cmd: "alttag",
            func: Some(toggle_alt_tag),
            arg: Arg {
                ui: 0,
                ..Default::default()
            },
            cmd_type: 1,
        },
        XCommand {
            cmd: "hidetags",
            func: Some(toggle_show_tags),
            arg: Arg {
                ui: 0,
                ..Default::default()
            },
            cmd_type: 1,
        },
        XCommand {
            cmd: "specialnext",
            func: Some(set_special_next),
            arg: Arg {
                ui: 0,
                ..Default::default()
            },
            cmd_type: 3,
        },
        XCommand {
            cmd: "tagmon",
            func: Some(tag_mon),
            arg: Arg {
                i: 1,
                ..Default::default()
            },
            cmd_type: 0,
        },
        XCommand {
            cmd: "followmon",
            func: Some(follow_mon),
            arg: Arg {
                i: 1,
                ..Default::default()
            },
            cmd_type: 0,
        },
        XCommand {
            cmd: "focusmon",
            func: Some(focus_mon),
            arg: Arg {
                i: 1,
                ..Default::default()
            },
            cmd_type: 0,
        },
        XCommand {
            cmd: "focusnmon",
            func: Some(focus_nmon),
            arg: Arg {
                i: 0,
                ..Default::default()
            },
            cmd_type: 5,
        },
        XCommand {
            cmd: "nametag",
            func: Some(name_tag),
            arg: Arg {
                v: Some(CMD_TAG),
                ..Default::default()
            },
            cmd_type: 4,
        },
        XCommand {
            cmd: "resetnametag",
            func: Some(reset_name_tag),
            arg: Arg::default(),
            cmd_type: 0,
        },
        XCommand {
            cmd: "scratchpad-make",
            func: Some(scratchpad_make),
            arg: Arg::default(),
            cmd_type: 4,
        },
        XCommand {
            cmd: "scratchpad-unmake",
            func: Some(scratchpad_unmake),
            arg: Arg::default(),
            cmd_type: 0,
        },
        XCommand {
            cmd: "scratchpad-toggle",
            func: Some(scratchpad_toggle),
            arg: Arg::default(),
            cmd_type: 4,
        },
        XCommand {
            cmd: "scratchpad-show",
            func: Some(scratchpad_show),
            arg: Arg::default(),
            cmd_type: 4,
        },
        XCommand {
            cmd: "scratchpad-hide",
            func: Some(scratchpad_hide),
            arg: Arg::default(),
            cmd_type: 4,
        },
        XCommand {
            cmd: "scratchpad-status",
            func: Some(scratchpad_status),
            arg: Arg::default(),
            cmd_type: 4,
        },
    ]
}

pub fn get_resources() -> Vec<ResourcePref> {
    vec![
        ResourcePref {
            name: "barheight",
            rtype: ResourceType::Integer,
        },
        ResourcePref {
            name: "font",
            rtype: ResourceType::String,
        },
        ResourcePref {
            name: "tag1",
            rtype: ResourceType::String,
        },
        ResourcePref {
            name: "tag2",
            rtype: ResourceType::String,
        },
        ResourcePref {
            name: "tag3",
            rtype: ResourceType::String,
        },
        ResourcePref {
            name: "tag4",
            rtype: ResourceType::String,
        },
        ResourcePref {
            name: "tag5",
            rtype: ResourceType::String,
        },
        ResourcePref {
            name: "tag6",
            rtype: ResourceType::String,
        },
        ResourcePref {
            name: "tag7",
            rtype: ResourceType::String,
        },
        ResourcePref {
            name: "tag8",
            rtype: ResourceType::String,
        },
        ResourcePref {
            name: "tag9",
            rtype: ResourceType::String,
        },
    ]
}

pub const CMD_DEFAULT: usize = 0;
pub const CMD_TERM: usize = 1;
pub const CMD_INSTANTMENU: usize = 2;
pub const CMD_CLIPMENU: usize = 3;
pub const CMD_SMART: usize = 4;
pub const CMD_INSTANTMENU_ST: usize = 5;
pub const CMD_QUICKMENU: usize = 6;
pub const CMD_INSTANTASSIST: usize = 7;
pub const CMD_INSTANTREPEAT: usize = 8;
pub const CMD_INSTANTPACMAN: usize = 9;
pub const CMD_INSTANTSHARE: usize = 10;
pub const CMD_NAUTILUS: usize = 11;
pub const CMD_SLOCK: usize = 12;
pub const CMD_ONEKEYLOCK: usize = 13;
pub const CMD_LANGSWITCH: usize = 14;
pub const CMD_OSLOCK: usize = 15;
pub const CMD_HELP: usize = 16;
pub const CMD_SEARCH: usize = 17;
pub const CMD_KEYLAYOUTSWITCH: usize = 18;
pub const CMD_ISWITCH: usize = 19;
pub const CMD_INSTANTSWITCH: usize = 20;
pub const CMD_CARETINSTANTSWITCH: usize = 21;
pub const CMD_INSTANTSKIPPY: usize = 22;
pub const CMD_INSTANTSHUTDOWN: usize = 23;
pub const CMD_SYSTEMMONITOR: usize = 24;
pub const CMD_NOTIFY: usize = 25;
pub const CMD_YAZI: usize = 26;
pub const CMD_PANTHER: usize = 27;
pub const CMD_CONTROLCENTER: usize = 28;
pub const CMD_DISPLAY: usize = 29;
pub const CMD_PAVUCONTROL: usize = 30;
pub const CMD_INSTANTSETTINGS: usize = 31;
pub const CMD_CODE: usize = 32;
pub const CMD_STARTMENU: usize = 33;
pub const CMD_SCROT: usize = 34;
pub const CMD_FSCROT: usize = 35;
pub const CMD_CLIPSCROT: usize = 36;
pub const CMD_FCLIPSCROT: usize = 37;
pub const CMD_FIREFOX: usize = 38;
pub const CMD_EDITOR: usize = 39;
pub const CMD_PLAYERNEXT: usize = 40;
pub const CMD_PLAYERPREVIOUS: usize = 41;
pub const CMD_PLAYERPAUSE: usize = 42;
pub const CMD_SPOTICLI: usize = 43;
pub const CMD_UPVOL: usize = 44;
pub const CMD_DOWNVOL: usize = 45;
pub const CMD_MUTEVOL: usize = 46;
pub const CMD_UPBRIGHT: usize = 47;
pub const CMD_DOWNBRIGHT: usize = 48;
pub const CMD_TAG: usize = 49;

pub fn init_config() -> Config {
    Config {
        borderpx: BORDERPX,
        snap: 32,
        startmenusize: 30,
        systraypinning: 0,
        systrayspacing: 0,
        systraypinningfailfirst: true,
        showsystray: true,
        showbar: true,
        topbar: true,
        barheight: 0,
        resizehints: 1,
        decorhints: 1,
        mfact: 0.55,
        nmaster: 1,
        tags: get_tags(),
        tagsalt: get_tagsalt(),
        tagcolors: get_tagcolors(),
        windowcolors: get_windowcolors(),
        closebuttoncolors: get_closebuttoncolors(),
        bordercolors: get_bordercolors(),
        statusbarcolors: get_statusbarcolors(),
        layouts: get_layouts(),
        keys: get_keys(),
        dkeys: get_dkeys(),
        buttons: get_buttons(),
        rules: get_rules(),
        commands: get_commands(),
        resources: get_resources(),
        fonts: get_fonts(),
        external_commands: get_external_commands(),
        tagmask: TAGMASK,
        numtags: MAX_TAGS as i32,
    }
}

#[derive(Debug, Clone)]
pub struct Config {
    pub borderpx: u32,
    pub snap: u32,
    pub startmenusize: u32,
    pub systraypinning: u32,
    pub systrayspacing: u32,
    pub systraypinningfailfirst: bool,
    pub showsystray: bool,
    pub showbar: bool,
    pub topbar: bool,
    pub barheight: i32,
    pub resizehints: i32,
    pub decorhints: i32,
    pub mfact: f32,
    pub nmaster: i32,
    pub tags: [[u8; MAX_TAGLEN]; MAX_TAGS],
    pub tagsalt: Vec<&'static str>,
    pub tagcolors: Vec<Vec<Vec<&'static str>>>,
    pub windowcolors: Vec<Vec<Vec<&'static str>>>,
    pub closebuttoncolors: Vec<Vec<Vec<&'static str>>>,
    pub bordercolors: Vec<&'static str>,
    pub statusbarcolors: Vec<&'static str>,
    pub layouts: Vec<&'static dyn Layout>,
    pub keys: Vec<Key>,
    pub dkeys: Vec<Key>,
    pub buttons: Vec<Button>,
    pub rules: Vec<Rule>,
    pub commands: Vec<XCommand>,
    pub resources: Vec<ResourcePref>,
    pub fonts: Vec<&'static str>,
    pub external_commands: ExternalCommands,
    pub tagmask: u32,
    pub numtags: i32,
}

pub fn run_autostart() {}

pub fn get_tag_color(hover: SchemeHover, tag_scheme: SchemeTag, col: ColIndex) -> &'static str {
    use colors::*;
    match (hover, tag_scheme, col) {
        (SchemeHover::NoHover, SchemeTag::Inactive, ColIndex::Fg) => COL_TEXT,
        (SchemeHover::NoHover, SchemeTag::Inactive, ColIndex::Bg) => COL_BG,
        (SchemeHover::NoHover, SchemeTag::Inactive, ColIndex::Detail) => COL_BG,
        (SchemeHover::NoHover, SchemeTag::Filled, ColIndex::Fg) => COL_TEXT,
        (SchemeHover::NoHover, SchemeTag::Filled, ColIndex::Bg) => COL_BG_ACCENT,
        (SchemeHover::NoHover, SchemeTag::Filled, ColIndex::Detail) => COL_LIGHT_BLUE,
        (SchemeHover::NoHover, SchemeTag::Focus, ColIndex::Fg) => COL_BLACK,
        (SchemeHover::NoHover, SchemeTag::Focus, ColIndex::Bg) => COL_LIGHT_GREEN,
        (SchemeHover::NoHover, SchemeTag::Focus, ColIndex::Detail) => COL_GREEN,
        (SchemeHover::NoHover, SchemeTag::NoFocus, ColIndex::Fg) => COL_BLACK,
        (SchemeHover::NoHover, SchemeTag::NoFocus, ColIndex::Bg) => COL_LIGHT_YELLOW,
        (SchemeHover::NoHover, SchemeTag::NoFocus, ColIndex::Detail) => COL_YELLOW,
        (SchemeHover::NoHover, SchemeTag::Empty, ColIndex::Fg) => COL_BLACK,
        (SchemeHover::NoHover, SchemeTag::Empty, ColIndex::Bg) => COL_LIGHT_RED,
        (SchemeHover::NoHover, SchemeTag::Empty, ColIndex::Detail) => COL_RED,
        (SchemeHover::Hover, SchemeTag::Inactive, ColIndex::Fg) => COL_TEXT,
        (SchemeHover::Hover, SchemeTag::Inactive, ColIndex::Bg) => COL_BG_HOVER,
        (SchemeHover::Hover, SchemeTag::Inactive, ColIndex::Detail) => COL_BG,
        (SchemeHover::Hover, SchemeTag::Filled, ColIndex::Fg) => COL_TEXT,
        (SchemeHover::Hover, SchemeTag::Filled, ColIndex::Bg) => COL_BG_ACCENT_HOVER,
        (SchemeHover::Hover, SchemeTag::Filled, ColIndex::Detail) => COL_LIGHT_BLUE_HOVER,
        (SchemeHover::Hover, SchemeTag::Focus, ColIndex::Fg) => COL_BLACK,
        (SchemeHover::Hover, SchemeTag::Focus, ColIndex::Bg) => COL_LIGHT_GREEN_HOVER,
        (SchemeHover::Hover, SchemeTag::Focus, ColIndex::Detail) => COL_GREEN_HOVER,
        (SchemeHover::Hover, SchemeTag::NoFocus, ColIndex::Fg) => COL_BLACK,
        (SchemeHover::Hover, SchemeTag::NoFocus, ColIndex::Bg) => COL_LIGHT_YELLOW_HOVER,
        (SchemeHover::Hover, SchemeTag::NoFocus, ColIndex::Detail) => COL_YELLOW_HOVER,
        (SchemeHover::Hover, SchemeTag::Empty, ColIndex::Fg) => COL_BLACK,
        (SchemeHover::Hover, SchemeTag::Empty, ColIndex::Bg) => COL_LIGHT_RED_HOVER,
        (SchemeHover::Hover, SchemeTag::Empty, ColIndex::Detail) => COL_RED_HOVER,
    }
}

pub fn get_window_color(hover: SchemeHover, win_scheme: SchemeWin, col: ColIndex) -> &'static str {
    use colors::*;
    match (hover, win_scheme, col) {
        (SchemeHover::NoHover, SchemeWin::Focus, ColIndex::Fg) => COL_TEXT,
        (SchemeHover::NoHover, SchemeWin::Focus, ColIndex::Bg) => COL_BG_ACCENT,
        (SchemeHover::NoHover, SchemeWin::Focus, ColIndex::Detail) => COL_LIGHT_BLUE,
        (SchemeHover::NoHover, SchemeWin::Normal, ColIndex::Fg) => COL_TEXT,
        (SchemeHover::NoHover, SchemeWin::Normal, ColIndex::Bg) => COL_BG,
        (SchemeHover::NoHover, SchemeWin::Normal, ColIndex::Detail) => COL_BG,
        (SchemeHover::NoHover, SchemeWin::Minimized, ColIndex::Fg) => COL_BG_ACCENT,
        (SchemeHover::NoHover, SchemeWin::Minimized, ColIndex::Bg) => COL_BG,
        (SchemeHover::NoHover, SchemeWin::Minimized, ColIndex::Detail) => COL_BG,
        (SchemeHover::NoHover, SchemeWin::Sticky, ColIndex::Fg) => COL_BLACK,
        (SchemeHover::NoHover, SchemeWin::Sticky, ColIndex::Bg) => COL_LIGHT_YELLOW,
        (SchemeHover::NoHover, SchemeWin::Sticky, ColIndex::Detail) => COL_YELLOW,
        (SchemeHover::NoHover, SchemeWin::StickyFocus, ColIndex::Fg) => COL_BLACK,
        (SchemeHover::NoHover, SchemeWin::StickyFocus, ColIndex::Bg) => COL_LIGHT_GREEN,
        (SchemeHover::NoHover, SchemeWin::StickyFocus, ColIndex::Detail) => COL_GREEN,
        (SchemeHover::NoHover, SchemeWin::Overlay, ColIndex::Fg) => COL_BLACK,
        (SchemeHover::NoHover, SchemeWin::Overlay, ColIndex::Bg) => COL_LIGHT_YELLOW,
        (SchemeHover::NoHover, SchemeWin::Overlay, ColIndex::Detail) => COL_YELLOW,
        (SchemeHover::NoHover, SchemeWin::OverlayFocus, ColIndex::Fg) => COL_BLACK,
        (SchemeHover::NoHover, SchemeWin::OverlayFocus, ColIndex::Bg) => COL_LIGHT_GREEN,
        (SchemeHover::NoHover, SchemeWin::OverlayFocus, ColIndex::Detail) => COL_GREEN,
        (SchemeHover::Hover, SchemeWin::Focus, ColIndex::Fg) => COL_TEXT,
        (SchemeHover::Hover, SchemeWin::Focus, ColIndex::Bg) => COL_BG_ACCENT_HOVER,
        (SchemeHover::Hover, SchemeWin::Focus, ColIndex::Detail) => COL_LIGHT_BLUE_HOVER,
        (SchemeHover::Hover, SchemeWin::Normal, ColIndex::Fg) => COL_TEXT,
        (SchemeHover::Hover, SchemeWin::Normal, ColIndex::Bg) => COL_BG_HOVER,
        (SchemeHover::Hover, SchemeWin::Normal, ColIndex::Detail) => COL_BG_HOVER,
        (SchemeHover::Hover, SchemeWin::Minimized, ColIndex::Fg) => COL_BG_ACCENT_HOVER,
        (SchemeHover::Hover, SchemeWin::Minimized, ColIndex::Bg) => COL_BG,
        (SchemeHover::Hover, SchemeWin::Minimized, ColIndex::Detail) => COL_BG,
        (SchemeHover::Hover, SchemeWin::Sticky, ColIndex::Fg) => COL_BLACK,
        (SchemeHover::Hover, SchemeWin::Sticky, ColIndex::Bg) => COL_LIGHT_YELLOW_HOVER,
        (SchemeHover::Hover, SchemeWin::Sticky, ColIndex::Detail) => COL_YELLOW_HOVER,
        (SchemeHover::Hover, SchemeWin::StickyFocus, ColIndex::Fg) => COL_BLACK,
        (SchemeHover::Hover, SchemeWin::StickyFocus, ColIndex::Bg) => COL_LIGHT_GREEN_HOVER,
        (SchemeHover::Hover, SchemeWin::StickyFocus, ColIndex::Detail) => COL_GREEN_HOVER,
        (SchemeHover::Hover, SchemeWin::Overlay, ColIndex::Fg) => COL_BLACK,
        (SchemeHover::Hover, SchemeWin::Overlay, ColIndex::Bg) => COL_LIGHT_YELLOW_HOVER,
        (SchemeHover::Hover, SchemeWin::Overlay, ColIndex::Detail) => COL_YELLOW_HOVER,
        (SchemeHover::Hover, SchemeWin::OverlayFocus, ColIndex::Fg) => COL_BLACK,
        (SchemeHover::Hover, SchemeWin::OverlayFocus, ColIndex::Bg) => COL_LIGHT_GREEN_HOVER,
        (SchemeHover::Hover, SchemeWin::OverlayFocus, ColIndex::Detail) => COL_GREEN_HOVER,
    }
}

pub fn get_close_button_color(
    hover: SchemeHover,
    close_scheme: SchemeClose,
    col: ColIndex,
) -> &'static str {
    use colors::*;
    match (hover, close_scheme, col) {
        (SchemeHover::NoHover, SchemeClose::Normal, ColIndex::Fg) => COL_TEXT,
        (SchemeHover::NoHover, SchemeClose::Normal, ColIndex::Bg) => COL_LIGHT_RED,
        (SchemeHover::NoHover, SchemeClose::Normal, ColIndex::Detail) => COL_RED,
        (SchemeHover::NoHover, SchemeClose::Locked, ColIndex::Fg) => COL_TEXT,
        (SchemeHover::NoHover, SchemeClose::Locked, ColIndex::Bg) => COL_LIGHT_YELLOW,
        (SchemeHover::NoHover, SchemeClose::Locked, ColIndex::Detail) => COL_YELLOW,
        (SchemeHover::NoHover, SchemeClose::Fullscreen, ColIndex::Fg) => COL_TEXT,
        (SchemeHover::NoHover, SchemeClose::Fullscreen, ColIndex::Bg) => COL_LIGHT_RED,
        (SchemeHover::NoHover, SchemeClose::Fullscreen, ColIndex::Detail) => COL_RED,
        (SchemeHover::Hover, SchemeClose::Normal, ColIndex::Fg) => COL_TEXT,
        (SchemeHover::Hover, SchemeClose::Normal, ColIndex::Bg) => COL_LIGHT_RED_HOVER,
        (SchemeHover::Hover, SchemeClose::Normal, ColIndex::Detail) => COL_RED_HOVER,
        (SchemeHover::Hover, SchemeClose::Locked, ColIndex::Fg) => COL_TEXT,
        (SchemeHover::Hover, SchemeClose::Locked, ColIndex::Bg) => COL_LIGHT_YELLOW_HOVER,
        (SchemeHover::Hover, SchemeClose::Locked, ColIndex::Detail) => COL_YELLOW_HOVER,
        (SchemeHover::Hover, SchemeClose::Fullscreen, ColIndex::Fg) => COL_TEXT,
        (SchemeHover::Hover, SchemeClose::Fullscreen, ColIndex::Bg) => COL_LIGHT_RED_HOVER,
        (SchemeHover::Hover, SchemeClose::Fullscreen, ColIndex::Detail) => COL_RED_HOVER,
    }
}

pub fn get_border_color(border_scheme: SchemeBorder) -> &'static str {
    use colors::*;
    match border_scheme {
        SchemeBorder::Normal => COL_BG_ACCENT,
        SchemeBorder::TileFocus => COL_LIGHT_BLUE,
        SchemeBorder::FloatFocus => COL_LIGHT_GREEN,
        SchemeBorder::Snap => COL_LIGHT_YELLOW,
    }
}
