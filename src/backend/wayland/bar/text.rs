use std::cell::RefCell;
use std::collections::VecDeque;

use cosmic_text::{
    Attrs, Buffer, Color as CosmicColor, Family, FeatureTag, FontFeatures, FontSystem, Metrics,
    Shaping, SwashCache, Wrap,
};

use crate::bar::text::{self as bar_text, FontRole};
use crate::core_state::FontConfig;
use crate::types::{Point, Rect, Size};

use super::pixels;

// A normal bar has a few dozen stable labels. True LRU promotion keeps those
// hot entries resident while clocks and counters churn through the remaining
// space. The text budgets also prevent one pathological status value from
// being retained indefinitely. They count source bytes rather than attempting
// to guess cosmic-text's private allocation capacity.
const MEASURE_CACHE_ENTRY_LIMIT: usize = 512;
const MEASURE_CACHE_TEXT_LIMIT: usize = 64 * 1024;
const RENDER_CACHE_ENTRY_LIMIT: usize = 256;
const RENDER_CACHE_TEXT_LIMIT: usize = 32 * 1024;
const FONT_CONFIG_LIMIT: usize = 8;

struct CacheEntry<K, V> {
    key: K,
    value: V,
    text_bytes: usize,
}

/// Small LRU that owns each key exactly once.
///
/// At these limits, searching from the hot end is cheap and avoids a second
/// copy of every string in a hash map plus an eviction queue.
struct SmallLru<K, V> {
    entries: VecDeque<CacheEntry<K, V>>,
    entry_limit: usize,
    text_limit: usize,
    text_bytes: usize,
}

impl<K, V> SmallLru<K, V> {
    fn new(entry_limit: usize, text_limit: usize) -> Self {
        Self {
            entries: VecDeque::new(),
            entry_limit,
            text_limit,
            text_bytes: 0,
        }
    }

    fn get_mut_by(&mut self, matches: impl Fn(&K) -> bool) -> Option<&mut V> {
        let index = self.entries.iter().rposition(|entry| matches(&entry.key))?;
        let entry = self.entries.remove(index).expect("LRU index must exist");
        self.entries.push_back(entry);
        Some(&mut self.entries.back_mut().expect("just inserted").value)
    }

    fn insert(&mut self, key: K, value: V, text_bytes: usize) -> bool {
        if self.entry_limit == 0 || text_bytes > self.text_limit {
            return false;
        }
        while self.entries.len() >= self.entry_limit
            || self.text_bytes.saturating_add(text_bytes) > self.text_limit
        {
            let Some(evicted) = self.entries.pop_front() else {
                break;
            };
            self.text_bytes -= evicted.text_bytes;
        }
        self.text_bytes += text_bytes;
        self.entries.push_back(CacheEntry {
            key,
            value,
            text_bytes,
        });
        true
    }

    fn retain(&mut self, mut keep: impl FnMut(&K) -> bool) {
        self.entries.retain(|entry| keep(&entry.key));
        self.text_bytes = self.entries.iter().map(|entry| entry.text_bytes).sum();
    }

    #[cfg(test)]
    fn len(&self) -> usize {
        self.entries.len()
    }
}

struct CachedFontConfig {
    configured: FontConfig,
    resolved: FontConfig,
    id: u64,
    last_used: u64,
}

#[derive(Clone, Copy, Debug, Hash, PartialEq, Eq)]
struct RenderSize {
    width: i32,
    height: i32,
}

struct MeasureCache {
    entries: SmallLru<(u64, String), i32>,
}

impl MeasureCache {
    fn new() -> Self {
        Self {
            entries: SmallLru::new(MEASURE_CACHE_ENTRY_LIMIT, MEASURE_CACHE_TEXT_LIMIT),
        }
    }

    fn get(&mut self, font_config_id: u64, text: &str) -> Option<i32> {
        self.entries
            .get_mut_by(|(id, cached)| *id == font_config_id && cached == text)
            .copied()
    }

    fn insert(&mut self, font_config_id: u64, text: &str, width: i32) {
        if text.len() > MEASURE_CACHE_TEXT_LIMIT {
            return;
        }
        self.entries
            .insert((font_config_id, text.to_owned()), width, text.len());
    }

