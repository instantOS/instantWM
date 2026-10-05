use super::icon_theme::{IconThemes, desktop_icon_theme};
use super::*;
use std::collections::VecDeque;
use std::io::{Cursor, Read};
use std::os::unix::fs::MetadataExt;
use std::path::{Path, PathBuf};

pub(super) type IconPixmaps = Vec<(i32, i32, Vec<u8>)>;
pub(super) type IconImage = (Arc<[u8]>, Size);
const MAX_ICON_BYTES: usize = 16 * 1024 * 1024;
const MAX_ICON_PIXELS: usize = 1024 * 1024;
const MAX_CACHED_ICONS: usize = 128;
const MAX_CACHE_BYTES: usize = 8 * 1024 * 1024;

#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct IconSettings {
    pub theme: Option<String>,
    pub height: u32,
}

impl Default for IconSettings {
    fn default() -> Self {
        Self {
            theme: None,
            height: 24,
        }
    }
}

struct CachedIcon {
    path: PathBuf,
    height: u32,
    modified: Option<std::time::SystemTime>,
    len: u64,
    identity: (u64, u64, i64, i64),
    image: IconImage,
}

/// Owned by the discovery thread. The bounded LRU holds pixels, not SVG trees
/// or font databases, and checks file metadata before reusing an image.
pub(super) struct IconResolver {
    settings: IconSettings,
    theme: String,
    themes: IconThemes,
    cache: VecDeque<CachedIcon>,
}

impl Default for IconResolver {
    fn default() -> Self {
        Self {
            settings: IconSettings::default(),
            theme: desktop_icon_theme(),
            themes: IconThemes::system(),
            cache: VecDeque::new(),
        }
    }
}

impl IconResolver {
    pub(super) fn configure(&mut self, settings: IconSettings) {
        self.settings = settings;
        self.refresh_theme();
    }

    pub(super) fn refresh_theme(&mut self) {
        self.theme = self
            .settings
            .theme
            .clone()
            .filter(|s| !s.is_empty())
            .unwrap_or_else(desktop_icon_theme);
    }

    fn named(&mut self, name: &str, theme_path: &str) -> Option<IconImage> {
        let height = self.settings.height.clamp(1, 1024);
        let path = self.themes.find(name, theme_path, &self.theme, height)?;
        self.load(&path, height)
    }

    fn load(&mut self, path: &Path, height: u32) -> Option<IconImage> {
        let metadata = std::fs::metadata(path).ok()?;
        let modified = metadata.modified().ok();
        let identity = (
            metadata.dev(),
            metadata.ino(),
            metadata.ctime(),
            metadata.ctime_nsec(),
        );
        if let Some(index) = self.cache.iter().position(|entry| {
            entry.path == path
                && entry.height == height
                && entry.modified == modified
                && entry.len == metadata.len()
                && entry.identity == identity
        }) {
            let entry = self.cache.remove(index)?;
            let image = entry.image.clone();
            self.cache.push_back(entry);
            return Some(image);
        }
        self.cache
            .retain(|entry| entry.path != path || entry.height != height);
        if !metadata.is_file() || metadata.len() > MAX_ICON_BYTES as u64 {
            return None;
        }
        let mut bytes = Vec::new();
        std::fs::File::open(path)
            .ok()?
            .take(MAX_ICON_BYTES as u64 + 1)
            .read_to_end(&mut bytes)
            .ok()?;
        if bytes.len() > MAX_ICON_BYTES {
            return None;
        }
        let image = decode_icon(&bytes, height)?;
        while self.cache.len() >= MAX_CACHED_ICONS
            || self
                .cache
                .iter()
                .map(|entry| entry.image.0.len())
                .sum::<usize>()
                + image.0.len()
                > MAX_CACHE_BYTES
        {
            self.cache.pop_front()?;
        }
        self.cache.push_back(CachedIcon {
            path: path.to_owned(),
            height,
            modified,
            len: metadata.len(),
            identity,
            image: image.clone(),
        });
        Some(image)
    }
}

pub(super) fn fetch_item_icon_on_conn(
    conn: &Connection,
    service: &str,
    path: &str,
    icons: &mut IconResolver,
) -> Option<IconImage> {
    let proxy = uncached_proxy(conn, service, path, ITEM_IFACE).ok()?;
    // Prefer the theme's artwork, as recommended by StatusNotifierItem. These
    // properties are optional in practice; a failed name read must not prevent
    // pixmap-only clients from appearing.
    let name: String = proxy.get_property("IconName").unwrap_or_default();
    if !name.is_empty() {
        let theme_path: String = proxy.get_property("IconThemePath").unwrap_or_default();
        if let Some(image) = icons.named(&name, &theme_path) {
            return Some(image);
        }
    }
    let pixmaps: IconPixmaps = proxy.get_property("IconPixmap").ok()?;
    let (size, bytes) = select_largest_valid_pixmap(pixmaps)?;
    let rgba = dbus_icon_bytes_to_rgba(&bytes, size)?;
    Some((Arc::from(rgba), size))
}

