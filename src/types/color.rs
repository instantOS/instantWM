//! Color scheme types.
//!
//! Types for managing colors in the window manager UI.

use crate::backend::x11::draw::Color;
use crate::bar::color::{Rgba, deserialize_hex_color, serialize_hex_color};
use serde::{Deserialize, Serialize};

// =============================================================================
// Scheme enums - typed identifiers for color sets
// =============================================================================

/// Whether the cursor is hovering over the element.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SchemeHover {
    NoHover,
    Hover,
}

impl SchemeHover {
    pub const fn from_index(index: usize) -> Option<Self> {
        match index {
            0 => Some(Self::NoHover),
            1 => Some(Self::Hover),
            _ => None,
        }
    }
}

/// State of a tag button in the bar.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SchemeTag {
    /// No clients on this tag.
    Inactive,
    /// Has clients but not focused on this monitor.
    Filled,
    /// Active tag on the focused monitor.
    Focus,
    /// Active tag on an unfocused monitor.
    NoFocus,
    /// Empty / special state.
    Empty,
    /// Urgent state.
    Urgent,
}

impl SchemeTag {
    pub const fn from_index(index: usize) -> Option<Self> {
        match index {
            0 => Some(Self::Inactive),
            1 => Some(Self::Filled),
            2 => Some(Self::Focus),
            3 => Some(Self::NoFocus),
            4 => Some(Self::Empty),
            5 => Some(Self::Urgent),
            _ => None,
        }
    }
}

/// State of a window title button in the bar.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SchemeWin {
    Focus,
    Normal,
    Minimized,
    Sticky,
    StickyFocus,
    Overlay,
    OverlayFocus,
    Urgent,
}

impl SchemeWin {
    pub const fn from_index(index: usize) -> Option<Self> {
        match index {
            0 => Some(Self::Focus),
            1 => Some(Self::Normal),
            2 => Some(Self::Minimized),
            3 => Some(Self::Sticky),
            4 => Some(Self::StickyFocus),
            5 => Some(Self::Overlay),
            6 => Some(Self::OverlayFocus),
            7 => Some(Self::Urgent),
            _ => None,
        }
    }
}

/// State of the close button widget.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SchemeClose {
    Normal,
    Locked,
    Fullscreen,
}

impl SchemeClose {
    pub const fn from_index(index: usize) -> Option<Self> {
        match index {
            0 => Some(Self::Normal),
            1 => Some(Self::Locked),
            2 => Some(Self::Fullscreen),
            _ => None,
        }
    }
}

/// State of the window border.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SchemeBorder {
    Normal,
    TileFocus,
    FloatFocus,
    Snap,
}

impl SchemeBorder {
    pub const fn from_index(index: usize) -> Option<Self> {
        match index {
            0 => Some(Self::Normal),
            1 => Some(Self::TileFocus),
            2 => Some(Self::FloatFocus),
            3 => Some(Self::Snap),
            _ => None,
        }
    }
}

/// A color scheme with foreground, background, and detail colors.
#[derive(Debug, Clone, PartialEq)]
pub struct ColorScheme {
    /// Foreground color.
    pub fg: Color,
    /// Background color.
    pub bg: Color,
    /// Detail/accent color.
    pub detail: Color,
}

impl ColorScheme {
    /// Create a new color scheme.
    pub fn new(fg: Color, bg: Color, detail: Color) -> Self {
        Self { fg, bg, detail }
    }

    /// Create a color scheme from a single color (replicated to fg, bg, detail).
    ///
    /// Useful for things like borders that only need one color.
    pub fn from_single(color: Color) -> Self {
        Self {
            fg: color.clone(),
            bg: color.clone(),
            detail: color,
        }
    }

