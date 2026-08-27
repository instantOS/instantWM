use std::cell::RefCell;
use std::collections::{HashMap, VecDeque};

use cosmic_text::{
    Attrs, Buffer, Color as CosmicColor, Family, FeatureTag, FontFeatures, FontSystem, Metrics,
    Shaping, SwashCache, Wrap,
};

use crate::bar::text::{self as bar_text, FontRole};
use crate::core_state::FontConfig;
use crate::types::{Point, Rect, Size};

use super::pixels;

const TEXT_CACHE_LIMIT: usize = 2048;

struct CachedFontConfig {
    configured: FontConfig,
    resolved: FontConfig,
    id: u64,
}

#[derive(Clone, Copy, Debug, Hash, PartialEq, Eq)]
struct RenderSize {
    width: i32,
    height: i32,
}

#[derive(Default)]
struct MeasureCache {
    entries: HashMap<u64, HashMap<String, i32>>,
    insertion_order: VecDeque<(u64, String)>,
    len: usize,
}

impl MeasureCache {
    fn insert(&mut self, font_config_id: u64, text: &str, width: i32) {
        if self.len >= TEXT_CACHE_LIMIT
            && let Some((old_config, old_text)) = self.insertion_order.pop_front()
        {
            let remove_config = self.entries.get_mut(&old_config).is_some_and(|entries| {
                entries.remove(old_text.as_str());
                entries.is_empty()
            });
            if remove_config {
                self.entries.remove(&old_config);
            }
            self.len -= 1;
        }

        let owned = text.to_owned();
        let replaced = self
            .entries
            .entry(font_config_id)
            .or_default()
            .insert(owned.clone(), width)
            .is_some();
        if !replaced {
            self.insertion_order.push_back((font_config_id, owned));
            self.len += 1;
        }
    }
}

struct CachedRenderedText {
    buffer: Buffer,
}

#[derive(Default)]
struct RenderCache {
    entries: HashMap<u64, HashMap<String, HashMap<RenderSize, CachedRenderedText>>>,
    insertion_order: VecDeque<(u64, String, RenderSize)>,
    len: usize,
}

impl RenderCache {
    fn insert(
        &mut self,
        font_config_id: u64,
        text: &str,
        size: RenderSize,
        rendered: CachedRenderedText,
    ) {
        if self.len >= TEXT_CACHE_LIMIT
            && let Some((old_config, old_text, old_size)) = self.insertion_order.pop_front()
        {
            let mut remove_text = false;
            let mut remove_config = false;
            if let Some(texts) = self.entries.get_mut(&old_config) {
                if let Some(sizes) = texts.get_mut(old_text.as_str()) {
                    sizes.remove(&old_size);
                    remove_text = sizes.is_empty();
                }
                if remove_text {
                    texts.remove(old_text.as_str());
                }
                remove_config = texts.is_empty();
            }
            if remove_config {
                self.entries.remove(&old_config);
            }
            self.len -= 1;
        }

        let owned = text.to_owned();
        let replaced = self
            .entries
            .entry(font_config_id)
            .or_default()
            .entry(owned.clone())
            .or_default()
            .insert(size, rendered)
            .is_some();
        if !replaced {
            self.insertion_order
                .push_back((font_config_id, owned, size));
            self.len += 1;
        }
    }
}

pub(super) struct TextRasterizer {
    font_system: RefCell<FontSystem>,
    swash_cache: RefCell<SwashCache>,
    measure_cache: RefCell<MeasureCache>,
    render_cache: RefCell<RenderCache>,
    font_configs: Vec<CachedFontConfig>,
    active_font_config: usize,
    next_font_config_id: u64,
}

impl Default for TextRasterizer {
    fn default() -> Self {
        let font_system = FontSystem::new();
        let configured = FontConfig::default();
        let mut resolved = configured.clone();
        resolved.text_family = resolve_family(&font_system, &configured.text_family);
        resolved.icon_family = resolve_family(&font_system, &configured.icon_family);
        Self {
            font_system: RefCell::new(font_system),
            swash_cache: RefCell::new(SwashCache::new()),
            measure_cache: RefCell::new(MeasureCache::default()),
            render_cache: RefCell::new(RenderCache::default()),
            font_configs: vec![CachedFontConfig {
                configured,
                resolved,
                id: 0,
            }],
            active_font_config: 0,
            next_font_config_id: 1,
        }
    }
}