fn decode_icon(bytes: &[u8], height: u32) -> Option<IconImage> {
    if bytes.starts_with(b"\x89PNG\r\n\x1a\n") {
        return decode_png(bytes);
    }
    let mut options = resvg::usvg::Options::default();
    // Tray artwork is self-contained. Do not let an SVG resolve arbitrary
    // external files; embedded vector data remains supported.
    options.image_href_resolver.resolve_string = Box::new(|_, _| None);
    let tree = resvg::usvg::Tree::from_data(bytes, &options).ok()?;
    if tree.root().children().is_empty() {
        return None;
    }
    let source = tree.size();
    let scale = height.clamp(1, 1024) as f32 / source.height();
    let width = (source.width() * scale).ceil() as u32;
    let height = height.clamp(1, 1024);
    if width == 0 || width > 1024 {
        return None;
    }
    let mut pixmap = resvg::tiny_skia::Pixmap::new(width, height)?;
    resvg::render(
        &tree,
        resvg::tiny_skia::Transform::from_scale(scale, scale),
        &mut pixmap.as_mut(),
    );
    let mut rgba = Vec::with_capacity(pixmap.data().len());
    for pixel in pixmap.pixels() {
        let color = pixel.demultiply();
        rgba.extend_from_slice(&[color.red(), color.green(), color.blue(), color.alpha()]);
    }
    Some((Arc::from(rgba), Size::new(width as i32, height as i32)))
}

fn decode_png(bytes: &[u8]) -> Option<IconImage> {
    let mut decoder = png::Decoder::new(Cursor::new(bytes));
    decoder.set_transformations(png::Transformations::EXPAND | png::Transformations::STRIP_16);
    decoder.set_limits(png::Limits {
        bytes: MAX_ICON_BYTES,
    });
    let mut reader = decoder.read_info().ok()?;
    let info = reader.info();
    let pixels = (info.width as usize).checked_mul(info.height as usize)?;
    if pixels == 0 || pixels > MAX_ICON_PIXELS {
        return None;
    }
    let mut buffer = vec![0; reader.output_buffer_size()?];
    let output = reader.next_frame(&mut buffer).ok()?;
    let data = &buffer[..output.buffer_size()];
    let mut rgba = Vec::with_capacity(pixels * 4);
    match output.color_type {
        png::ColorType::Rgba => rgba.extend_from_slice(data),
        png::ColorType::Rgb => {
            for pixel in data.as_chunks::<3>().0 {
                rgba.extend_from_slice(&[pixel[0], pixel[1], pixel[2], 255]);
            }
        }
        png::ColorType::Grayscale => {
            for value in data {
                rgba.extend_from_slice(&[*value, *value, *value, 255]);
            }
        }
        png::ColorType::GrayscaleAlpha => {
            for pixel in data.as_chunks::<2>().0 {
                rgba.extend_from_slice(&[pixel[0], pixel[0], pixel[0], pixel[1]]);
            }
        }
        png::ColorType::Indexed => return None,
    }
    Some((
        Arc::from(rgba),
        Size::new(output.width as i32, output.height as i32),
    ))
}

pub(super) fn select_largest_valid_pixmap(pixmaps: IconPixmaps) -> Option<(Size, Vec<u8>)> {
    pixmaps
        .into_iter()
        .filter_map(|(width, height, bytes)| {
            let pixels = usize::try_from(width)
                .ok()?
                .checked_mul(usize::try_from(height).ok()?)?;
            let required_bytes = pixels.checked_mul(4)?;
            if width <= 0 || height <= 0 || pixels > MAX_ICON_PIXELS || bytes.len() < required_bytes
            {
                return None;
            }
            let area = i64::from(width) * i64::from(height);
            Some((area, width, height, bytes))
        })
        .max_by_key(|(area, _, _, _)| *area)
        .map(|(_, width, height, bytes)| (Size::new(width, height), bytes))
}