    fn remove_font_config(&mut self, id: u64) {
        self.entries.retain(|(cached_id, _)| *cached_id != id);
    }
}

struct CachedRenderedText {
    buffer: Buffer,
}

struct RenderCache {
    entries: SmallLru<(u64, String, RenderSize), CachedRenderedText>,
}

impl RenderCache {
    fn new() -> Self {
        Self {
            entries: SmallLru::new(RENDER_CACHE_ENTRY_LIMIT, RENDER_CACHE_TEXT_LIMIT),
        }
    }

    fn get_mut(
        &mut self,
        font_config_id: u64,
        text: &str,
        size: RenderSize,
    ) -> Option<&mut CachedRenderedText> {
        self.entries.get_mut_by(|(id, cached, cached_size)| {
            *id == font_config_id && cached == text && *cached_size == size
        })
    }

    fn insert(
        &mut self,
        font_config_id: u64,
        text: &str,
        size: RenderSize,
        rendered: CachedRenderedText,
    ) -> bool {
        self.entries.insert(
            (font_config_id, text.to_owned(), size),
            rendered,
            text.len(),
        )
    }

    fn remove_font_config(&mut self, id: u64) {
        self.entries.retain(|(cached_id, _, _)| *cached_id != id);
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
    font_config_clock: u64,
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
            measure_cache: RefCell::new(MeasureCache::new()),
            render_cache: RefCell::new(RenderCache::new()),
            font_configs: vec![CachedFontConfig {
                configured,
                resolved,
                id: 0,
                last_used: 0,
            }],
            active_font_config: 0,
            next_font_config_id: 1,
            font_config_clock: 0,
        }
    }
}

