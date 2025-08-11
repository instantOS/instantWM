use crate::config::Config;
use crate::error::Result;
use crate::types::{LayoutConfig, LayoutType, Rectangle, Tag, WindowId, WindowState};
use slotmap::{SlotMap, SecondaryMap};
use std::collections::HashMap;

pub struct WindowManager {
    pub tags: SlotMap<TagId, Tag>,
    pub windows: SlotMap<WindowId, WindowState>,
    pub current_tag: TagId,
    pub config: Config,
    pub screen_geometry: Rectangle,
}

new_key_type! {
    pub struct TagId;
}

impl WindowManager {
    pub fn new(config: Config, screen_geometry: Rectangle) -> Result<Self> {
        let mut tags = SlotMap::with_key();
        let mut tag_map = HashMap::new();
        
        // Create tags based on configuration
        for (i, name) in config.tags.names.iter().enumerate() {
            let layout_name = config.tags.layouts.get(i).unwrap_or(&config.tags.layouts[0]);
            let layout = config.layouts.get(layout_name)
                .cloned()
                .unwrap_or_else(|| LayoutConfig {
                    layout_type: LayoutType::Tiling,
                    master_ratio: 0.6,
                    master_count: 1,
                });
            
            let tag = Tag {
                id: TagId::new(i as u32),
                name: name.clone(),
                layout,
                windows: Vec::new(),
                focused_window: None,
            };
            
            let tag_id = tags.insert(tag);
            tag_map.insert(name.clone(), tag_id);
        }
        
        let current_tag = tags.keys().next().unwrap();
        
        Ok(Self {
            tags,
            windows: SlotMap::with_key(),
            current_tag,
            config,
            screen_geometry,
        })
    }

    pub fn add_window(&mut self, title: String, class: String, geometry: Rectangle) -> WindowId {
        let window = WindowState {
            id: WindowId::new(0), // Will be updated by slotmap
            title,
            class,
            floating: false,
            minimized: false,
            fullscreen: false,
            tag: self.current_tag,
            geometry: geometry.clone(),
            requested_geometry: geometry,
        };
        
        let id = self.windows.insert(window);
        let window = &mut self.windows[id];
        window.id = id;
        
        // Add to current tag
        let tag = &mut self.tags[self.current_tag];
        tag.windows.push(id);
        tag.focused_window = Some(id);
        
        self.arrange_tag(self.current_tag);
        id
    }

    pub fn remove_window(&mut self, id: WindowId) -> Result<()> {
        if let Some(window) = self.windows.remove(id) {
            // Remove from tag
            let tag = &mut self.tags[window.tag];
            tag.windows.retain(|&wid| wid != id);
            
            if tag.focused_window == Some(id) {
                tag.focused_window = tag.windows.last().copied();
            }
            
            self.arrange_tag(window.tag);
        }
        Ok(())
    }

    pub fn switch_tag(&mut self, tag_id: TagId) {
        if self.tags.contains_key(tag_id) {
            self.current_tag = tag_id;
            // Hide windows from old tag, show windows from new tag
            self.update_window_visibility();
        }
    }

    pub fn move_window_to_tag(&mut self, window_id: WindowId, tag_id: TagId) -> Result<()> {
        if !self.windows.contains_key(window_id) || !self.tags.contains_key(tag_id) {
            return Ok(());
        }
        
        let window = &mut self.windows[window_id];
        let old_tag = window.tag;
        
        // Remove from old tag
        let old_tag_obj = &mut self.tags[old_tag];
        old_tag_obj.windows.retain(|&wid| wid != window_id);
        if old_tag_obj.focused_window == Some(window_id) {
            old_tag_obj.focused_window = old_tag_obj.windows.last().copied();
        }
        
        // Add to new tag
        window.tag = tag_id;
        let new_tag = &mut self.tags[tag_id];
        new_tag.windows.push(window_id);
        new_tag.focused_window = Some(window_id);
        
        self.arrange_tag(old_tag);
        self.arrange_tag(tag_id);
        self.update_window_visibility();
        
        Ok(())
    }

    pub fn toggle_floating(&mut self, window_id: WindowId) -> Result<()> {
        if let Some(window) = self.windows.get_mut(window_id) {
            window.floating = !window.floating;
            self.arrange_tag(window.tag);
        }
        Ok(())
    }

