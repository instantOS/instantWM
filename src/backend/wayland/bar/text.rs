use std::cell::RefCell;
use std::collections::HashMap;

use cosmic_text::{
    Attrs, Buffer, Color as CosmicColor, Family, FontSystem, Metrics, Shaping, SwashCache, Wrap,
};

use crate::bar::text::{self as bar_text, FontRole};
use crate::core_state::FontConfig;
use crate::types::{Point, Rect, Size};

use super::pixels;

const TEXT_CACHE_LIMIT: usize = 2048;
// Many patched-font icons paint slightly beyond their nominal advance. Keep
// enough tracking on those glyphs that the following normal-font run cannot
// start inside the icon's ink bounds.
const ICON_LETTER_SPACING_EM: f32 = 0.12;

#[derive(Clone, Debug, Hash, PartialEq, Eq)]
struct TextMeasureKey {
    text: String,
    font_size_bits: u32,
}

#[derive(Clone, Debug, Hash, PartialEq, Eq)]
struct TextRenderKey {
    text: String,
    width: i32,
    height: i32,
    font_size_bits: u32,
}

struct CachedMeasuredText {
    #[allow(dead_code)]
    buffer: Buffer,
    width: i32,
}

struct CachedRenderedText {
    buffer: Buffer,
}

pub(super) struct TextRasterizer {
    font_system: RefCell<FontSystem>,
    swash_cache: RefCell<SwashCache>,
    measure_cache: RefCell<HashMap<TextMeasureKey, CachedMeasuredText>>,
    render_cache: RefCell<HashMap<TextRenderKey, CachedRenderedText>>,
    configured_fonts: Option<FontConfig>,
    fonts: FontConfig,
}

impl Default for TextRasterizer {
    fn default() -> Self {
        Self {
            font_system: RefCell::new(FontSystem::new()),
            swash_cache: RefCell::new(SwashCache::new()),
            measure_cache: RefCell::new(HashMap::new()),
            render_cache: RefCell::new(HashMap::new()),
            configured_fonts: None,
            fonts: FontConfig::default(),
        }
    }
}

impl TextRasterizer {
    pub(super) fn set_fonts(&mut self, configured: &FontConfig) {
        if self.configured_fonts.as_ref() == Some(configured) {
            return;
        }

        let mut resolved = configured.clone();
        {
            let fs = self.font_system.borrow();
            resolved.text_family = resolve_family(&fs, &configured.text_family);
            resolved.icon_family = resolve_family(&fs, &configured.icon_family);
        }
        self.configured_fonts = Some(configured.clone());
        self.fonts = resolved;
        self.measure_cache.get_mut().clear();
        self.render_cache.get_mut().clear();
    }

    pub(super) fn width(&self, text: &str, box_height: i32) -> i32 {
        if text.is_empty() {
            return 0;
        }
        let font_size = self.fonts.text_size;
        let key = TextMeasureKey {
            text: text.to_string(),
            font_size_bits: font_size.to_bits(),
        };

        if let Some(cached) = self.measure_cache.borrow().get(&key) {
            return cached.width;
        }

        let cached = {
            let mut fs = self.font_system.borrow_mut();
            let metrics = Metrics::new(font_size, font_size);
            let mut buffer = Buffer::new(&mut fs, metrics);
            buffer.set_size(None, None);
            buffer.set_wrap(Wrap::None);
            self.set_buffer_text(&mut buffer, text, box_height);
            buffer.shape_until_scroll(&mut fs, false);
            let width = buffer
                .layout_runs()
                .map(|run| run.line_w)
                .fold(0.0_f32, f32::max)
                .ceil() as i32;
            CachedMeasuredText { buffer, width }
        };

        let width = cached.width;
        let mut cache = self.measure_cache.borrow_mut();
        if cache.len() > TEXT_CACHE_LIMIT {
            cache.clear();
        }
        cache.insert(key, cached);
        width
    }