impl TextRasterizer {
    pub(super) fn set_fonts(&mut self, configured: &FontConfig) {
        self.font_config_clock = self.font_config_clock.wrapping_add(1);
        if self.font_configs[self.active_font_config].configured == *configured {
            self.font_configs[self.active_font_config].last_used = self.font_config_clock;
            return;
        }

        if let Some(index) = self
            .font_configs
            .iter()
            .position(|cached| cached.configured == *configured)
        {
            self.active_font_config = index;
            self.font_configs[index].last_used = self.font_config_clock;
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

        if self.font_configs.len() >= FONT_CONFIG_LIMIT {
            let evicted = self
                .font_configs
                .iter()
                .enumerate()
                .min_by_key(|(_, cached)| cached.last_used)
                .map(|(index, _)| index)
                .expect("font cache is non-empty");
            let evicted_id = self.font_configs.remove(evicted).id;
            self.measure_cache.get_mut().remove_font_config(evicted_id);
            self.render_cache.get_mut().remove_font_config(evicted_id);
            // SwashCache has no selective eviction API. Configuration churn is
            // rare, and resetting it here prevents glyph images for evicted
            // font sizes and families from accumulating forever.
            *self.swash_cache.get_mut() = SwashCache::new();
            if self.active_font_config > evicted {
                self.active_font_config -= 1;
            }
        }

        self.font_configs.push(CachedFontConfig {
            configured: configured.clone(),
            resolved,
            id,
            last_used: self.font_config_clock,
        });
        self.active_font_config = self.font_configs.len() - 1;
    }

    pub(super) fn width(&self, text: &str, box_height: i32) -> i32 {
        if text.is_empty() {
            return 0;
        }
        let font_config_id = self.active_fonts().id;

        if let Some(width) = self.measure_cache.borrow_mut().get(font_config_id, text) {
            return width;
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
            .borrow_mut()
            .get_mut(font_config_id, text, size)
            .is_some();
        let mut uncached = None;
        if !is_cached {
            let mut fs = self.font_system.borrow_mut();
            let metrics = Metrics::new(font_size, bounds.h as f32);
            let mut buffer = Buffer::new(&mut fs, metrics);
            buffer.set_size(Some(bounds.w as f32), Some(bounds.h as f32));
            buffer.set_wrap(Wrap::None);
            self.set_buffer_text(&mut buffer, text, bounds.h);
            buffer.shape_until_scroll(&mut fs, false);

            let rendered = CachedRenderedText { buffer };
            if text.len() <= RENDER_CACHE_TEXT_LIMIT {
                let inserted =
                    self.render_cache
                        .borrow_mut()
                        .insert(font_config_id, text, size, rendered);
                debug_assert!(inserted);
            } else {
                // Oversized status strings are rendered normally but are not
                // allowed to evict the useful working set or stay resident.
                uncached = Some(rendered);
            }
        }

        let mut fs = self.font_system.borrow_mut();
        let mut sc = self.swash_cache.borrow_mut();
        let mut cache = self.render_cache.borrow_mut();
        let cached = if let Some(uncached) = uncached.as_mut() {
            uncached
        } else if let Some(cached) = cache.get_mut(font_config_id, text, size) {
            cached
        } else {
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
    use super::{
        FONT_CONFIG_LIMIT, MEASURE_CACHE_ENTRY_LIMIT, MeasureCache, SmallLru, TextRasterizer,
        attrs_for_run,
    };
    use crate::core_state::FontConfig;
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
        let base_entries = rasterizer.measure_cache.get_mut().entries.len();
        assert!(base_entries > 0);

        rasterizer.set_fonts(&base.scaled(2.0));
        rasterizer.width("cache me", 60);
        assert!(rasterizer.measure_cache.get_mut().entries.len() > base_entries);

        rasterizer.set_fonts(&base);
        assert_eq!(
            rasterizer.measure_cache.get_mut().entries.len(),
            base_entries + 1
        );
        assert_eq!(rasterizer.font_configs.len(), 2);
    }

    #[test]
    fn measurement_cache_is_bounded_and_evicts_the_oldest_entry() {
        let mut cache = MeasureCache::new();
        for index in 0..=MEASURE_CACHE_ENTRY_LIMIT {
            cache.insert(0, &format!("entry-{index}"), index as i32);
        }

        assert_eq!(cache.entries.len(), MEASURE_CACHE_ENTRY_LIMIT);
        assert_eq!(cache.get(0, "entry-0"), None);
        assert_eq!(
            cache.get(0, &format!("entry-{MEASURE_CACHE_ENTRY_LIMIT}")),
            Some(MEASURE_CACHE_ENTRY_LIMIT as i32)
        );
    }

    #[test]
    fn recently_used_entries_survive_churn() {
        let mut cache = SmallLru::new(3, 100);
        assert!(cache.insert("stable", 1, 6));
        assert!(cache.insert("old", 2, 3));
        assert!(cache.insert("newer", 3, 5));
        assert_eq!(cache.get_mut_by(|key| *key == "stable"), Some(&mut 1));

        assert!(cache.insert("newest", 4, 6));
        assert!(cache.get_mut_by(|key| *key == "old").is_none());
        assert_eq!(cache.get_mut_by(|key| *key == "stable"), Some(&mut 1));
    }

    #[test]
    fn text_budget_rejects_one_oversized_entry_and_bounds_total_text() {
        let mut cache = SmallLru::new(10, 8);
        assert!(!cache.insert("oversized", (), 9));
        assert!(cache.insert("first", (), 5));
        assert!(cache.insert("second", (), 5));

        assert_eq!(cache.len(), 1);
        assert_eq!(cache.text_bytes, 5);
        assert!(cache.get_mut_by(|key| *key == "second").is_some());
    }

    #[test]
    fn font_configuration_churn_has_a_fixed_working_set() {
        let mut rasterizer = TextRasterizer::default();
        for index in 0..FONT_CONFIG_LIMIT + 4 {
            rasterizer.set_fonts(&FontConfig {
                text_size: 10.0 + index as f32,
                ..FontConfig::default()
            });
            rasterizer.width(&format!("font-{index}"), 30);
        }

        assert_eq!(rasterizer.font_configs.len(), FONT_CONFIG_LIMIT);
        let retained_ids: Vec<_> = rasterizer
            .font_configs
            .iter()
            .map(|config| config.id)
            .collect();
        assert!(
            rasterizer
                .measure_cache
                .get_mut()
                .entries
                .entries
                .iter()
                .all(|entry| retained_ids.contains(&entry.key.0))
        );
    }
}