    pub fn set_layout(&mut self, tag_id: TagId, layout: LayoutConfig) {
        if let Some(tag) = self.tags.get_mut(tag_id) {
            tag.layout = layout;
            self.arrange_tag(tag_id);
        }
    }

    pub fn arrange_tag(&mut self, tag_id: TagId) {
        let tag = &mut self.tags[tag_id];
        let windows: Vec<&mut WindowState> = tag.windows
            .iter()
            .filter_map(|&id| self.windows.get_mut(id))
            .filter(|w| !w.floating && !w.minimized && !w.fullscreen)
            .collect();
        
        if windows.is_empty() {
            return;
        }
        
        let screen = self.screen_geometry;
        let gaps = self.config.appearance.gap_size as i32;
        let inner_gap = self.config.appearance.inner_gap as i32;
        
        let usable_width = screen.width as i32 - 2 * gaps;
        let usable_height = screen.height as i32 - 2 * gaps;
        
        match tag.layout.layout_type {
            LayoutType::Tiling => self.arrange_tiling(&windows, usable_width, usable_height, gaps, inner_gap),
            LayoutType::Floating => {} // Floating windows keep their positions
            LayoutType::Monocle => self.arrange_monocle(&windows, usable_width, usable_height, gaps),
        }
    }

    fn arrange_tiling(
        &mut self,
        windows: &[&mut WindowState],
        usable_width: i32,
        usable_height: i32,
        gaps: i32,
        inner_gap: i32,
    ) {
        let master_count = self.tags[self.current_tag].layout.master_count.min(windows.len());
        let master_width = (usable_width as f32 * self.tags[self.current_tag].layout.master_ratio) as i32;
        let stack_width = usable_width - master_width - inner_gap;
        
        // Master area
        for (i, window) in windows.iter_mut().take(master_count).enumerate() {
            let height = usable_height / master_count as i32 - inner_gap * (master_count as i32 - 1) / master_count as i32;
            let y = gaps + i as i32 * (height + inner_gap);
            
            window.geometry = Rectangle {
                x: gaps,
                y,
                width: master_width as u32,
                height: height as u32,
            };
        }
        
        // Stack area
        let stack_start = master_count;
        let stack_count = windows.len() - stack_start;
        if stack_count > 0 {
            let stack_height = usable_height / stack_count as i32 - inner_gap * (stack_count as i32 - 1) / stack_count as i32;
            
            for (i, window) in windows.iter_mut().skip(stack_start).enumerate() {
                let y = gaps + i as i32 * (stack_height + inner_gap);
                
                window.geometry = Rectangle {
                    x: gaps + master_width + inner_gap,
                    y,
                    width: stack_width as u32,
                    height: stack_height as u32,
                };
            }
        }
    }

    fn arrange_monocle(
        &mut self,
        windows: &[&mut WindowState],
        usable_width: i32,
        usable_height: i32,
        gaps: i32,
    ) {
        for window in windows {
            window.geometry = Rectangle {
                x: gaps,
                y: gaps,
                width: usable_width as u32,
                height: usable_height as u32,
            };
        }
    }

    fn update_window_visibility(&mut self) {
        // This would handle showing/hiding windows based on current tag
        // In Wayland, we'd use protocol extensions to minimize/unminimize
    }

    pub fn get_focused_window(&self) -> Option<WindowId> {
        self.tags[self.current_tag].focused_window
    }

    pub fn focus_window(&mut self, window_id: WindowId) -> Result<()> {
        if let Some(window) = self.windows.get(window_id) {
            let tag_id = window.tag;
            self.tags[tag_id].focused_window = Some(window_id);
        }
        Ok(())
    }

    pub fn get_windows_for_tag(&self, tag_id: TagId) -> Vec<WindowId> {
        self.tags[tag_id].windows.clone()
    }

    pub fn get_window(&self, id: WindowId) -> Option<&WindowState> {
        self.windows.get(id)
    }

    pub fn get_window_mut(&mut self, id: WindowId) -> Option<&mut WindowState> {
        self.windows.get_mut(id)
    }
}