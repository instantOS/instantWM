//! Backend-agnostic mode transition functions.
//!
//! These functions manage only the state side of mode transitions (client mode,
//! float geometry, border width, monitor maximized slot). Backend-specific I/O
//! (configure_window, send_configure, move_resize) is left to the caller.

use crate::globals::Globals;
use crate::types::{BaseClientMode, WindowId};

/// Outcome of a maximize state transition.
pub enum MaximizedOutcome {
    Entered { base: BaseClientMode },
    Exited { base: BaseClientMode },
}

/// Outcome of a fullscreen state transition.
pub enum FullscreenOutcome {
    Entered { was_floating: bool },
    Exited,
}

/// Transition a window into or out of maximized mode.
///
/// Handles: mode transition, float_geo save, mon.maximized update.
/// Does NOT handle: move_resize, arrange, surface configure, raise.
pub fn set_maximized(
    globals: &mut Globals,
    win: WindowId,
    enter: bool,
) -> Option<MaximizedOutcome> {
    if enter {
        set_maximized_enter(globals, win)
    } else {
        set_maximized_exit(globals, win)
    }
}

fn set_maximized_enter(globals: &mut Globals, win: WindowId) -> Option<MaximizedOutcome> {
    let client = globals.clients.get_mut(&win)?;
    let base = client.mode.base_mode();

    // Save float geo if not already floating.
    if !client.mode.is_floating() {
        client.float_geo = client.geo;
    }

    client.mode = client.mode.as_maximized();

    // Update mon.maximized. Try the window's monitor first, fall back to selected.
    if let Some(mid) = globals.clients.monitor_id(win) {
        if let Some(mon) = globals.monitor_mut(mid) {
            mon.maximized = Some(win);
        }
    } else if let Some(mon) = globals.selected_monitor_mut_opt() {
        mon.maximized = Some(win);
    }

    Some(MaximizedOutcome::Entered { base })
}

fn set_maximized_exit(globals: &mut Globals, win: WindowId) -> Option<MaximizedOutcome> {
    let client = globals.clients.get_mut(&win)?;
    let base = client.mode.base_mode();
    client.mode = client.mode.restored();
    globals.clear_maximized_for(win);
    Some(MaximizedOutcome::Exited { base })
}

/// Transition a window into or out of fullscreen mode.
///
/// Handles: mode transition, border width save/restore.
/// Does NOT handle: move_resize, arrange, surface configure, _NET_WM_STATE.
pub fn set_fullscreen(
    globals: &mut Globals,
    win: WindowId,
    enter: bool,
) -> Option<FullscreenOutcome> {
    if enter {
        set_fullscreen_enter(globals, win)
    } else {
        set_fullscreen_exit(globals, win)
    }
}

fn set_fullscreen_enter(globals: &mut Globals, win: WindowId) -> Option<FullscreenOutcome> {
    let client = globals.clients.get_mut(&win)?;
    let was_floating = client.mode.is_floating();
    client.mode = client.mode.as_fullscreen();
    client.save_border_width();
    client.border_width = 0;
    Some(FullscreenOutcome::Entered { was_floating })
}

fn set_fullscreen_exit(globals: &mut Globals, win: WindowId) -> Option<FullscreenOutcome> {
    let client = globals.clients.get_mut(&win)?;
    client.mode = client.mode.restored();
    client.restore_border_width();
    Some(FullscreenOutcome::Exited)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::types::{Client, ClientMode};

    fn test_globals_with_client(mode: ClientMode) -> Globals {
        let mut g = Globals::default();
        let win = WindowId(1);
        let mut client = Client {
            win,
            border_width: 2,
            old_border_width: 2,
            ..Client::default()
        };
        client.mode = mode;
        g.clients.insert(win, client);
        g
    }

    #[test]
    fn set_maximized_enter_from_tiling() {
        let mut g = test_globals_with_client(ClientMode::Tiling);
        let win = WindowId(1);
        let outcome = set_maximized(&mut g, win, true).unwrap();
        match outcome {
            MaximizedOutcome::Entered { base } => assert_eq!(base, BaseClientMode::Tiling),
            _ => panic!("expected Entered"),
        }
        assert!(g.clients.get(&win).unwrap().mode.is_maximized());
    }

    #[test]
    fn set_maximized_exit_restores_tiling() {
        let mut g = test_globals_with_client(ClientMode::Tiling);
        let win = WindowId(1);
        set_maximized(&mut g, win, true);
        let outcome = set_maximized(&mut g, win, false).unwrap();
        match outcome {
            MaximizedOutcome::Exited { base } => assert_eq!(base, BaseClientMode::Tiling),
            _ => panic!("expected Exited"),
        }
        assert!(g.clients.get(&win).unwrap().mode.is_tiling());
    }

    #[test]
    fn set_maximized_roundtrip_floating() {
        let mut g = test_globals_with_client(ClientMode::Floating);
        let win = WindowId(1);
        set_maximized(&mut g, win, true);
        assert!(g.clients.get(&win).unwrap().mode.is_maximized());
        let outcome = set_maximized(&mut g, win, false).unwrap();
        match outcome {
            MaximizedOutcome::Exited { base } => assert_eq!(base, BaseClientMode::Floating),
            _ => panic!("expected Exited"),
        }
        assert!(g.clients.get(&win).unwrap().mode.is_floating());
    }

    #[test]
    fn set_fullscreen_enter_saves_border() {
        let mut g = test_globals_with_client(ClientMode::Tiling);
        let win = WindowId(1);
        let outcome = set_fullscreen(&mut g, win, true).unwrap();
        match outcome {
            FullscreenOutcome::Entered { was_floating } => assert!(!was_floating),
            _ => panic!("expected Entered"),
        }
        let c = g.clients.get(&win).unwrap();
        assert!(c.mode.is_true_fullscreen());
        assert_eq!(c.border_width, 0);
        assert_eq!(c.old_border_width, 2);
    }

    #[test]
    fn set_fullscreen_exit_restores_border() {
        let mut g = test_globals_with_client(ClientMode::Tiling);
        let win = WindowId(1);
        set_fullscreen(&mut g, win, true);
        let outcome = set_fullscreen(&mut g, win, false).unwrap();
        assert!(matches!(outcome, FullscreenOutcome::Exited));
        let c = g.clients.get(&win).unwrap();
        assert!(c.mode.is_tiling());
        assert_eq!(c.border_width, 2);
    }
}