impl TextRasterizer {
    pub(super) fn set_fonts(&mut self, configured: &FontConfig) {
        if self.font_configs[self.active_font_config].configured == *configured {
            return;
        }

        if let Some(index) = self
            .font_configs
            .iter()
            .position(|cached| cached.configured == *configured)
        {
            self.active_font_config = index;
            return;
        }

        let mut resolved = configured.clone();
        {
            let fs = self.font_system.borrow();
            resolved.text_family = resolve_family(&fs, &configured.text_family);
            resolved.icon_family = resolve_family(&fs, &configured.icon_family);
        }
        let id = self.next_font_config_id;
        self.next_font_config_id = self
            .next_font_config_id
            .checked_add(1)
            .expect("font configuration ID space exhausted");
        self.font_configs.push(CachedFontConfig {
            configured: configured.clone(),
            resolved,
            id,
        });
        self.active_font_config = self.font_configs.len() - 1;
    }

    pub(super) fn width(&self, text: &str, box_height: i32) -> i32 {
        if text.is_empty() {
            return 0;
        }
        let font_config_id = self.active_fonts().id;

        if let Some(width) = self
            .measure_cache
            .borrow()
            .entries
            .get(&font_config_id)
            .and_then(|entries| entries.get(text))
        {
            return *width;
        }

        let width = {
            let mut fs = self.font_system.borrow_mut();
            let font_size = self.active_fonts().resolved.text_size;
            let metrics = Metrics::new(font_size, font_size);
            let mut buffer = Buffer::new(&mut fs, metrics);
            buffer.set_size(None, None);
            buffer.set_wrap(Wrap::None);
            self.set_buffer_text(&mut buffer, text, box_height);
            buffer.shape_until_scroll(&mut fs, false);
            buffer
                .layout_runs()
                .map(|run| run.line_w)
                .fold(0.0_f32, f32::max)
                .ceil() as i32
        };

        self.measure_cache
            .borrow_mut()
            .insert(font_config_id, text, width);
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

        let fonts = self.active_fonts();
        let font_size = fonts.resolved.text_size;
        let font_config_id = fonts.id;
        let [r, g, b, a] = color.to_rgba8();
        let cosmic_color = CosmicColor::rgba(r, g, b, a);
        let size = RenderSize {
            width: bounds.w,
            height: bounds.h,
        };

        let is_cached = self
            .render_cache
            .borrow()
            .entries
            .get(&font_config_id)
            .and_then(|entries| entries.get(text))
            .is_some_and(|entries| entries.contains_key(&size));
        if !is_cached {
            let mut fs = self.font_system.borrow_mut();
            let metrics = Metrics::new(font_size, bounds.h as f32);
            let mut buffer = Buffer::new(&mut fs, metrics);
            buffer.set_size(Some(bounds.w as f32), Some(bounds.h as f32));
            buffer.set_wrap(Wrap::None);
            self.set_buffer_text(&mut buffer, text, bounds.h);
            buffer.shape_until_scroll(&mut fs, false);

            self.render_cache.borrow_mut().insert(
                font_config_id,
                text,
                size,
                CachedRenderedText { buffer },
            );
        }

        let mut fs = self.font_system.borrow_mut();
        let mut sc = self.swash_cache.borrow_mut();
        let mut cache = self.render_cache.borrow_mut();
        let Some(cached) = cache
            .entries
            .get_mut(&font_config_id)
            .and_then(|entries| entries.get_mut(text))
            .and_then(|entries| entries.get_mut(&size))
        else {
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

    fn active_fonts(&self) -> &CachedFontConfig {
        &self.font_configs[self.active_font_config]
    }

    fn set_buffer_text(&self, buffer: &mut Buffer, text: &str, box_height: i32) {
        let fonts = &self.active_fonts().resolved;
        let default_attrs = Attrs::new().family(Family::Name(&fonts.text_family));
        let spans = bar_text::gapped_runs(text, fonts.text_size, fonts.icon_size)
            .into_iter()
            .map(|segment| {
                let (family, size) = match segment.role {
                    FontRole::Icon => (&fonts.icon_family, fonts.icon_size),
                    FontRole::Text => (&fonts.text_family, fonts.text_size),
                };
                // The trailing boundary gap rides a span holding only the
                // run's last grapheme, so tracking cannot touch the rest.
                let mut attrs = attrs_for_run(family, size, box_height);
                if let Some(gap) = segment.gap_em {
                    attrs = attrs.letter_spacing(gap);
                }
                if segment.prevent_ligatures {
                    let mut features = FontFeatures::new();
                    features
                        .disable(FeatureTag::STANDARD_LIGATURES)
                        .disable(FeatureTag::CONTEXTUAL_LIGATURES)
                        .disable(FeatureTag::DISCRETIONARY_LIGATURES);
                    attrs = attrs.font_features(features);
                }
                (segment.text, attrs)
            });
        buffer.set_rich_text(spans, &default_attrs, Shaping::Advanced, None);
    }
}

fn attrs_for_run(family: &str, size: f32, box_height: i32) -> Attrs<'_> {
    let line_height = if box_height > 0 {
        box_height as f32
    } else {
        size
    };
    Attrs::new()
        .family(Family::Name(family))
        .metrics(Metrics::new(size, line_height))
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
    use super::{FontConfig, MeasureCache, TEXT_CACHE_LIMIT, TextRasterizer, attrs_for_run};
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
        rasterizer.font_configs[rasterizer.active_font_config]
            .resolved
            .text_family = "resolution-sentinel".to_string();
        rasterizer.set_fonts(&configured);

        assert_eq!(
            rasterizer.active_fonts().resolved.text_family,
            "resolution-sentinel"
        );
    }

