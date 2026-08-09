use super::*;

/// Keyboard (XKB) layout runtime state.
#[derive(Debug, Clone, Default)]
pub struct KeyboardLayoutState {
    /// Configured XKB layouts with optional variants.
    pub layouts: Vec<KeyboardLayout>,
    /// XKB options string.
    pub options: Option<String>,
    /// XKB model string.
    pub model: Option<String>,
    /// Swap Caps Lock and Escape.
    pub swap_escape: bool,
    /// Index of the currently active layout in `layouts`.
    pub current: usize,
}

impl KeyboardLayoutState {
    pub fn is_empty(&self) -> bool {
        self.layouts.is_empty()
    }

    pub fn len(&self) -> usize {
        self.layouts.len()
    }

    pub fn layout(&self, index: usize) -> Option<&KeyboardLayout> {
        self.layouts.get(index)
    }

    pub fn find_layout_index(&self, name: &str) -> Option<usize> {
        self.layouts.iter().position(|layout| layout.name == name)
    }

    pub fn reset_layouts(&mut self, layouts: Vec<KeyboardLayout>) {
        self.layouts = layouts;
        self.current = 0;
    }

    pub fn add_layout(&mut self, layout: KeyboardLayout) -> Result<usize, String> {
        if self.find_layout_index(&layout.name).is_some() {
            return Err(format!("layout '{}' already exists", layout.name));
        }

        let new_index = self.layouts.len();
        self.layouts.push(layout);
        Ok(new_index)
    }

    pub fn remove_layout(&mut self, index: usize) -> Result<(), String> {
        if self.layouts.len() == 1 {
            return Err("cannot remove the last layout".to_string());
        }

        self.layouts.remove(index);

        if index < self.current {
            self.current -= 1;
        } else if index == self.current && self.current >= self.layouts.len() {
            self.current = self.layouts.len() - 1;
        }

        Ok(())
    }

    /// The currently active layout name, or `None` if no layouts are configured.
    pub fn current_layout(&self) -> Option<&str> {
        self.layouts.get(self.current).map(|l| l.name.as_str())
    }

    /// The variant for the currently active layout, or empty string.
    pub fn current_variant(&self) -> &str {
        self.layouts
            .get(self.current)
            .and_then(|l| l.variant.as_deref())
            .unwrap_or("")
    }

    /// Format the currently active layout for status and IPC output.
    pub fn status(&self) -> String {
        if self.is_empty() {
            return "no layouts configured".to_string();
        }
        let current_name = self.current_layout().unwrap_or("unknown");
        let variant = self.current_variant();
        let variant = if variant.is_empty() {
            String::new()
        } else {
            format!(" ({variant})")
        };
        format!(
            "{}/{}: {}{}",
            self.current + 1,
            self.len(),
            current_name,
            variant
        )
    }
}
