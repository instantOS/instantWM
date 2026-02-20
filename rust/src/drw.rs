use x11rb::protocol::xproto::Window;

// TODO: Port drawing primitives from drw.c

pub struct Drw {
    // TODO: Add display, screen, root, drawable, gc, fontset, etc.
}

pub struct Cur {
    pub cursor: u32,
}

pub struct Clr {
    pub rgb: u32,
}

impl Drw {
    pub fn new() -> Self {
        Self {}
    }

    // TODO: drw_fontset_getwidth
    pub fn fontset_getwidth(&self, _text: &str) -> i32 {
        0
    }

    // TODO: drw_text
    pub fn text(&mut self, _x: i32, _y: i32, _w: u32, _h: u32, _invert: bool, _text: &str) -> i32 {
        0
    }

    // TODO: drw_rect
    pub fn rect(&mut self, _x: i32, _y: i32, _w: u32, _h: u32, _filled: bool, _invert: bool) {}

    // TODO: drw_map
    pub fn map(&self, _win: Window, _x: i32, _y: i32, _w: u32, _h: u32) {}

    // TODO: drw_setscheme
    pub fn setscheme(&mut self, _scheme: &Clr) {}
}
