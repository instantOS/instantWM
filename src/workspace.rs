use crate::layouts::{Layout, TileLayout};
use crate::Window;
use smithay::utils::Rectangle;

pub struct Workspace {
    windows: Vec<Window>,
    layout: Box<dyn Layout>,
}

impl Workspace {
    pub fn new() -> Self {
        Self {
            windows: Vec::new(),
            layout: Box::new(TileLayout::default()),
        }
    }

    pub fn add_window(&mut self, window: Window) {
        self.windows.push(window);
    }

    pub fn arrange(&mut self, area: Rectangle<i32, smithay::utils::Physical>) {
        self.layout.arrange(&mut self.windows, area);
    }

    pub fn windows(&self) -> &[Window] {
        &self.windows
    }

    pub fn layout_symbol(&self) -> &str {
        self.layout.symbol()
    }
}
