//! Color scheme types.
//!
//! Backend-neutral color vocabulary: parsed RGBA values and typed scheme
//! identifiers shared by config, IPC, and both window-manager backends.
//! Backend-native color representations (e.g. allocated Xft pixels) live in
//! their respective backends and are produced from these types at the edge.

use serde::{Deserialize, Deserializer, Serialize, Serializer};

// =============================================================================
// RGBA color value
// =============================================================================

/// An RGBA color represented by floating-point components.
///
/// Components are expected to be in `[0.0, 1.0]`; byte conversions clamp
/// values outside that range.
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct Rgba([f32; 4]);

impl Rgba {
    pub const ZERO: Self = Self([0.0; 4]);

    pub const fn new(r: f32, g: f32, b: f32, a: f32) -> Self {
        Self([r, g, b, a])
    }

    pub const fn rgb(r: f32, g: f32, b: f32) -> Self {
        Self([r, g, b, 1.0])
    }

    pub const fn r(self) -> f32 {
        self.0[0]
    }

    pub const fn g(self) -> f32 {
        self.0[1]
    }

    pub const fn b(self) -> f32 {
        self.0[2]
    }

    pub const fn a(self) -> f32 {
        self.0[3]
    }

    pub const fn with_alpha(self, alpha: f32) -> Self {
        Self::new(self.r(), self.g(), self.b(), alpha)
    }

    pub const fn into_array(self) -> [f32; 4] {
        self.0
    }

    /// Convert the components to bytes, clamping them to the normalized range
    /// and rounding to the nearest integer.
    pub fn to_rgba8(self) -> [u8; 4] {
        fn component_to_u8(component: f32) -> u8 {
            (component.clamp(0.0, 1.0) * 255.0).round() as u8
        }

        [
            component_to_u8(self.r()),
            component_to_u8(self.g()),
            component_to_u8(self.b()),
            component_to_u8(self.a()),
        ]
    }
}

impl std::fmt::Display for Rgba {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        let [r, g, b, a] = self.to_rgba8();
        if a == 255 {
            write!(f, "#{:02X}{:02X}{:02X}", r, g, b)
        } else {
            write!(f, "#{:02X}{:02X}{:02X}{:02X}", r, g, b, a)
        }
    }
}

/// Pack the color as `0xRRGGBB`. The alpha component is discarded.
impl From<Rgba> for u32 {
    fn from(rgba: Rgba) -> Self {
        let [r, g, b, _] = rgba.to_rgba8();
        let (r, g, b) = (u32::from(r), u32::from(g), u32::from(b));
        (r << 16) | (g << 8) | b
    }
}

// =============================================================================
// RGBA8 color value
// =============================================================================

/// An RGBA color with one byte per channel.
///
/// Unlike [`Rgba`], which models colors as they arrive from config and IPC,
/// this is the color form renderers actually produce: glyph coverage, icon
/// bitmaps, and compositor surfaces are all 8-bit. The components are always
/// ordered `[r, g, b, a]` and are never premultiplied — the byte order a
/// target surface stores them in is that surface's business, not the color's.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Rgba8([u8; 4]);

impl Rgba8 {
    pub const ZERO: Self = Self([0, 0, 0, 0]);
    pub const fn new(r: u8, g: u8, b: u8, a: u8) -> Self {
        Self([r, g, b, a])
    }

    /// The components in `[r, g, b, a]` order.
    pub const fn components(self) -> [u8; 4] {
        self.0
    }

    /// The components in `[b, g, r, a]` order, matching the in-memory layout of
    /// an ARGB8888 surface on a little-endian host.
    pub const fn little_endian_argb(self) -> [u8; 4] {
        let [r, g, b, a] = self.0;
        [b, g, r, a]
    }

    /// Whether the color is fully transparent and therefore a no-op to blend.
    pub const fn is_transparent(self) -> bool {
        self.0[3] == 0
    }
}

impl From<Rgba> for Rgba8 {
    /// Quantize to bytes, clamping components outside the normalized range.
    fn from(rgba: Rgba) -> Self {
        Self(rgba.to_rgba8())
    }
}

impl From<[u8; 4]> for Rgba8 {
    fn from(components: [u8; 4]) -> Self {
        Self(components)
    }
}

impl std::str::FromStr for Rgba {
    type Err = String;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        let s = s.trim();
        let hex = s.strip_prefix('#').unwrap_or(s);
        if hex.len() != 6 && hex.len() != 8 {
            return Err(format!("invalid hex color: {s}"));
        }
        let parse = |range: std::ops::Range<usize>| -> Result<u8, String> {
            u8::from_str_radix(&hex[range], 16).map_err(|_| format!("invalid hex color: {s}"))
        };
        let r = parse(0..2)?;
        let g = parse(2..4)?;
        let b = parse(4..6)?;
        let a = if hex.len() == 8 { parse(6..8)? } else { 255 };
        Ok(Self([
            r as f32 / 255.0,
            g as f32 / 255.0,
            b as f32 / 255.0,
            a as f32 / 255.0,
        ]))
    }
}

impl Serialize for Rgba {
    fn serialize<S: Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
        serializer.serialize_str(&self.to_string())
    }
}