    /// Create a color scheme from a vector of colors.
    ///
    /// Returns `None` if the vector has fewer than 3 elements.
    pub fn from_vec(vec: Vec<Color>) -> Option<Self> {
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
    pub fn as_vec(&self) -> Vec<Color> {
        vec![self.fg.clone(), self.bg.clone(), self.detail.clone()]
    }

    pub fn is_zeroed(&self) -> bool {
        self.fg.color.pixel == 0
    }
}

impl Default for ColorScheme {
    fn default() -> Self {
        let zero = Color::default();
        Self {
            fg: zero.clone(),
            bg: zero.clone(),
            detail: zero,
        }
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
    pub fg: Color,
    /// Background color.
    pub bg: Color,
    /// Detail/accent color.
    pub detail: Color,
}

impl StatusScheme {
    /// Create a new status scheme.
    pub fn new(fg: Color, bg: Color, detail: Color) -> Self {
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

// =============================================================================
// Configuration RGBA Types (for config loading)
// =============================================================================

/// Color scheme with pre-parsed RGBA values.
///
/// Colors are parsed once at config load time via serde, not at runtime.
#[derive(Debug, Clone, Copy, PartialEq, Deserialize, Serialize)]
#[serde(default)]
pub struct ColorSchemeRgba {
    /// Foreground color.
    #[serde(
        deserialize_with = "deserialize_hex_color",
        serialize_with = "serialize_hex_color"
    )]
    pub fg: Rgba,
    /// Background color.
    #[serde(
        deserialize_with = "deserialize_hex_color",
        serialize_with = "serialize_hex_color"
    )]
    pub bg: Rgba,
    /// Detail color.
    #[serde(
        deserialize_with = "deserialize_hex_color",
        serialize_with = "serialize_hex_color"
    )]
    pub detail: Rgba,
}

impl Default for ColorSchemeRgba {
    fn default() -> Self {
        Self::empty()
    }
}

/// Tag scheme groupings (non-hover or hover).
#[derive(Debug, Clone, PartialEq, Deserialize, Serialize, Default)]
#[serde(default)]
pub struct TagColorSet {
    pub inactive: ColorSchemeRgba,
    pub filled: ColorSchemeRgba,
    pub focus: ColorSchemeRgba,
    pub nofocus: ColorSchemeRgba,
    pub empty: ColorSchemeRgba,
    pub urgent: ColorSchemeRgba,
}

impl TagColorSet {
    pub fn scheme(&self, scheme: SchemeTag) -> &ColorSchemeRgba {
        match scheme {
            SchemeTag::Inactive => &self.inactive,
            SchemeTag::Filled => &self.filled,
            SchemeTag::Focus => &self.focus,
            SchemeTag::NoFocus => &self.nofocus,
            SchemeTag::Empty => &self.empty,
            SchemeTag::Urgent => &self.urgent,
        }
    }

    pub fn scheme_mut(&mut self, scheme: SchemeTag) -> &mut ColorSchemeRgba {
        match scheme {
            SchemeTag::Inactive => &mut self.inactive,
            SchemeTag::Filled => &mut self.filled,
            SchemeTag::Focus => &mut self.focus,
            SchemeTag::NoFocus => &mut self.nofocus,
            SchemeTag::Empty => &mut self.empty,
            SchemeTag::Urgent => &mut self.urgent,
        }
    }
}

/// Window scheme groupings (non-hover or hover).
#[derive(Debug, Clone, PartialEq, Deserialize, Serialize, Default)]
#[serde(default)]
pub struct WindowColorSet {
    pub focus: ColorSchemeRgba,
    pub normal: ColorSchemeRgba,
    pub minimized: ColorSchemeRgba,
    pub sticky: ColorSchemeRgba,
    pub sticky_focus: ColorSchemeRgba,
    pub overlay: ColorSchemeRgba,
    pub overlay_focus: ColorSchemeRgba,
    pub urgent: ColorSchemeRgba,
}

impl WindowColorSet {
    pub fn scheme(&self, scheme: SchemeWin) -> &ColorSchemeRgba {
        match scheme {
            SchemeWin::Focus => &self.focus,
            SchemeWin::Normal => &self.normal,
            SchemeWin::Minimized => &self.minimized,
            SchemeWin::Sticky => &self.sticky,
            SchemeWin::StickyFocus => &self.sticky_focus,
            SchemeWin::Overlay => &self.overlay,
            SchemeWin::OverlayFocus => &self.overlay_focus,
            SchemeWin::Urgent => &self.urgent,
        }
    }