pub(super) fn dbus_icon_bytes_to_rgba(bytes: &[u8], size: Size) -> Option<Vec<u8>> {
    if !size.is_positive() {
        return None;
    }
    let px_count = (size.w as usize).checked_mul(size.h as usize)?;
    let need = px_count.checked_mul(4)?;
    if bytes.len() < need {
        return None;
    }

    let mut out = vec![0u8; need];
    for i in 0..px_count {
        let si = i * 4;
        // StatusNotifierItem::IconPixmap stores ARGB32 pixels in network byte
        // order, so each pixel arrives as A, R, G, B bytes on the wire.
        let a = bytes[si];
        let r = bytes[si + 1];
        let g = bytes[si + 2];
        let b = bytes[si + 3];
        out[si] = r;
        out[si + 1] = g;
        out[si + 2] = b;
        out[si + 3] = a;
    }
    Some(out)
}

#[cfg(test)]
mod tests {
    use super::*;

    pub(super) const SVG: &[u8] = br##"<svg xmlns="http://www.w3.org/2000/svg" width="16" height="8"><rect width="16" height="8" fill="#ff0000" fill-opacity="0.5"/></svg>"##;

    #[test]
    fn svg_preserves_aspect_ratio_and_returns_straight_alpha() {
        let (pixels, size) = decode_icon(SVG, 12).unwrap();
        assert_eq!(size, Size::new(24, 12));
        assert_eq!(&pixels[..4], &[255, 0, 0, 128]);
        assert!(
            pixels
                .as_chunks::<4>()
                .0
                .iter()
                .all(|p| *p == [255, 0, 0, 128])
        );
    }

    #[test]
    fn png_expands_palette_transparency_without_a_filename_extension() {
        let mut bytes = Vec::new();
        {
            let mut encoder = png::Encoder::new(&mut bytes, 2, 1);
            encoder.set_color(png::ColorType::Indexed);
            encoder.set_depth(png::BitDepth::Eight);
            encoder.set_palette(vec![255, 0, 0, 0, 255, 0]);
            encoder.set_trns(vec![128, 255]);
            encoder
                .write_header()
                .unwrap()
                .write_image_data(&[0, 1])
                .unwrap();
        }
        let dir = tempfile::tempdir().unwrap();
        let file = dir.path().join("systray_random");
        std::fs::write(&file, bytes).unwrap();
        let mut resolver = IconResolver::default();
        let (pixels, size) = resolver.named(file.to_str().unwrap(), "").unwrap();
        assert_eq!(size, Size::new(2, 1));
        assert_eq!(&*pixels, &[255, 0, 0, 128, 0, 255, 0, 255]);
    }

    #[test]
    fn pixels_are_cached_and_file_replacements_and_size_changes_are_seen() {
        let dir = tempfile::tempdir().unwrap();
        let file = dir.path().join("icon.svg");
        std::fs::write(&file, SVG).unwrap();
        let mut resolver = IconResolver::default();
        let first = resolver.load(&file, 24).unwrap();
        let second = resolver.load(&file, 24).unwrap();
        assert!(Arc::ptr_eq(&first.0, &second.0));
        let larger = resolver.load(&file, 48).unwrap();
        assert_eq!(larger.1, Size::new(96, 48));
        assert!(!Arc::ptr_eq(&first.0, &larger.0));
        let changed = String::from_utf8(SVG.to_vec())
            .unwrap()
            .replace("#ff0000", "#0000ff");
        // Replace the file with a different length, independent of timestamp resolution.
        std::fs::write(&file, format!("{changed}\n")).unwrap();
        let updated = resolver.load(&file, 24).unwrap();
        assert_eq!(&updated.0[..4], &[0, 0, 255, 128]);
        assert!(!Arc::ptr_eq(&first.0, &updated.0));
        std::fs::remove_file(&file).unwrap();
        assert!(resolver.load(&file, 24).is_none());
    }

    #[test]
    fn missing_icons_are_retried_and_cache_is_bounded() {
        let dir = tempfile::tempdir().unwrap();
        let file = dir.path().join("icon.svg");
        let mut resolver = IconResolver::default();
        assert!(resolver.load(&file, 1).is_none());
        std::fs::write(&file, SVG).unwrap();
        for height in 1..=MAX_CACHED_ICONS as u32 + 1 {
            assert!(resolver.load(&file, height).is_some());
        }
        assert_eq!(resolver.cache.len(), MAX_CACHED_ICONS);
        assert_eq!(resolver.cache.front().unwrap().height, 2);
    }

    #[test]
    fn invalid_or_excessively_large_icons_are_rejected() {
        assert!(decode_icon(b"not an image", 24).is_none());
        let wide = br#"<svg xmlns="http://www.w3.org/2000/svg" width="100000" height="1"/>"#;
        assert!(decode_icon(wide, 24).is_none());
        assert!(select_largest_valid_pixmap(vec![(0, 1, vec![]), (2048, 2048, vec![])]).is_none());
    }
}
