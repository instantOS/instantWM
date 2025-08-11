use crate::config::Config;
use crate::types::{Rectangle, WindowId};
use crate::window_manager::{TagId, WindowManager};
use smithay::backend::renderer::{
    element::{Element, Id, Kind, RenderElement},
    gles::GlesRenderer,
    Frame, Renderer,
};
use smithay::utils::{Logical, Point, Rectangle as SmithayRectangle, Size};
use std::sync::{Arc, Mutex};

pub struct TopBar {
    pub geometry: Rectangle,
    pub config: Config,
    pub window_manager: Arc<Mutex<WindowManager>>,
    pub drag_target: Option<DragTarget>,
}

#[derive(Debug, Clone)]
pub struct DragTarget {
    pub tag_id: TagId,
    pub window_id: Option<WindowId>,
    pub position: Point<i32, Logical>,
}

impl TopBar {
    pub fn new(config: Config, screen_width: u32, window_manager: Arc<Mutex<WindowManager>>) -> Self {
        let height = config.top_bar.height as u32;
        Self {
            geometry: Rectangle {
                x: 0,
                y: 0,
                width: screen_width,
                height,
            },
            config,
            window_manager,
            drag_target: None,
        }
    }

    pub fn render(&self, renderer: &mut GlesRenderer) -> Result<Vec<TopBarElement>, Box<dyn std::error::Error>> {
        let mut elements = Vec::new();
        
        // Background
        let bg_element = TopBarElement::new(
            self.geometry.clone(),
            self.config.top_bar.background_color.clone(),
            TopBarElementType::Background,
        );
        elements.push(bg_element);
        
        // Tag buttons
        let tag_width = 60;
        let tag_height = self.geometry.height as u32 - 10;
        let tag_y = 5;
        
        let wm = self.window_manager.lock().unwrap();
        let mut x = 10;
        
        for (i, (tag_id, tag)) in wm.tags.iter().enumerate() {
            let is_active = tag_id == wm.current_tag;
            let has_windows = !tag.windows.is_empty();
            
            let color = if is_active {
                &self.config.top_bar.active_tag_color
            } else if has_windows {
                &self.config.top_bar.occupied_tag_color
            } else {
                &self.config.top_bar.empty_tag_color
            };
            
            let tag_element = TopBarElement::new(
                Rectangle {
                    x,
                    y: tag_y,
                    width: tag_width,
                    height: tag_height,
                },
                color.clone(),
                TopBarElementType::Tag {
                    tag_id,
                    name: tag.name.clone(),
                    is_active,
                    has_windows,
                },
            );
            elements.push(tag_element);
            
            x += tag_width + 5;
        }
        
        // Window titles
        let title_x = x + 20;
        let title_width = 200;
        
        if let Some(focused) = wm.get_focused_window() {
            if let Some(window) = wm.get_window(focused) {
                let title_element = TopBarElement::new(
                    Rectangle {
                        x: title_x,
                        y: tag_y,
                        width: title_width,
                        height: tag_height,
                    },
                    self.config.top_bar.text_color.clone(),
                    TopBarElementType::WindowTitle {
                        title: window.title.clone(),
                        class: window.class.clone(),
                    },
                );
                elements.push(title_element);
            }
        }
        
        Ok(elements)
    }

    pub fn handle_click(&mut self, x: i32, y: i32) -> bool {
        if !self.geometry.contains((x, y)) {
            return false;
        }
        
        let tag_width = 60;
        let tag_height = self.geometry.height as u32 - 10;
        let tag_y = 5;
        
        let mut tag_x = 10;
        
        let mut wm = self.window_manager.lock().unwrap();
        
        for (tag_id, _) in wm.tags.iter() {
            let tag_rect = Rectangle {
                x: tag_x,
                y: tag_y,
                width: tag_width,
                height: tag_height,
            };
            
            if tag_rect.contains((x, y)) {
                wm.switch_tag(tag_id);
                return true;
            }
            
            tag_x += tag_width + 5;
        }
        
        false
    }

    pub fn handle_drag_start(&mut self, x: i32, y: i32) -> bool {
        if !self.geometry.contains((x, y)) {
            return false;
        }
        
        // Check if dragging over a tag
        let tag_width = 60;
        let tag_height = self.geometry.height as u32 - 10;
        let tag_y = 5;
        
        let mut tag_x = 10;
        
        let wm = self.window_manager.lock().unwrap();
        
        for (tag_id, _) in wm.tags.iter() {
            let tag_rect = Rectangle {
                x: tag_x,
                y: tag_y,
                width: tag_width,
                height: tag_height,
            };
            
            if tag_rect.contains((x, y)) {
                self.drag_target = Some(DragTarget {
                    tag_id,
                    window_id: None,
                    position: (x, y).into(),
                });
                return true;
            }
            
            tag_x += tag_width + 5;
        }
        
        false
    }

    pub fn handle_drag_end(&mut self, window_id: WindowId) -> bool {
        if let Some(drag_target) = &self.drag_target {
            let mut wm = self.window_manager.lock().unwrap();
            let _ = wm.move_window_to_tag(window_id, drag_target.tag_id);
            self.drag_target = None;
            return true;
        }
        false
    }
}

pub struct TopBarElement {
    pub geometry: Rectangle,
    pub color: String,
    pub element_type: TopBarElementType,
}

#[derive(Debug, Clone)]
pub enum TopBarElementType {
    Background,
    Tag {
        tag_id: TagId,
        name: String,
        is_active: bool,
        has_windows: bool,
    },
    WindowTitle {
        title: String,
        class: String,
    },
}

impl TopBarElement {
    pub fn new(geometry: Rectangle, color: String, element_type: TopBarElementType) -> Self {
        Self {
            geometry,
            color,
            element_type,
        }
    }
}

impl RenderElement<GlesRenderer> for TopBarElement {
    fn id(&self) -> Id {
        Id::new(0) // TODO: Generate proper IDs
    }

    fn location(&self, _scale: smithay::utils::Scale<f64>) -> Point<i32, Logical> {
        (self.geometry.x, self.geometry.y).into()
    }

    fn src(&self) -> Option<SmithayRectangle<f64, Logical>> {
        None
    }

    fn geometry(&self, _scale: smithay::utils::Scale<f64>) -> SmithayRectangle<i32, Logical> {
        SmithayRectangle::from_loc_and_size(
            (self.geometry.x, self.geometry.y),
            (self.geometry.width as i32, self.geometry.height as i32),
        )
    }

    fn accumulated_damage(
        &self,
        _scale: smithay::utils::Scale<f64>,
        _for_values: Option<smithay::utils::Scale<f64>>,
    ) -> Vec<SmithayRectangle<i32, Logical>> {
        vec![self.geometry(smithay::utils::Scale::from(1.0))]
    }

    fn draw(
        &self,
        _frame: &mut <GlesRenderer as Renderer>::Frame<'_>,
        _scale: smithay::utils::Scale<f64>,
        _damage: &[SmithayRectangle<i32, Logical>],
        _log: &slog::Logger,
    ) -> Result<(), smithay::backend::renderer::Error> {
        // TODO: Implement actual rendering
        Ok(())
    }

    fn underlying_storage(&self, _renderer: &mut GlesRenderer) -> Option<smithay::backend::renderer::element::UnderlyingStorage> {
        None
    }
}