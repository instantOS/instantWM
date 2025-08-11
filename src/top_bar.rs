//! Top bar implementation for InstantWM
//!
//! This module handles the top bar that displays tag information, window titles,
//! system tray, and other status information.

use crate::types::Config;
use crate::window_manager::WindowManager;
use smithay::utils::Rectangle;
use std::sync::{Arc, Mutex};
use tracing::debug;

/// Top bar component for displaying workspace and window information
pub struct TopBar {
    pub config: Config,
    pub screen_width: u32,
    pub window_manager: Arc<Mutex<WindowManager>>,
    pub height: u32,
    pub visible: bool,
    pub background_color: [f32; 4], // RGBA
    pub text_color: [f32; 4],       // RGBA
}

#[derive(Debug, Clone)]
pub struct TopBarSegment {
    pub text: String,
    pub x: i32,
    pub width: u32,
    pub clickable: bool,
    pub action: Option<String>,
}

#[derive(Debug, Clone)]
pub struct TopBarState {
    pub segments: Vec<TopBarSegment>,
    pub total_width: u32,
    pub needs_redraw: bool,
}

impl TopBar {
    pub fn new(
        config: Config,
        screen_width: u32,
        window_manager: Arc<Mutex<WindowManager>>,
    ) -> Self {
        let height = config.appearance.bar_height;
        let background_color = parse_color(&config.appearance.bar_background);
        let text_color = parse_color(&config.appearance.bar_foreground);

        Self {
            config,
            screen_width,
            window_manager,
            height,
            visible: true,
            background_color,
            text_color,
        }
    }

    /// Toggle bar visibility
    pub fn toggle_visibility(&mut self) {
        self.visible = !self.visible;
        debug!("Top bar visibility toggled to: {}", self.visible);
    }

    /// Update the top bar content based on current window manager state
    pub fn update(&mut self) -> TopBarState {
        let mut segments = Vec::new();
        let mut x_offset = 10; // Left padding

        if let Ok(wm) = self.window_manager.lock() {
            // Add tag segments
            for (i, tag) in wm.tags.iter().enumerate() {
                let is_current = i as u32 == wm.current_tag;
                let has_windows = !tag.windows.is_empty();

                let tag_text = if is_current {
                    format!("[{}]", tag.name)
                } else if has_windows {
                    format!("({})", tag.name)
                } else {
                    tag.name.clone()
                };

                let tag_width =
                    estimate_text_width(&tag_text, self.config.appearance.bar_font_size);

                segments.push(TopBarSegment {
                    text: tag_text,
                    x: x_offset,
                    width: tag_width,
                    clickable: true,
                    action: Some(format!("switch_tag {}", i + 1)),
                });

                x_offset += tag_width as i32 + 5; // Spacing between tags
            }

            // Add separator
            x_offset += 20;
            segments.push(TopBarSegment {
                text: "|".to_string(),
                x: x_offset,
                width: 10,
                clickable: false,
                action: None,
            });
            x_offset += 15;

            // Add current layout
            let layout_text = format!("{:?}", wm.current_layout());
            let layout_width =
                estimate_text_width(&layout_text, self.config.appearance.bar_font_size);

            segments.push(TopBarSegment {
                text: layout_text,
                x: x_offset,
                width: layout_width,
                clickable: true,
                action: Some("cycle_layout".to_string()),
            });
            x_offset += layout_width as i32 + 20;

            // Add focused window title
            if let Some(_focused) = wm.get_focused_window() {
                let title = "Untitled".to_string(); // TODO: Implement proper title access
                let max_title_width = (self.screen_width as i32 - x_offset - 200).max(0) as u32; // Reserve space for right side
                let truncated_title = if title.len() > 50 {
                    format!("{}...", &title[..47])
                } else {
                    title
                };

                let title_width =
                    estimate_text_width(&truncated_title, self.config.appearance.bar_font_size)
                        .min(max_title_width);

                segments.push(TopBarSegment {
                    text: truncated_title,
                    x: x_offset,
                    width: title_width,
                    clickable: false,
                    action: None,
                });
            }

            // Add system info on the right side
            let time_text = get_current_time();
            let time_width = estimate_text_width(&time_text, self.config.appearance.bar_font_size);
            let time_x = self.screen_width as i32 - time_width as i32 - 10;

            segments.push(TopBarSegment {
                text: time_text,
                x: time_x,
                width: time_width,
                clickable: false,
                action: None,
            });
        }

        TopBarState {
            segments,
            total_width: self.screen_width,
            needs_redraw: true,
        }
    }

    /// Handle click on the top bar
    pub fn handle_click(&mut self, x: i32, y: i32) -> Option<String> {
        if !self.visible || y < 0 || y >= self.height as i32 {
            return None;
        }

        let state = self.update();

        for segment in &state.segments {
            if segment.clickable && x >= segment.x && x < segment.x + segment.width as i32 {
                debug!("Top bar click on segment: {}", segment.text);
                return segment.action.clone();
            }
        }

        None
    }

