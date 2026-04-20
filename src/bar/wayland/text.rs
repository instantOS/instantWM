use std::cell::RefCell;
use std::collections::HashMap;

use cosmic_text::{
    Attrs, Buffer, Color as CosmicColor, FontSystem, Metrics, Shaping, SwashCache, Wrap,
};

use super::pixels;

const TEXT_CACHE_LIMIT: usize = 2048;
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
}

impl Default for TextRasterizer {
    fn default() -> Self {
        Self {
            font_system: RefCell::new(FontSystem::new()),
            swash_cache: RefCell::new(SwashCache::new()),
            measure_cache: RefCell::new(HashMap::new()),
            render_cache: RefCell::new(HashMap::new()),
            font_size: DEFAULT_FONT_SIZE,
        }
    }
}

impl TextRasterizer {
    pub(super) fn set_font_size(&mut self, font_size: f32) {
        if font_size.is_finite() && font_size > 0.0 {
            if self.font_size.to_bits() != font_size.to_bits() {
                self.font_size = font_size;
            }
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
            buffer.set_size(&mut fs, None, None);
            buffer.set_wrap(&mut fs, Wrap::None);
            buffer.set_text(&mut fs, text, Attrs::new(), Shaping::Advanced);
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
        canvas_w: i32,
        canvas_h: i32,
        x: i32,
        y: i32,
        w: i32,
        h: i32,
        text: &str,
        color: [f32; 4],
    ) {
        if text.is_empty() || w <= 0 || h <= 0 {
            return;
        }

        let font_size = self.effective_font_size(text, h);
        let cosmic_color = CosmicColor::rgba(
            (color[0] * 255.0) as u8,
            (color[1] * 255.0) as u8,
            (color[2] * 255.0) as u8,
            (color[3] * 255.0) as u8,
        );
        let key = TextRenderKey {
            text: text.to_string(),
            width: w,
            height: h,
            font_size_bits: font_size.to_bits(),
        };

        {
            let mut cache = self.render_cache.borrow_mut();
            if !cache.contains_key(&key) {
                let mut fs = self.font_system.borrow_mut();
                let metrics = Metrics::new(font_size, h as f32);
                let mut buffer = Buffer::new(&mut fs, metrics);
                buffer.set_size(&mut fs, Some(w as f32), Some(h as f32));
                buffer.set_wrap(&mut fs, Wrap::None);
                buffer.set_text(&mut fs, text, Attrs::new(), Shaping::Advanced);
                buffer.shape_until_scroll(&mut fs, false);
                if cache.len() > TEXT_CACHE_LIMIT {
                    cache.clear();
                }
                cache.insert(key.clone(), CachedRenderedText { buffer });
            }
        }

        let mut fs = self.font_system.borrow_mut();
        let mut sc = self.swash_cache.borrow_mut();
        let cache = self.render_cache.borrow();
        let Some(cached) = cache.get(&key) else {
            return;
        };

        cached
            .buffer
            .draw(&mut fs, &mut sc, cosmic_color, |gx, gy, _, _, color| {
                if gx < 0 || gy < 0 || gx >= w || gy >= h {
                    return;
                }
                pixels::fill_pixel(
                    pixels,
                    canvas_w,
                    canvas_h,
                    x + gx,
                    y + gy,
                    color.r(),
                    color.g(),
                    color.b(),
                    color.a(),
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
}