    #[test]
    fn changed_configured_families_are_resolved() {
        let mut rasterizer = TextRasterizer::default();
        rasterizer.set_fonts(&FontConfig {
            text_family: "first-family".into(),
            ..FontConfig::default()
        });
        rasterizer.font_configs[rasterizer.active_font_config]
            .resolved
            .text_family = "resolution-sentinel".to_string();

        let configured = FontConfig {
            text_family: "second-family".into(),
            ..FontConfig::default()
        };
        rasterizer.set_fonts(&configured);

        assert_eq!(
            rasterizer.active_fonts().configured.text_family,
            "second-family"
        );
        assert_ne!(
            rasterizer.active_fonts().resolved.text_family,
            "resolution-sentinel"
        );
    }

    #[test]
    fn icon_runs_carry_their_independent_size() {
        let attrs = attrs_for_run("Symbols Nerd Font", 18.0, 32);
        let metrics: Metrics = attrs.metrics_opt.expect("run metrics").into();
        assert_eq!(metrics.font_size, 18.0);
        assert_eq!(metrics.line_height, 32.0);
    }

    #[test]
    fn switching_monitor_font_scales_preserves_cached_layouts() {
        let mut rasterizer = TextRasterizer::default();
        let base = FontConfig::default();
        rasterizer.set_fonts(&base);
        rasterizer.width("cache me", 30);
        let base_entries = rasterizer.measure_cache.get_mut().len;
        assert!(base_entries > 0);

        rasterizer.set_fonts(&base.scaled(2.0));
        rasterizer.width("cache me", 60);
        assert!(rasterizer.measure_cache.get_mut().len > base_entries);

        rasterizer.set_fonts(&base);
        assert_eq!(rasterizer.measure_cache.get_mut().len, base_entries + 1);
        assert_eq!(rasterizer.font_configs.len(), 2);
    }

    #[test]
    fn measurement_cache_evicts_one_entry_instead_of_clearing_everything() {
        let mut cache = MeasureCache::default();
        for index in 0..=TEXT_CACHE_LIMIT {
            cache.insert(0, &format!("entry-{index}"), index as i32);
        }

        assert_eq!(cache.len, TEXT_CACHE_LIMIT);
        assert!(!cache.entries[&0].contains_key("entry-0"));
        assert!(cache.entries[&0].contains_key(format!("entry-{TEXT_CACHE_LIMIT}").as_str()));
    }
}
