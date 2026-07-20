use std::cell::RefCell;
use std::collections::HashMap;

use cosmic_text::{
    Attrs, Buffer, Color as CosmicColor, Family, FontSystem, Metrics, Shaping, SwashCache, Wrap,
};

use crate::types::{Point, Rect, Size};

use super::pixels;

const TEXT_CACHE_LIMIT: usize = 2048;
// Many patched-font icons paint slightly beyond their nominal advance. Keep
// enough tracking on those glyphs that the following normal-font run cannot
// start inside the icon's ink bounds.
const ICON_LETTER_SPACING_EM: f32 = 0.12;
pub(super) const DEFAULT_FONT_SIZE: f32 = 14.0;

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
    font_size: f32,
    configured_font_families: Vec<String>,
    font_families: Vec<String>,
}

impl Default for TextRasterizer {
    fn default() -> Self {
        Self {
            font_system: RefCell::new(FontSystem::new()),
            swash_cache: RefCell::new(SwashCache::new()),
            measure_cache: RefCell::new(HashMap::new()),
            render_cache: RefCell::new(HashMap::new()),
            font_size: DEFAULT_FONT_SIZE,
            configured_font_families: Vec::new(),
            font_families: Vec::new(),
        }
    }
}

impl TextRasterizer {
    pub(super) fn set_font_families(&mut self, configured: &[String]) {
        if self.configured_font_families == configured {
            return;
        }

        let resolved = {
            let fs = self.font_system.borrow();
            configured
                .iter()
                .map(|configured_family| {
                    let wanted = normalized_family(configured_family);
                    fs.db()
                        .faces()
                        .flat_map(|face| face.families.iter().map(|(name, _)| name))
                        .find(|name| normalized_family(name) == wanted)
                        .cloned()
                        .unwrap_or_else(|| configured_family.clone())
                })
                .collect::<Vec<_>>()
        };
        self.configured_font_families = configured.to_vec();
        if self.font_families != resolved {
            self.font_families = resolved;
            self.measure_cache.get_mut().clear();
            self.render_cache.get_mut().clear();
        }
    }

    pub(super) fn set_font_size(&mut self, font_size: f32) {
        if font_size.is_finite()
            && font_size > 0.0
            && self.font_size.to_bits() != font_size.to_bits()
        {
            self.font_size = font_size;
        }
    }

    pub(super) fn width(&self, text: &str, box_height: i32) -> i32 {
        if text.is_empty() {
            return 0;
        }
        let font_size = self.effective_font_size(text, box_height);
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
            self.set_buffer_text(&mut buffer, text);
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
        color: crate::bar::color::Rgba,
    ) {
        if text.is_empty() || !bounds.size().is_positive() {
            return;
        }

        let font_size = self.effective_font_size(text, bounds.h);
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
                self.set_buffer_text(&mut buffer, text);
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

    pub(super) fn is_powerline_text(text: &str) -> bool {
        let mut saw_glyph = false;
        for ch in text.chars() {
            if ch.is_whitespace() {
                continue;
            }
            if !('\u{e0b0}'..='\u{e0d4}').contains(&ch) {
                return false;
            }
            saw_glyph = true;
        }
        saw_glyph
    }

    fn effective_font_size(&self, text: &str, box_height: i32) -> f32 {
        if box_height > 0 && Self::is_powerline_text(text) {
            let max_size = (box_height - 3).max(1) as f32;
            (self.font_size + 2.0).min(max_size)
        } else {
            self.font_size
        }
    }

    fn set_buffer_text(&self, buffer: &mut Buffer, text: &str) {
        let Some(primary) = self.font_families.first() else {
            buffer.set_text(text, &Attrs::new(), Shaping::Advanced, None);
            return;
        };
        let default_attrs = Attrs::new().family(Family::Name(primary));
        let icon_family = self.font_families.get(1).unwrap_or(primary);
        let mut spans = Vec::new();
        let mut start = 0;
        let mut private = text.chars().next().is_some_and(is_private_use);
        for (index, ch) in text.char_indices().skip(1) {
            let next_private = is_private_use(ch);
            if next_private != private {
                let family = if private { icon_family } else { primary };
                let attrs = attrs_for_run(family, private);
                spans.push((&text[start..index], attrs));
                start = index;
                private = next_private;
            }
        }
        if start < text.len() {
            let family = if private { icon_family } else { primary };
            spans.push((&text[start..], attrs_for_run(family, private)));
        }
        buffer.set_rich_text(spans, &default_attrs, Shaping::Advanced, None);
    }
}

fn attrs_for_run(family: &str, private: bool) -> Attrs<'_> {
    let attrs = Attrs::new().family(Family::Name(family));
    if private {
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

fn is_private_use(ch: char) -> bool {
    matches!(ch as u32, 0xE000..=0xF8FF | 0xF0000..=0xFFFFD | 0x100000..=0x10FFFD)
}

#[cfg(test)]
mod tests {
    use super::TextRasterizer;

    #[test]
    fn unchanged_configured_families_skip_resolution() {
        let configured = vec!["sans serif".to_string()];
        let mut rasterizer = TextRasterizer::default();
        rasterizer.set_font_families(&configured);

        // A repeated input must return before touching the resolved list. This
        // pins the hot-path guard independently of which fonts the host has.
        rasterizer.font_families = vec!["resolution-sentinel".to_string()];
        rasterizer.set_font_families(&configured);

        assert_eq!(rasterizer.font_families, ["resolution-sentinel"]);
    }

    #[test]
    fn changed_configured_families_are_resolved() {
        let mut rasterizer = TextRasterizer::default();
        rasterizer.set_font_families(&["first-family".to_string()]);
        rasterizer.font_families = vec!["resolution-sentinel".to_string()];

        rasterizer.set_font_families(&["second-family".to_string()]);

        assert_eq!(rasterizer.configured_font_families, ["second-family"]);
        assert_ne!(rasterizer.font_families, ["resolution-sentinel"]);
    }
}
