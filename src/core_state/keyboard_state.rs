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

    /// Replace the configuration-derived fields while keeping the active
    /// layout when it is still configured.
    ///
    /// A config reload is not a layout switch. The active entry is identified
    /// by name and variant rather than by index so reordering or inserting
    /// layouts does not move the user to a different keymap. If the active
    /// layout was removed, fall back to the first configured layout.
    pub fn reconfigure(
        &mut self,
        layouts: Vec<KeyboardLayout>,
        options: Option<String>,
        model: Option<String>,
        swap_escape: bool,
    ) {
        let active = self
            .layouts
            .get(self.current)
            .map(|layout| (layout.name.clone(), layout.variant.clone()));

        self.layouts = layouts;
        self.options = options;
        self.model = model;
        self.swap_escape = swap_escape;
        self.current = active
            .and_then(|(name, variant)| {
                self.layouts
                    .iter()
                    .position(|layout| layout.name == name && layout.variant == variant)
            })
            .unwrap_or(0);
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

#[cfg(test)]
mod tests {
    use super::*;

    fn layout(name: &str, variant: Option<&str>) -> KeyboardLayout {
        KeyboardLayout {
            name: name.to_string(),
            variant: variant.map(str::to_string),
        }
    }

    fn state_with(layouts: Vec<KeyboardLayout>, current: usize) -> KeyboardLayoutState {
        KeyboardLayoutState {
            layouts,
            current,
            ..Default::default()
        }
    }

    #[test]
    fn reconfigure_preserves_the_active_layout_across_reordering() {
        let mut state = state_with(
            vec![layout("us", None), layout("de", Some("nodeadkeys"))],
            1,
        );

        state.reconfigure(
            vec![
                layout("fr", None),
                layout("us", None),
                layout("de", Some("nodeadkeys")),
            ],
            Some("grp:alt_shift_toggle".into()),
            Some("pc105".into()),
            true,
        );

        assert_eq!(state.current, 2);
        assert_eq!(state.current_layout(), Some("de"));
        assert_eq!(state.current_variant(), "nodeadkeys");
        assert_eq!(state.options.as_deref(), Some("grp:alt_shift_toggle"));
        assert_eq!(state.model.as_deref(), Some("pc105"));
        assert!(state.swap_escape);
    }

    #[test]
    fn reconfigure_matches_the_variant_not_only_the_name() {
        let mut state = state_with(
            vec![layout("de", None), layout("de", Some("nodeadkeys"))],
            1,
        );

        state.reconfigure(
            vec![layout("de", Some("nodeadkeys")), layout("de", None)],
            None,
            None,
            false,
        );

        assert_eq!(state.current, 0);
        assert_eq!(state.current_variant(), "nodeadkeys");
    }

    #[test]
    fn reconfigure_falls_back_to_the_first_layout_when_active_is_removed() {
        let mut state = state_with(vec![layout("us", None), layout("de", None)], 1);

        state.reconfigure(
            vec![layout("fr", None), layout("us", None)],
            None,
            None,
            false,
        );

        assert_eq!(state.current, 0);
        assert_eq!(state.current_layout(), Some("fr"));
    }
}