    pub fn scheme_mut(&mut self, scheme: SchemeWin) -> &mut ColorSchemeRgba {
        match scheme {
            SchemeWin::Focus => &mut self.focus,
            SchemeWin::Normal => &mut self.normal,
            SchemeWin::Minimized => &mut self.minimized,
            SchemeWin::Sticky => &mut self.sticky,
            SchemeWin::StickyFocus => &mut self.sticky_focus,
            SchemeWin::Overlay => &mut self.overlay,
            SchemeWin::OverlayFocus => &mut self.overlay_focus,
            SchemeWin::Urgent => &mut self.urgent,
        }
    }
}

/// Close button scheme groupings (non-hover or hover).
#[derive(Debug, Clone, PartialEq, Deserialize, Serialize, Default)]
#[serde(default)]
pub struct CloseButtonColorSet {
    pub normal: ColorSchemeRgba,
    pub locked: ColorSchemeRgba,
    pub fullscreen: ColorSchemeRgba,
}

impl CloseButtonColorSet {
    pub fn scheme(&self, scheme: SchemeClose) -> &ColorSchemeRgba {
        match scheme {
            SchemeClose::Normal => &self.normal,
            SchemeClose::Locked => &self.locked,
            SchemeClose::Fullscreen => &self.fullscreen,
        }
    }

    pub fn scheme_mut(&mut self, scheme: SchemeClose) -> &mut ColorSchemeRgba {
        match scheme {
            SchemeClose::Normal => &mut self.normal,
            SchemeClose::Locked => &mut self.locked,
            SchemeClose::Fullscreen => &mut self.fullscreen,
        }
    }
}

impl ColorSchemeRgba {
    /// Create a new color scheme from RGBA values.
    pub fn new(fg: Rgba, bg: Rgba, detail: Rgba) -> Self {
        Self { fg, bg, detail }
    }

    /// Construct an empty (all black) scheme.
    pub fn empty() -> Self {
        Self::new([0.0; 4], [0.0; 4], [0.0; 4])
    }

    pub fn is_empty(&self) -> bool {
        self.fg == [0.0; 4] && self.bg == [0.0; 4] && self.detail == [0.0; 4]
    }
}

/// Tag color configurations using strings.
#[derive(Debug, Clone, PartialEq, Default, Deserialize, Serialize)]
#[serde(default)]
pub struct TagColorConfigs {
    /// Non-hover color configs.
    #[serde(rename = "normal")]
    pub no_hover: TagColorSet,
    /// Hover color configs.
    pub hover: TagColorSet,
}

impl TagColorConfigs {
    pub fn schemes(&self, hover: SchemeHover) -> &TagColorSet {
        match hover {
            SchemeHover::NoHover => &self.no_hover,
            SchemeHover::Hover => &self.hover,
        }
    }

    pub fn schemes_mut(&mut self, hover: SchemeHover) -> &mut TagColorSet {
        match hover {
            SchemeHover::NoHover => &mut self.no_hover,
            SchemeHover::Hover => &mut self.hover,
        }
    }

    pub fn scheme(&self, hover: SchemeHover, scheme: SchemeTag) -> &ColorSchemeRgba {
        self.schemes(hover).scheme(scheme)
    }

    pub fn scheme_mut(&mut self, hover: SchemeHover, scheme: SchemeTag) -> &mut ColorSchemeRgba {
        self.schemes_mut(hover).scheme_mut(scheme)
    }
}

/// Window color configurations using strings.
#[derive(Debug, Clone, PartialEq, Default, Deserialize, Serialize)]
#[serde(default)]
pub struct WindowColorConfigs {
    /// Non-hover color configs.
    #[serde(rename = "normal")]
    pub no_hover: WindowColorSet,
    /// Hover color configs.
    pub hover: WindowColorSet,
}

impl WindowColorConfigs {
    pub fn schemes(&self, hover: SchemeHover) -> &WindowColorSet {
        match hover {
            SchemeHover::NoHover => &self.no_hover,
            SchemeHover::Hover => &self.hover,
        }
    }

    pub fn schemes_mut(&mut self, hover: SchemeHover) -> &mut WindowColorSet {
        match hover {
            SchemeHover::NoHover => &mut self.no_hover,
            SchemeHover::Hover => &mut self.hover,
        }
    }

    pub fn scheme(&self, hover: SchemeHover, scheme: SchemeWin) -> &ColorSchemeRgba {
        self.schemes(hover).scheme(scheme)
    }

