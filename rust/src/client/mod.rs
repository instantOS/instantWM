//! Client window management.
//!
//! This module is the public surface for everything related to managing X11
//! client windows.  The implementation is split across focused sub-modules;
//! this file re-exports the public API so that callers can write
//! `crate::client::manage(...)` instead of
//! `crate::client::lifecycle::manage(...)`.
//!
//! # Sub-module map
//!
//! | Module          | Responsibility                                              |
//! |-----------------|-------------------------------------------------------------|
//! | `constants`     | WM_STATE, MWM hints, WM_HINTS, XSizeHints constants         |
//! | `list`          | Intrusive linked-list helpers (attach / detach / traverse)  |
//! | `geometry`      | Resize, size-hint enforcement, dimension helpers            |
//! | `visibility`    | Show / hide / show_hide, WM_STATE queries                   |
//! | `focus`         | Input focus, button grabs, ConfigureNotify, ClientMessage   |
//! | `fullscreen`    | Real and fake fullscreen transitions                        |
//! | `state`         | X11 property read/write (titles, rules, hints, lists)       |
//! | `kill`          | Graceful and forceful window termination                    |
//! | `lifecycle`     | manage() / unmanage() – adopting and releasing windows      |
//! | `layout_ops`    | zoom (promote to master), change_floating (clear snap)      |

// ---------------------------------------------------------------------------
// Sub-modules
// ---------------------------------------------------------------------------

pub mod constants;
pub mod focus;
pub mod fullscreen;
pub mod geometry;
pub mod kill;
pub mod layout_ops;
pub mod lifecycle;
pub mod list;
pub mod state;
pub mod visibility;

// ---------------------------------------------------------------------------
// Flat re-exports – the public API of `crate::client`
// ---------------------------------------------------------------------------

// -- Constants ---------------------------------------------------------------
pub use constants::{
    BROKEN, MWM_DECOR_ALL, MWM_DECOR_BORDER, MWM_DECOR_TITLE, MWM_HINTS_DECORATIONS,
    MWM_HINTS_DECORATIONS_FIELD, MWM_HINTS_FLAGS_FIELD, SIZE_HINTS_P_ASPECT,
    SIZE_HINTS_P_BASE_SIZE, SIZE_HINTS_P_MAX_SIZE, SIZE_HINTS_P_MIN_SIZE, SIZE_HINTS_P_RESIZE_INC,
    WM_HINTS_INPUT_HINT, WM_HINTS_URGENCY_HINT, WM_STATE_ICONIC, WM_STATE_NORMAL,
    WM_STATE_WITHDRAWN,
};

// -- Linked-list management --------------------------------------------------
pub use list::{attach, attach_stack, detach, detach_stack, next_tiled, pop, win_to_client};

// -- Geometry ----------------------------------------------------------------
pub use geometry::{
    apply_size_hints, client_height, client_width, resize, resize_client, resize_client_rect,
    scale_client, update_size_hints, update_size_hints_win,
};

// -- Visibility --------------------------------------------------------------
pub use visibility::{get_state, hide, is_hidden, show, show_hide};

// -- Focus / input -----------------------------------------------------------
pub use focus::{
    configure, grab_buttons, send_event, set_focus, unfocus_win, ANIM_CLIENT, LAST_CLIENT,
};

// -- Fullscreen --------------------------------------------------------------
pub use fullscreen::{
    restore_border_width, save_border_width, set_fullscreen, toggle_fake_fullscreen,
};

// -- X11 state / properties --------------------------------------------------
pub use state::{
    apply_rules, get_atom_prop, set_client_state, set_client_tag_prop, set_urgent,
    update_client_list, update_motif_hints, update_title, update_window_type, update_wm_hints,
};

// -- Kill --------------------------------------------------------------------
pub use kill::{close_win, kill_client, shut_kill};

// -- Lifecycle ---------------------------------------------------------------
pub use lifecycle::{manage, unmanage};

// -- Layout operations -------------------------------------------------------
pub use layout_ops::{change_floating, zoom};
