//! Color scheme types.
//!
//! Types for managing colors in the window manager UI.

use crate::drw::Color;

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
    /// Urgent / special state.
    Empty,
}

impl SchemeTag {
    pub const fn from_index(index: usize) -> Option<Self> {
        match index {
            0 => Some(Self::Inactive),
            1 => Some(Self::Filled),
            2 => Some(Self::Focus),
            3 => Some(Self::NoFocus),
            4 => Some(Self::Empty),
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

/// Which color component to read from a scheme triplet.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ColIndex {
    Fg,
    Bg,
    Detail,
}

impl ColIndex {
    pub const fn from_index(index: usize) -> Option<Self> {
        match index {
            0 => Some(Self::Fg),
            1 => Some(Self::Bg),
            2 => Some(Self::Detail),
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

/// Tag scheme groupings (non-hover or hover).
#[derive(Debug, Clone, PartialEq, Default)]
pub struct TagSchemesSet {
    pub inactive: ColorScheme,
    pub filled: ColorScheme,
    pub focus: ColorScheme,
    pub nofocus: ColorScheme,
    pub empty: ColorScheme,
}

impl TagSchemesSet {
    pub fn scheme(&self, scheme: SchemeTag) -> &ColorScheme {
        match scheme {
            SchemeTag::Inactive => &self.inactive,
            SchemeTag::Filled => &self.filled,
            SchemeTag::Focus => &self.focus,
            SchemeTag::NoFocus => &self.nofocus,
            SchemeTag::Empty => &self.empty,
        }
    }

    pub fn scheme_mut(&mut self, scheme: SchemeTag) -> &mut ColorScheme {
        match scheme {
            SchemeTag::Inactive => &mut self.inactive,
            SchemeTag::Filled => &mut self.filled,
            SchemeTag::Focus => &mut self.focus,
            SchemeTag::NoFocus => &mut self.nofocus,
            SchemeTag::Empty => &mut self.empty,
        }
    }
}

/// Color schemes for tag buttons (hover and non-hover states).
#[derive(Debug, Clone, PartialEq, Default)]
pub struct TagSchemes {
    /// Schemes when not hovering.
    pub no_hover: TagSchemesSet,
    /// Schemes when hovering.
    pub hover: TagSchemesSet,
}

/// Window scheme groupings (non-hover or hover).
#[derive(Debug, Clone, PartialEq, Default)]
pub struct WindowSchemesSet {
    pub focus: ColorScheme,
    pub normal: ColorScheme,
    pub minimized: ColorScheme,
    pub sticky: ColorScheme,
    pub sticky_focus: ColorScheme,
    pub overlay: ColorScheme,
    pub overlay_focus: ColorScheme,
}

impl WindowSchemesSet {
    pub fn scheme(&self, scheme: SchemeWin) -> &ColorScheme {
        match scheme {
            SchemeWin::Focus => &self.focus,
            SchemeWin::Normal => &self.normal,
            SchemeWin::Minimized => &self.minimized,
            SchemeWin::Sticky => &self.sticky,
            SchemeWin::StickyFocus => &self.sticky_focus,
            SchemeWin::Overlay => &self.overlay,
            SchemeWin::OverlayFocus => &self.overlay_focus,
        }
    }

    pub fn scheme_mut(&mut self, scheme: SchemeWin) -> &mut ColorScheme {
        match scheme {
            SchemeWin::Focus => &mut self.focus,
            SchemeWin::Normal => &mut self.normal,
            SchemeWin::Minimized => &mut self.minimized,
            SchemeWin::Sticky => &mut self.sticky,
            SchemeWin::StickyFocus => &mut self.sticky_focus,
            SchemeWin::Overlay => &mut self.overlay,
            SchemeWin::OverlayFocus => &mut self.overlay_focus,
        }
    }
}

/// Color schemes for window title buttons.
#[derive(Debug, Clone, PartialEq, Default)]
pub struct WindowSchemes {
    /// Schemes when not hovering.
    pub no_hover: WindowSchemesSet,
    /// Schemes when hovering.
    pub hover: WindowSchemesSet,
}

/// Close button scheme groupings (non-hover or hover).
#[derive(Debug, Clone, PartialEq, Default)]
pub struct CloseButtonSchemesSet {
    pub normal: ColorScheme,
    pub locked: ColorScheme,
    pub fullscreen: ColorScheme,
}

impl CloseButtonSchemesSet {
    pub fn scheme(&self, scheme: SchemeClose) -> &ColorScheme {
        match scheme {
            SchemeClose::Normal => &self.normal,
            SchemeClose::Locked => &self.locked,
            SchemeClose::Fullscreen => &self.fullscreen,
        }
    }

    pub fn scheme_mut(&mut self, scheme: SchemeClose) -> &mut ColorScheme {
        match scheme {
            SchemeClose::Normal => &mut self.normal,
            SchemeClose::Locked => &mut self.locked,
            SchemeClose::Fullscreen => &mut self.fullscreen,
        }
    }
}

/// Color schemes for close buttons.
#[derive(Debug, Clone, PartialEq, Default)]
pub struct CloseButtonSchemes {
    /// Schemes when not hovering.
    pub no_hover: CloseButtonSchemesSet,
    /// Schemes when hovering.
    pub hover: CloseButtonSchemesSet,
}

// =============================================================================
// Configuration String Types (for config loading)
// =============================================================================

/// Color scheme using string colors (before parsing).
#[derive(Debug, Clone, PartialEq)]
pub struct ColorSchemeStrings {
    /// Foreground color string.
    pub fg: String,
    /// Background color string.
    pub bg: String,
    /// Detail color string.
    pub detail: String,
}

/// Tag scheme groupings (non-hover or hover).
#[derive(Debug, Clone, PartialEq)]
pub struct TagColorSet {
    pub inactive: ColorSchemeStrings,
    pub filled: ColorSchemeStrings,
    pub focus: ColorSchemeStrings,
    pub nofocus: ColorSchemeStrings,
    pub empty: ColorSchemeStrings,
}

impl Default for TagColorSet {
    fn default() -> Self {
        Self {
            inactive: ColorSchemeStrings::empty(),
            filled: ColorSchemeStrings::empty(),
            focus: ColorSchemeStrings::empty(),
            nofocus: ColorSchemeStrings::empty(),
            empty: ColorSchemeStrings::empty(),
        }
    }
}

impl TagColorSet {
    pub fn scheme(&self, scheme: SchemeTag) -> &ColorSchemeStrings {
        match scheme {
            SchemeTag::Inactive => &self.inactive,
            SchemeTag::Filled => &self.filled,
            SchemeTag::Focus => &self.focus,
            SchemeTag::NoFocus => &self.nofocus,
            SchemeTag::Empty => &self.empty,
        }
    }

    pub fn scheme_mut(&mut self, scheme: SchemeTag) -> &mut ColorSchemeStrings {
        match scheme {
            SchemeTag::Inactive => &mut self.inactive,
            SchemeTag::Filled => &mut self.filled,
            SchemeTag::Focus => &mut self.focus,
            SchemeTag::NoFocus => &mut self.nofocus,
            SchemeTag::Empty => &mut self.empty,
        }
    }
}

/// Window scheme groupings (non-hover or hover).
#[derive(Debug, Clone, PartialEq)]
pub struct WindowColorSet {
    pub focus: ColorSchemeStrings,
    pub normal: ColorSchemeStrings,
    pub minimized: ColorSchemeStrings,
    pub sticky: ColorSchemeStrings,
    pub sticky_focus: ColorSchemeStrings,
    pub overlay: ColorSchemeStrings,
    pub overlay_focus: ColorSchemeStrings,
}

impl Default for WindowColorSet {
    fn default() -> Self {
        Self {
            focus: ColorSchemeStrings::empty(),
            normal: ColorSchemeStrings::empty(),
            minimized: ColorSchemeStrings::empty(),
            sticky: ColorSchemeStrings::empty(),
            sticky_focus: ColorSchemeStrings::empty(),
            overlay: ColorSchemeStrings::empty(),
            overlay_focus: ColorSchemeStrings::empty(),
        }
    }
}

impl WindowColorSet {
    pub fn scheme(&self, scheme: SchemeWin) -> &ColorSchemeStrings {
        match scheme {
            SchemeWin::Focus => &self.focus,
            SchemeWin::Normal => &self.normal,
            SchemeWin::Minimized => &self.minimized,
            SchemeWin::Sticky => &self.sticky,
            SchemeWin::StickyFocus => &self.sticky_focus,
            SchemeWin::Overlay => &self.overlay,
            SchemeWin::OverlayFocus => &self.overlay_focus,
        }
    }

    pub fn scheme_mut(&mut self, scheme: SchemeWin) -> &mut ColorSchemeStrings {
        match scheme {
            SchemeWin::Focus => &mut self.focus,
            SchemeWin::Normal => &mut self.normal,
            SchemeWin::Minimized => &mut self.minimized,
            SchemeWin::Sticky => &mut self.sticky,
            SchemeWin::StickyFocus => &mut self.sticky_focus,
            SchemeWin::Overlay => &mut self.overlay,
            SchemeWin::OverlayFocus => &mut self.overlay_focus,
        }
    }
}

/// Close button scheme groupings (non-hover or hover).
#[derive(Debug, Clone, PartialEq)]
pub struct CloseButtonColorSet {
    pub normal: ColorSchemeStrings,
    pub locked: ColorSchemeStrings,
    pub fullscreen: ColorSchemeStrings,
}

impl Default for CloseButtonColorSet {
    fn default() -> Self {
        Self {
            normal: ColorSchemeStrings::empty(),
            locked: ColorSchemeStrings::empty(),
            fullscreen: ColorSchemeStrings::empty(),
        }
    }
}

impl CloseButtonColorSet {
    pub fn scheme(&self, scheme: SchemeClose) -> &ColorSchemeStrings {
        match scheme {
            SchemeClose::Normal => &self.normal,
            SchemeClose::Locked => &self.locked,
            SchemeClose::Fullscreen => &self.fullscreen,
        }
    }

    pub fn scheme_mut(&mut self, scheme: SchemeClose) -> &mut ColorSchemeStrings {
        match scheme {
            SchemeClose::Normal => &mut self.normal,
            SchemeClose::Locked => &mut self.locked,
            SchemeClose::Fullscreen => &mut self.fullscreen,
        }
    }
}

impl ColorSchemeStrings {
    /// Create a new color scheme from strings.
    pub fn new(fg: impl Into<String>, bg: impl Into<String>, detail: impl Into<String>) -> Self {
        Self {
            fg: fg.into(),
            bg: bg.into(),
            detail: detail.into(),
        }
    }

    /// Construct an empty (all blank) scheme.
    pub fn empty() -> Self {
        Self::new("", "", "")
    }

    /// Read a color by component.
    pub fn get(&self, col: ColIndex) -> &str {
        match col {
            ColIndex::Fg => &self.fg,
            ColIndex::Bg => &self.bg,
            ColIndex::Detail => &self.detail,
        }
    }

    /// Mutate a color by component.
    pub fn set(&mut self, col: ColIndex, value: impl Into<String>) {
        let value = value.into();
        match col {
            ColIndex::Fg => self.fg = value,
            ColIndex::Bg => self.bg = value,
            ColIndex::Detail => self.detail = value,
        }
    }

    pub fn is_empty(&self) -> bool {
        self.fg.is_empty() && self.bg.is_empty() && self.detail.is_empty()
    }
}

/// Tag color configurations using strings.
#[derive(Debug, Clone, PartialEq, Default)]
pub struct TagColorConfigs {
    /// Non-hover color configs.
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

    pub fn scheme(&self, hover: SchemeHover, scheme: SchemeTag) -> &ColorSchemeStrings {
        self.schemes(hover).scheme(scheme)
    }

    pub fn scheme_mut(&mut self, hover: SchemeHover, scheme: SchemeTag) -> &mut ColorSchemeStrings {
        self.schemes_mut(hover).scheme_mut(scheme)
    }
}

/// Window color configurations using strings.
#[derive(Debug, Clone, PartialEq, Default)]
pub struct WindowColorConfigs {
    /// Non-hover color configs.
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

    pub fn scheme(&self, hover: SchemeHover, scheme: SchemeWin) -> &ColorSchemeStrings {
        self.schemes(hover).scheme(scheme)
    }

    pub fn scheme_mut(&mut self, hover: SchemeHover, scheme: SchemeWin) -> &mut ColorSchemeStrings {
        self.schemes_mut(hover).scheme_mut(scheme)
    }
}

/// Close button color configurations using strings.
#[derive(Debug, Clone, PartialEq, Default)]
pub struct CloseButtonColorConfigs {
    /// Non-hover color configs.
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

    pub fn scheme(&self, hover: SchemeHover, scheme: SchemeClose) -> &ColorSchemeStrings {
        self.schemes(hover).scheme(scheme)
    }

    pub fn scheme_mut(
        &mut self,
        hover: SchemeHover,
        scheme: SchemeClose,
    ) -> &mut ColorSchemeStrings {
        self.schemes_mut(hover).scheme_mut(scheme)
    }
}

/// Border color configuration using strings.
#[derive(Debug, Clone, PartialEq, Default)]
pub struct BorderColorConfig {
    /// Normal border color.
    pub normal: String,
    /// Focused tiled window color.
    pub tile_focus: String,
    /// Focused floating window color.
    pub float_focus: String,
    /// Snap indicator color.
    pub snap: String,
}

impl BorderColorConfig {
    pub fn get(&self, scheme: SchemeBorder) -> &str {
        match scheme {
            SchemeBorder::Normal => &self.normal,
            SchemeBorder::TileFocus => &self.tile_focus,
            SchemeBorder::FloatFocus => &self.float_focus,
            SchemeBorder::Snap => &self.snap,
        }
    }

    pub fn set(&mut self, scheme: SchemeBorder, value: impl Into<String>) {
        let value = value.into();
        match scheme {
            SchemeBorder::Normal => self.normal = value,
            SchemeBorder::TileFocus => self.tile_focus = value,
            SchemeBorder::FloatFocus => self.float_focus = value,
            SchemeBorder::Snap => self.snap = value,
        }
    }
}

/// Status color configuration using strings.
#[derive(Debug, Clone, PartialEq, Default)]
pub struct StatusColorConfig {
    /// Status bar colors.
    pub fg: String,
    /// Status bar background.
    pub bg: String,
    /// Status bar detail/accent.
    pub detail: String,
}

impl StatusColorConfig {
    pub fn as_scheme(&self) -> ColorSchemeStrings {
        ColorSchemeStrings::new(self.fg.clone(), self.bg.clone(), self.detail.clone())
    }

    pub fn get(&self, col: ColIndex) -> &str {
        match col {
            ColIndex::Fg => &self.fg,
            ColIndex::Bg => &self.bg,
            ColIndex::Detail => &self.detail,
        }
    }

    pub fn set(&mut self, col: ColIndex, value: impl Into<String>) {
        let value = value.into();
        match col {
            ColIndex::Fg => self.fg = value,
            ColIndex::Bg => self.bg = value,
            ColIndex::Detail => self.detail = value,
        }
    }
}