    pub fn scheme_mut(&mut self, hover: SchemeHover, scheme: SchemeWin) -> &mut ColorSchemeRgba {
        self.schemes_mut(hover).scheme_mut(scheme)
    }
}

/// Close button color configurations using strings.
#[derive(Debug, Clone, PartialEq, Default, Deserialize, Serialize)]
#[serde(default)]
pub struct CloseButtonColorConfigs {
    /// Non-hover color configs.
    #[serde(rename = "normal")]
    pub no_hover: CloseButtonColorSet,
    /// Hover color configs.
    pub hover: CloseButtonColorSet,
}

impl CloseButtonColorConfigs {
    pub fn schemes(&self, hover: SchemeHover) -> &CloseButtonColorSet {
        match hover {
            SchemeHover::NoHover => &self.no_hover,
            SchemeHover::Hover => &self.hover,
        }
    }

    pub fn schemes_mut(&mut self, hover: SchemeHover) -> &mut CloseButtonColorSet {
        match hover {
            SchemeHover::NoHover => &mut self.no_hover,
            SchemeHover::Hover => &mut self.hover,
        }
    }

    pub fn scheme(&self, hover: SchemeHover, scheme: SchemeClose) -> &ColorSchemeRgba {
        self.schemes(hover).scheme(scheme)
    }

    pub fn scheme_mut(&mut self, hover: SchemeHover, scheme: SchemeClose) -> &mut ColorSchemeRgba {
        self.schemes_mut(hover).scheme_mut(scheme)
    }
}

/// Border color configuration with pre-parsed RGBA values.
#[derive(Debug, Clone, Copy, PartialEq, Default, Deserialize, Serialize)]
#[serde(default)]
pub struct BorderColorConfig {
    /// Normal border color.
    #[serde(
        deserialize_with = "deserialize_hex_color",
        serialize_with = "serialize_hex_color"
    )]
    pub normal: Rgba,
    /// Focused tiled window color.
    #[serde(
        deserialize_with = "deserialize_hex_color",
        serialize_with = "serialize_hex_color"
    )]
    pub tile_focus: Rgba,
    /// Focused floating window color.
    #[serde(
        deserialize_with = "deserialize_hex_color",
        serialize_with = "serialize_hex_color"
    )]
    pub float_focus: Rgba,
    /// Snap indicator color.
    #[serde(
        deserialize_with = "deserialize_hex_color",
        serialize_with = "serialize_hex_color"
    )]
    pub snap: Rgba,
}

impl BorderColorConfig {
    pub fn get(&self, scheme: SchemeBorder) -> Rgba {
        match scheme {
            SchemeBorder::Normal => self.normal,
            SchemeBorder::TileFocus => self.tile_focus,
            SchemeBorder::FloatFocus => self.float_focus,
            SchemeBorder::Snap => self.snap,
        }
    }

    pub fn set(&mut self, scheme: SchemeBorder, value: Rgba) {
        match scheme {
            SchemeBorder::Normal => self.normal = value,
            SchemeBorder::TileFocus => self.tile_focus = value,
            SchemeBorder::FloatFocus => self.float_focus = value,
            SchemeBorder::Snap => self.snap = value,
        }
    }
}

/// Status bar color configuration with pre-parsed RGBA values.
#[derive(Debug, Clone, Copy, PartialEq, Default, Deserialize, Serialize)]
#[serde(default)]
pub struct StatusColorConfig {
    /// Status bar foreground.
    #[serde(
        deserialize_with = "deserialize_hex_color",
        serialize_with = "serialize_hex_color"
    )]
    pub fg: Rgba,
    /// Status bar background.
    #[serde(
        deserialize_with = "deserialize_hex_color",
        serialize_with = "serialize_hex_color"
    )]
    pub bg: Rgba,
    /// Status bar detail/accent.
    #[serde(
        deserialize_with = "deserialize_hex_color",
        serialize_with = "serialize_hex_color"
    )]
    pub detail: Rgba,
}

impl StatusColorConfig {
    pub fn as_scheme(&self) -> ColorSchemeRgba {
        ColorSchemeRgba::new(self.fg, self.bg, self.detail)
    }
}