impl<'de> Deserialize<'de> for Rgba {
    fn deserialize<D: Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        let s = String::deserialize(deserializer)?;
        s.parse().map_err(serde::de::Error::custom)
    }
}

// =============================================================================
// Scheme enums - typed identifiers for color sets
// =============================================================================

/// Whether the cursor is hovering over the element.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SchemeHover {
    NoHover,
    Hover,
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

/// Persistent window role classification for bar styling.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum WindowRole {
    Normal,
    Sticky,
    EdgeScratchpad,
    Minimized,
}

/// Window focus state for bar styling.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum WindowFocus {
    Normal,
    Focused,
}

// =============================================================================
// Configuration RGBA Types (for config loading)
// =============================================================================

/// Color scheme with pre-parsed RGBA values.
///
/// Colors are parsed once at config load time via serde, not at runtime.
#[derive(Debug, Clone, Copy, PartialEq, Deserialize, Serialize)]
#[serde(default)]
pub struct ColorScheme {
    /// Foreground color.
    pub foreground: Rgba,
    /// Background color.
    pub background: Rgba,
    /// Detail color.
    pub detail: Rgba,
}

impl Default for ColorScheme {
    fn default() -> Self {
        Self::empty()
    }
}

/// Tag scheme groupings (non-hover or hover).
#[derive(Debug, Clone, PartialEq, Deserialize, Serialize, Default)]
#[serde(default)]
pub struct TagColorSet {
    pub inactive: ColorScheme,
    pub filled: ColorScheme,
    pub focus: ColorScheme,
    pub nofocus: ColorScheme,
    pub empty: ColorScheme,
    pub urgent: ColorScheme,
}

impl TagColorSet {
    pub fn colors_for(&self, role: SchemeTag) -> &ColorScheme {
        match role {
            SchemeTag::Inactive => &self.inactive,
            SchemeTag::Filled => &self.filled,
            SchemeTag::Focus => &self.focus,
            SchemeTag::NoFocus => &self.nofocus,
            SchemeTag::Empty => &self.empty,
            SchemeTag::Urgent => &self.urgent,
        }
    }
}

/// Window scheme groupings (non-hover or hover).
#[derive(Debug, Clone, PartialEq, Deserialize, Serialize, Default)]
#[serde(default)]
pub struct WindowColorSet {
    pub focus: ColorScheme,
    pub normal: ColorScheme,
    pub minimized: ColorScheme,
    pub sticky: ColorScheme,
    pub sticky_focus: ColorScheme,
    pub edge_scratchpad: ColorScheme,
    pub edge_scratchpad_focus: ColorScheme,
    pub urgent: ColorScheme,
}

impl WindowColorSet {
    /// Resolve color scheme by orthogonal role and focus state.
    pub fn role_colors(&self, role: WindowRole, focus: WindowFocus) -> &ColorScheme {
        match (role, focus) {
            (WindowRole::Normal, WindowFocus::Normal) => &self.normal,
            (WindowRole::Normal, WindowFocus::Focused) => &self.focus,
            (WindowRole::Sticky, WindowFocus::Normal) => &self.sticky,
            (WindowRole::Sticky, WindowFocus::Focused) => &self.sticky_focus,
            (WindowRole::EdgeScratchpad, WindowFocus::Normal) => &self.edge_scratchpad,
            (WindowRole::EdgeScratchpad, WindowFocus::Focused) => &self.edge_scratchpad_focus,
            (WindowRole::Minimized, WindowFocus::Normal) => &self.minimized,
            (WindowRole::Minimized, WindowFocus::Focused) => &self.focus,
        }
    }
}

/// Close button scheme groupings (non-hover or hover).
#[derive(Debug, Clone, PartialEq, Deserialize, Serialize, Default)]
#[serde(default)]
pub struct CloseButtonColorSet {
    pub normal: ColorScheme,
    pub locked: ColorScheme,
    pub fullscreen: ColorScheme,
}

impl ColorScheme {
    /// Create a new color scheme from RGBA values.
    pub fn new(foreground: Rgba, background: Rgba, detail: Rgba) -> Self {
        Self {
            foreground,
            background,
            detail,
        }
    }

    /// Construct an empty (all black) scheme.
    pub fn empty() -> Self {
        Self::new(Rgba::ZERO, Rgba::ZERO, Rgba::ZERO)
    }

    pub fn is_empty(&self) -> bool {
        self.foreground == Rgba::ZERO && self.background == Rgba::ZERO && self.detail == Rgba::ZERO
    }
}

/// Tag color configuration with named normal/hover variants.
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
    pub fn colors_for(&self, hover: SchemeHover, role: SchemeTag) -> &ColorScheme {
        match hover {
            SchemeHover::NoHover => &self.no_hover,
            SchemeHover::Hover => &self.hover,
        }
        .colors_for(role)
    }
}