    /// Get the geometry of the top bar
    pub fn geometry(&self) -> Rectangle<i32, smithay::utils::Logical> {
        if self.visible {
            Rectangle::new(
                (0, 0).into(),
                (self.screen_width as i32, self.height as i32).into(),
            )
        } else {
            Rectangle::new((0, 0).into(), (0, 0).into())
        }
    }

    /// Check if the top bar is visible
    pub fn is_visible(&self) -> bool {
        self.visible
    }

    /// Get the height of the top bar (0 if hidden)
    pub fn effective_height(&self) -> u32 {
        if self.visible {
            self.height
        } else {
            0
        }
    }

    /// Update screen width when display changes
    pub fn update_screen_width(&mut self, width: u32) {
        self.screen_width = width;
        debug!("Top bar screen width updated to: {}", width);
    }

    /// Get background color
    pub fn background_color(&self) -> [f32; 4] {
        self.background_color
    }

    /// Get text color
    pub fn text_color(&self) -> [f32; 4] {
        self.text_color
    }
}

/// Parse color string (hex format) to RGBA float array
fn parse_color(color_str: &str) -> [f32; 4] {
    if let Some(hex) = color_str.strip_prefix('#') {
        if hex.len() == 6 {
            if let (Ok(r), Ok(g), Ok(b)) = (
                u8::from_str_radix(&hex[0..2], 16),
                u8::from_str_radix(&hex[2..4], 16),
                u8::from_str_radix(&hex[4..6], 16),
            ) {
                return [r as f32 / 255.0, g as f32 / 255.0, b as f32 / 255.0, 1.0];
            }
        }
    }

    // Default to black if parsing fails
    tracing::warn!("Failed to parse color: {}", color_str);
    [0.0, 0.0, 0.0, 1.0]
}

/// Estimate text width (rough approximation)
fn estimate_text_width(text: &str, font_size: u32) -> u32 {
    // Very rough estimate: average character width is about 0.6 * font_size
    let char_width = (font_size as f32 * 0.6) as u32;
    (text.len() as u32 * char_width).max(20)
}

/// Get current time as string
fn get_current_time() -> String {
    use std::time::{SystemTime, UNIX_EPOCH};

    match SystemTime::now().duration_since(UNIX_EPOCH) {
        Ok(duration) => {
            let timestamp = duration.as_secs();
            let hours = (timestamp / 3600) % 24;
            let minutes = (timestamp / 60) % 60;
            format!("{:02}:{:02}", hours, minutes)
        }
        Err(_) => "??:??".to_string(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::types::Rectangle;

    #[test]
    fn test_color_parsing() {
        assert_eq!(parse_color("#FF0000"), [1.0, 0.0, 0.0, 1.0]); // Red
        assert_eq!(parse_color("#00FF00"), [0.0, 1.0, 0.0, 1.0]); // Green
        assert_eq!(parse_color("#0000FF"), [0.0, 0.0, 1.0, 1.0]); // Blue
        assert_eq!(parse_color("#FFFFFF"), [1.0, 1.0, 1.0, 1.0]); // White
        assert_eq!(parse_color("#000000"), [0.0, 0.0, 0.0, 1.0]); // Black
    }

    #[test]
    fn test_text_width_estimation() {
        assert!(estimate_text_width("Hello", 12) > 0);
        assert!(estimate_text_width("Hello World", 12) > estimate_text_width("Hello", 12));
    }

    #[test]
    fn test_top_bar_geometry() {
        let config = Config::default();
        let screen_geometry = smithay::utils::Rectangle::from_size((1920, 1080).into());
        let window_manager = Arc::new(Mutex::new(
            WindowManager::new(config.clone(), screen_geometry).unwrap(),
        ));

        let top_bar = TopBar::new(config, 1920, window_manager);
        let geometry = top_bar.geometry();

        assert_eq!(geometry.loc.x, 0);
        assert_eq!(geometry.loc.y, 0);
        assert_eq!(geometry.size.w, 1920);
        assert_eq!(geometry.size.h as u32, top_bar.height);
    }

    #[test]
    fn test_top_bar_visibility_toggle() {
        let config = Config::default();
        let screen_geometry = smithay::utils::Rectangle::from_size((1920, 1080).into());
        let window_manager = Arc::new(Mutex::new(
            WindowManager::new(config.clone(), screen_geometry).unwrap(),
        ));

        let mut top_bar = TopBar::new(config, 1920, window_manager);
        assert!(top_bar.is_visible());

        top_bar.toggle_visibility();
        assert!(!top_bar.is_visible());
        assert_eq!(top_bar.effective_height(), 0);

        top_bar.toggle_visibility();
        assert!(top_bar.is_visible());
        assert!(top_bar.effective_height() > 0);
    }
}