    pub(super) fn rasterize(
        &self,
        pixels: &mut [u8],
        canvas_size: Size,
        bounds: Rect,
        text: &str,
        color: crate::types::color::Rgba,
    ) {
        if text.is_empty() || !bounds.size().is_positive() {
            return;
        }

        let font_size = self.fonts.text_size;
        let [r, g, b, a] = color.to_rgba8();
        let cosmic_color = CosmicColor::rgba(r, g, b, a);
        let key = TextRenderKey {
            text: text.to_string(),
            width: bounds.w,
            height: bounds.h,
            font_size_bits: font_size.to_bits(),
        };

        {
            let mut cache = self.render_cache.borrow_mut();
            if !cache.contains_key(&key) {
                let mut fs = self.font_system.borrow_mut();
                let metrics = Metrics::new(font_size, bounds.h as f32);
                let mut buffer = Buffer::new(&mut fs, metrics);
                buffer.set_size(Some(bounds.w as f32), Some(bounds.h as f32));
                buffer.set_wrap(Wrap::None);
                self.set_buffer_text(&mut buffer, text, bounds.h);
                buffer.shape_until_scroll(&mut fs, false);
                if cache.len() > TEXT_CACHE_LIMIT {
                    cache.clear();
                }
                cache.insert(key.clone(), CachedRenderedText { buffer });
            }
        }

        let mut fs = self.font_system.borrow_mut();
        let mut sc = self.swash_cache.borrow_mut();
        let mut cache = self.render_cache.borrow_mut();
        let Some(cached) = cache.get_mut(&key) else {
            return;
        };

        cached
            .buffer
            .draw(&mut fs, &mut sc, cosmic_color, |gx, gy, _, _, color| {
                if gx < 0 || gy < 0 || gx >= bounds.w || gy >= bounds.h {
                    return;
                }
                pixels::fill_pixel(
                    pixels,
                    canvas_size,
                    Point::new(bounds.x + gx, bounds.y + gy),
                    [color.r(), color.g(), color.b(), color.a()],
                );
            });
    }

    fn set_buffer_text(&self, buffer: &mut Buffer, text: &str, box_height: i32) {
        let default_attrs = Attrs::new().family(Family::Name(&self.fonts.text_family));
        let spans = bar_text::runs(text).into_iter().map(|run| {
            let (family, size) = match run.role {
                FontRole::Icon => (&self.fonts.icon_family, self.fonts.icon_size),
                FontRole::Text => (&self.fonts.text_family, self.fonts.text_size),
            };
            (run.text, attrs_for_run(family, size, box_height, run.role))
        });
        buffer.set_rich_text(spans, &default_attrs, Shaping::Advanced, None);
    }
}

fn attrs_for_run(family: &str, size: f32, box_height: i32, role: FontRole) -> Attrs<'_> {
    let line_height = if box_height > 0 {
        box_height as f32
    } else {
        size
    };
    let attrs = Attrs::new()
        .family(Family::Name(family))
        .metrics(Metrics::new(size, line_height));
    if role == FontRole::Icon {
        attrs.letter_spacing(ICON_LETTER_SPACING_EM)
    } else {
        attrs
    }
}

fn normalized_family(family: &str) -> String {
    family
        .chars()
        .filter(|ch| ch.is_alphanumeric())
        .flat_map(char::to_lowercase)
        .collect()
}

fn resolve_family(font_system: &FontSystem, configured: &str) -> String {
    let wanted = normalized_family(configured);
    font_system
        .db()
        .faces()
        .flat_map(|face| face.families.iter().map(|(name, _)| name))
        .find(|name| normalized_family(name) == wanted)
        .cloned()
        .unwrap_or_else(|| configured.to_string())
}

#[cfg(test)]
mod tests {
    use super::{FontConfig, FontRole, TextRasterizer, attrs_for_run};
    use cosmic_text::Metrics;

    #[test]
    fn unchanged_configured_families_skip_resolution() {
        let configured = FontConfig {
            text_family: "sans serif".into(),
            ..FontConfig::default()
        };
        let mut rasterizer = TextRasterizer::default();
        rasterizer.set_fonts(&configured);

        // A repeated input must return before touching the resolved list. This
        // pins the hot-path guard independently of which fonts the host has.
        rasterizer.fonts.text_family = "resolution-sentinel".to_string();
        rasterizer.set_fonts(&configured);

        assert_eq!(rasterizer.fonts.text_family, "resolution-sentinel");
    }

    #[test]
    fn changed_configured_families_are_resolved() {
        let mut rasterizer = TextRasterizer::default();
        rasterizer.set_fonts(&FontConfig {
            text_family: "first-family".into(),
            ..FontConfig::default()
        });
        rasterizer.fonts.text_family = "resolution-sentinel".to_string();

        let configured = FontConfig {
            text_family: "second-family".into(),
            ..FontConfig::default()
        };
        rasterizer.set_fonts(&configured);

        assert_eq!(
            rasterizer
                .configured_fonts
                .as_ref()
                .expect("configured fonts")
                .text_family,
            "second-family"
        );
        assert_ne!(rasterizer.fonts.text_family, "resolution-sentinel");
    }

    #[test]
    fn icon_runs_carry_their_independent_size() {
        let attrs = attrs_for_run("Symbols Nerd Font", 18.0, 32, FontRole::Icon);
        let metrics: Metrics = attrs.metrics_opt.expect("run metrics").into();
        assert_eq!(metrics.font_size, 18.0);
        assert_eq!(metrics.line_height, 32.0);
    }
}
