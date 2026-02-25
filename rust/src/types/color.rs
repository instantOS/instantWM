//! Color scheme types.
//!
//! Types for managing colors in the window manager UI.

use crate::drw::Clr;

/// A color scheme with foreground, background, and detail colors.
#[derive(Debug, Clone, PartialEq, Default)]
pub struct ColorScheme {
    /// Foreground color.
    pub fg: Clr,
    /// Background color.
    pub bg: Clr,
    /// Detail/accent color.
    pub detail: Clr,
}

impl ColorScheme {
    /// Create a new color scheme.
    pub fn new(fg: Clr, bg: Clr, detail: Clr) -> Self {
        Self { fg, bg, detail }
    }

    /// Create a color scheme from a vector of colors.
    ///
    /// Returns `None` if the vector has fewer than 3 elements.
    pub fn from_vec(vec: Vec<Clr>) -> Option<Self> {
        if vec.len() >= 3 {
            Some(Self {
                fg: vec[0].clone(),
                bg: vec[1].clone(),
                detail: vec[2].clone(),
            })
        } else {
            None
        }
    }

    /// Convert this color scheme to a vector.
    pub fn as_vec(&self) -> Vec<Clr> {
        vec![self.fg.clone(), self.bg.clone(), self.detail.clone()]
    }
}

/// Color scheme variants for different border states.
#[derive(Debug, Clone, PartialEq, Default)]
pub struct BorderScheme {
    /// Normal/unfocused border colors.
    pub normal: ColorScheme,
    /// Focused tiled window border colors.
    pub tile_focus: ColorScheme,
    /// Focused floating window border colors.
    pub float_focus: ColorScheme,
    /// Snap indicator border colors.
    pub snap: ColorScheme,
}

/// Color scheme for status bar elements.
#[derive(Debug, Clone, PartialEq, Default)]
pub struct StatusScheme {
    /// Foreground color.
    pub fg: Clr,
    /// Background color.
    pub bg: Clr,
    /// Detail/accent color.
    pub detail: Clr,
}

impl StatusScheme {
    /// Create a new status scheme.
    pub fn new(fg: Clr, bg: Clr, detail: Clr) -> Self {
        Self { fg, bg, detail }
    }

    /// Convert to a standard color scheme.
    pub fn as_color_scheme(&self) -> ColorScheme {
        ColorScheme {
            fg: self.fg.clone(),
            bg: self.bg.clone(),
            detail: self.detail.clone(),
        }
    }
}

/// Color schemes for tag buttons (hover and non-hover states).
#[derive(Debug, Clone, PartialEq, Default)]
pub struct TagSchemes {
    /// Schemes when not hovering.
    pub no_hover: Vec<ColorScheme>,
    /// Schemes when hovering.
    pub hover: Vec<ColorScheme>,
}

/// Color schemes for window title buttons.
#[derive(Debug, Clone, PartialEq, Default)]
pub struct WindowSchemes {
    /// Schemes when not hovering.
    pub no_hover: Vec<ColorScheme>,
    /// Schemes when hovering.
    pub hover: Vec<ColorScheme>,
}

/// Color schemes for close buttons.
#[derive(Debug, Clone, PartialEq, Default)]
pub struct CloseButtonSchemes {
    /// Schemes when not hovering.
    pub no_hover: Vec<ColorScheme>,
    /// Schemes when hovering.
    pub hover: Vec<ColorScheme>,
}

// =============================================================================
// Configuration String Types (for xresources/config loading)
// =============================================================================

/// Color scheme using string colors (before parsing).
#[derive(Debug, Clone, PartialEq)]
pub struct ColorSchemeStrings {
    /// Foreground color string.
    pub fg: &'static str,
    /// Background color string.
    pub bg: &'static str,
    /// Detail color string.
    pub detail: &'static str,
}

impl ColorSchemeStrings {
    /// Create a new color scheme from strings.
    pub fn new(fg: &'static str, bg: &'static str, detail: &'static str) -> Self {
        Self { fg, bg, detail }
    }

    /// Convert to a vector of strings.
    pub fn to_vec(&self) -> Vec<&'static str> {
        vec![self.fg, self.bg, self.detail]
    }
}

/// Tag color configurations using strings.
#[derive(Debug, Clone, PartialEq, Default)]
pub struct TagColorConfigs {
    /// Non-hover color configs.
    pub no_hover: Vec<ColorSchemeStrings>,
    /// Hover color configs.
    pub hover: Vec<ColorSchemeStrings>,
}

/// Window color configurations using strings.
#[derive(Debug, Clone, PartialEq, Default)]
pub struct WindowColorConfigs {
    /// Non-hover color configs.
    pub no_hover: Vec<ColorSchemeStrings>,
    /// Hover color configs.
    pub hover: Vec<ColorSchemeStrings>,
}

/// Close button color configurations using strings.
#[derive(Debug, Clone, PartialEq, Default)]
pub struct CloseButtonColorConfigs {
    /// Non-hover color configs.
    pub no_hover: Vec<ColorSchemeStrings>,
    /// Hover color configs.
    pub hover: Vec<ColorSchemeStrings>,
}

/// Border color configuration using strings.
#[derive(Debug, Clone, PartialEq)]
pub struct BorderColorConfig {
    /// Normal border colors.
    pub normal: ColorSchemeStrings,
    /// Focused tiled window colors.
    pub tile_focus: ColorSchemeStrings,
    /// Focused floating window colors.
    pub float_focus: ColorSchemeStrings,
    /// Snap indicator colors.
    pub snap: ColorSchemeStrings,
}

impl Default for BorderColorConfig {
    fn default() -> Self {
        Self {
            normal: ColorSchemeStrings::new("", "", ""),
            tile_focus: ColorSchemeStrings::new("", "", ""),
            float_focus: ColorSchemeStrings::new("", "", ""),
            snap: ColorSchemeStrings::new("", "", ""),
        }
    }
}

/// Status color configuration using strings.
#[derive(Debug, Clone, PartialEq)]
pub struct StatusColorConfig {
    /// Status bar colors.
    pub colors: ColorSchemeStrings,
}

impl Default for StatusColorConfig {
    fn default() -> Self {
        Self {
            colors: ColorSchemeStrings::new("", "", ""),
        }
    }
}
