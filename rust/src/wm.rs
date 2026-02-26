//! Window-manager root object.
//!
//! `Wm` owns all runtime state and the active backend.

use crate::backend::x11::X11Backend;
use crate::contexts::WmCtx;
use crate::globals::Globals;

pub struct Wm {
    pub g: Globals,
    pub x11: X11Backend,
    pub running: bool,
    pub bar: crate::bar::BarState,
    pub focus: crate::client::focus::FocusState,
}

impl Wm {
    pub fn new(x11: X11Backend) -> Self {
        Self {
            g: Globals::default(),
            x11,
            running: true,
            bar: crate::bar::BarState::default(),
            focus: crate::client::focus::FocusState::default(),
        }
    }

    pub fn quit(&mut self) {
        self.running = false;
    }

    pub fn ctx(&mut self) -> WmCtx<'_> {
        WmCtx::new(
            &mut self.g,
            &self.x11.conn,
            self.x11.screen_num,
            &mut self.running,
            &mut self.bar,
            &mut self.focus,
        )
    }
}