/// Window color configuration with named normal/hover variants.
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
    /// Resolve color scheme by orthogonal role and focus state.
    pub fn role_colors(
        &self,
        hover: SchemeHover,
        role: WindowRole,
        focus: WindowFocus,
    ) -> &ColorScheme {
        match hover {
            SchemeHover::NoHover => &self.no_hover,
            SchemeHover::Hover => &self.hover,
        }
        .role_colors(role, focus)
    }

    /// Resolve alert color scheme for urgent windows.
    pub fn urgent_colors(&self, hover: SchemeHover) -> &ColorScheme {
        match hover {
            SchemeHover::NoHover => &self.no_hover.urgent,
            SchemeHover::Hover => &self.hover.urgent,
        }
    }
}

/// Close button color configuration with named normal/hover variants.
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
    /// Compose close button styling across orthogonal locked and fullscreen dimensions.
    ///
    /// Returns the base scheme and an optional detail override scheme (e.g. for fullscreen accent when locked).
    pub fn composed_colors(
        &self,
        hover: SchemeHover,
        is_locked: bool,
        is_fullscreen: bool,
    ) -> (&ColorScheme, Option<&ColorScheme>) {
        let set = match hover {
            SchemeHover::NoHover => &self.no_hover,
            SchemeHover::Hover => &self.hover,
        };
        let base = if is_locked {
            &set.locked
        } else if is_fullscreen {
            &set.fullscreen
        } else {
            &set.normal
        };
        let detail_override = (is_locked && is_fullscreen).then_some(&set.fullscreen);
        (base, detail_override)
    }

    /// Theme color shared by the close button and destructive window gestures.
    pub fn gesture_color(&self) -> Rgba {
        self.hover.normal.detail
    }
}

/// Border color configuration with pre-parsed RGBA values.
#[derive(Debug, Clone, Copy, PartialEq, Default, Deserialize, Serialize)]
#[serde(default)]
pub struct BorderColorConfig {
    /// Normal border color.
    pub normal: Rgba,
    /// Focused tiled window color.
    pub tile_focus: Rgba,
    /// Focused floating window color.
    pub float_focus: Rgba,
    /// Snap indicator color.
    pub snap: Rgba,
}

/// Status bar color configuration with pre-parsed RGBA values.
#[derive(Debug, Clone, Copy, PartialEq, Default, Deserialize, Serialize)]
#[serde(default)]
pub struct StatusColorConfig {
    /// Status bar foreground.
    pub foreground: Rgba,
    /// Status bar background.
    pub background: Rgba,
    /// Status bar detail/accent.
    pub detail: Rgba,
    /// Separator between i3bar status blocks.
    pub separator: Rgba,
    /// Accent used to identify the clickable i3bar block under the pointer.
    pub hover: Rgba,
}

impl StatusColorConfig {
    pub fn as_scheme(&self) -> ColorScheme {
        ColorScheme::new(self.foreground, self.background, self.detail)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn hex_roundtrip() {
        let rgba = Rgba::new(0.5, 0.25, 0.75, 1.0);
        let hex = rgba.to_string();
        assert_eq!(hex, "#8040BF");
        let parsed = hex.parse::<Rgba>().unwrap();
        // Hex roundtrip isn't exact for floats (0.5*255=127.5→128, 128/255≠0.5)
        // but the hex string roundtrips exactly.
        assert_eq!(parsed.to_string(), hex);
    }

    #[test]
    fn hex_with_alpha() {
        let rgba = Rgba::new(1.0, 0.0, 0.0, 0.5);
        let hex = rgba.to_string();
        assert_eq!(hex, "#FF000080");
        let parsed: Rgba = hex.parse().unwrap();
        assert_eq!(parsed.to_string(), hex);
    }

    #[test]
    fn rgba8_roundtrips_and_orders_argb_by_hand() {
        let bytes: Rgba8 = [0, 128, 255, 64].into();
        assert_eq!(bytes.components(), [0, 128, 255, 64]);
        // The ARGB8888 word is stored little-endian, so B leads.
        assert_eq!(bytes.little_endian_argb(), [255, 128, 0, 64]);
        assert!(!bytes.is_transparent());
        assert!(Rgba8::new(1, 2, 3, 0).is_transparent());
        assert!(Rgba8::ZERO.is_transparent());

        // `Rgba` clamps and rounds on the way down, and `to_rgba8` is the
        // exact inverse of the `From` impl.
        let floats = Rgba::new(0.0, 0.5, 1.0, 0.25);
        assert_eq!(Rgba8::from(floats).components(), floats.to_rgba8());
    }

    #[test]
    fn to_u32() {
        let rgba = Rgba::new(1.0, 0.5, 0.0, 1.0);
        let packed: u32 = rgba.into();
        assert_eq!(packed, 0xFF8000);
    }

    #[test]
    fn rgba8_is_clamped_and_rounded() {
        let rgba = Rgba::new(-0.1, 0.5, 1.1, 0.25);
        assert_eq!(rgba.to_rgba8(), [0, 128, 255, 64]);
    }

    #[test]
    fn debug_preserves_precision_hidden_by_display() {
        let first = Rgba::rgb(0.5, 0.0, 0.0);
        let second = Rgba::rgb(0.5001, 0.0, 0.0);
        assert_eq!(first.to_string(), second.to_string());
        assert_ne!(format!("{first:?}"), format!("{second:?}"));
    }
}
