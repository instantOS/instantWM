//! Shared cursor theme loading and standalone cursor rendering.
//!
//! On the DRM backend the compositor must render the cursor itself (there is
//! no host compositor to delegate to).  `CursorManager` loads xcursor
//! theme images lazily on demand and caches them for efficient rendering.
//! CPU-backed frames expose their pixels to Smithay for hardware cursor planes,
//! while retaining texture import for software composition. Animation is supported.

use std::cell::RefCell;
use std::collections::HashMap;
use std::env;
use std::fs::File;
use std::io::Read;
use std::rc::Rc;

use anyhow::Context;
use smithay::backend::allocator::Fourcc;
use smithay::backend::renderer::element::Kind;
use smithay::backend::renderer::element::memory::{
    MemoryRenderBuffer, MemoryRenderBufferRenderElement,
};
use smithay::backend::renderer::gles::GlesRenderer;
use smithay::input::pointer::CursorIcon;
use smithay::utils::{Physical, Point, Transform};
use xcursor::CursorTheme;
use xcursor::parser::{Image, parse_xcursor};

use crate::backend::wayland::render::cursor::ResolvedCursor;

static FALLBACK_CURSOR_DATA: &[u8] = include_bytes!("drm/cursor.rgba");

struct CursorFrame {
    buffer: MemoryRenderBuffer,
    hotspot_x: i32,
    hotspot_y: i32,
}

#[derive(Clone)]
pub struct XCursor {
    images: Vec<Image>,
    animation_duration: u64,
}

impl XCursor {
    fn from_images(mut images: Vec<Image>, size: i32) -> anyhow::Result<Self> {
        let nominal_size = images
            .iter()
            .min_by_key(|image| (i64::from(size) - i64::from(image.size)).abs())
            .map(|image| image.size)
            .context("no images in cursor")?;
        // Animation frames share a nominal size, not necessarily pixel dimensions.
        images.retain(|image| image.size == nominal_size);
        let animation_duration = images.iter().map(|image| u64::from(image.delay)).sum();
        Ok(Self {
            images,
            animation_duration,
        })
    }

    pub fn frames(&self) -> &[Image] {
        &self.images
    }

    pub fn frame(&self, millis: u32) -> (usize, &Image) {
        if self.animation_duration == 0 || self.images.len() <= 1 {
            return (0, &self.images[0]);
        }

        let millis = u64::from(millis) % self.animation_duration;
        let mut accumulated = 0;

        for (i, img) in self.images.iter().enumerate() {
            if accumulated + u64::from(img.delay) > millis {
                return (i, img);
            }
            accumulated += u64::from(img.delay);
        }

        (0, &self.images[0])
    }

    pub fn is_animated(&self) -> bool {
        self.images
            .iter()
            .filter(|image| image.delay > 0)
            .take(2)
            .count()
            > 1
    }
}

type XCursorCache = HashMap<(CursorIcon, i32), Option<Rc<XCursor>>>;
type CursorFrameCache = HashMap<(CursorIcon, i32), Rc<Vec<CursorFrame>>>;

pub struct CursorManager {
    theme: CursorTheme,
    theme_name: String,
    size: u8,
    named_cursor_cache: RefCell<XCursorCache>,
    frame_cache: RefCell<CursorFrameCache>,
}

impl CursorManager {
    pub(crate) fn from_environment(config: &crate::config::config_toml::CursorConfig) -> Self {
        let theme = env::var("XCURSOR_THEME").unwrap_or_else(|_| config.theme.clone());
        let size = env::var("XCURSOR_SIZE")
            .ok()
            .and_then(|value| value.parse::<u32>().ok())
            .unwrap_or(config.size);
        Self::new(&theme, size.clamp(1, u8::MAX as u32) as u8)
    }

    pub fn new(theme: &str, size: u8) -> Self {
        Self::ensure_env(theme, size);

        let theme_name = theme;
        let theme = CursorTheme::load(theme);

        Self {
            theme,
            theme_name: theme_name.to_owned(),
            size,
            named_cursor_cache: Default::default(),
            frame_cache: Default::default(),
        }
    }

    /// Project the same default image and toolkit theme into the X server.
    /// Client-owned cursors remain owned by the application.
    pub(crate) fn configure_xwayland(&self, xwm: &mut smithay::xwayland::X11Wm) {
        let cursor = self.get_cursor(CursorIcon::Default, 1);
        let frame = cursor.frame(0).1;
        if let Err(error) = xwm.set_cursor(
            &frame.pixels_rgba,
            (frame.width as u16, frame.height as u16).into(),
            (frame.xhot as u16, frame.yhot as u16).into(),
        ) {
            log::warn!("failed to set XWayland default cursor: {error}");
        }
        if let Err(error) = xwm.set_xsettings(
            [
                ("Gtk/CursorThemeName".into(), self.theme_name.clone().into()),
                ("Gtk/CursorThemeSize".into(), i32::from(self.size).into()),
            ]
            .into_iter(),
        ) {
            log::warn!("failed to set XWayland cursor settings: {error}");
        }
    }

    fn ensure_env(theme: &str, size: u8) {
        unsafe {
            env::set_var("XCURSOR_THEME", theme);
            env::set_var("XCURSOR_SIZE", size.to_string());
        }
    }

    pub fn reload(&mut self, theme: &str, size: u8) {
        Self::ensure_env(theme, size);
        self.theme = CursorTheme::load(theme);
        self.theme_name = theme.to_owned();
        self.size = size;
        self.named_cursor_cache.get_mut().clear();
        self.frame_cache.get_mut().clear();
    }

    fn load_xcursor(&self, name: &str, size: i32) -> anyhow::Result<XCursor> {
        let path = self
            .theme
            .load_icon(name)
            .ok_or_else(|| anyhow::anyhow!("no cursor icon"))?;

        let mut file = File::open(path).context("error opening cursor icon file")?;
        let mut buf = vec![];
        file.read_to_end(&mut buf)
            .context("error reading cursor icon file")?;

        let images = parse_xcursor(&buf).context("error parsing cursor icon file")?;
        XCursor::from_images(images, size)
    }

    fn get_cursor_with_name(&self, icon: CursorIcon, scale: i32) -> Option<Rc<XCursor>> {
        self.named_cursor_cache
            .borrow_mut()
            .entry((icon, scale))
            .or_insert_with_key(|(icon, scale)| {
                let size = self.size as i32 * scale;
                let mut cursor = self.load_xcursor(icon.name(), size);

                if cursor.is_err() {
                    for name in icon.alt_names() {
                        cursor = self.load_xcursor(name, size);
                        if cursor.is_ok() {
                            break;
                        }
                    }
                }

                if let Err(err) = &cursor {
                    log::warn!("error loading xcursor {}@{size}: {:?}", icon.name(), err);
                }

                if *icon == CursorIcon::Default && cursor.is_err() {
                    cursor = Ok(Self::fallback_cursor());
                }

                cursor.ok().map(Rc::new)
            })
            .clone()
    }

    fn fallback_cursor() -> XCursor {
        let images = vec![Image {
            size: 32,
            width: 64,
            height: 64,
            xhot: 1,
            yhot: 1,
            delay: 0,
            pixels_rgba: Vec::from(FALLBACK_CURSOR_DATA),
            pixels_argb: vec![],
        }];

        XCursor {
            images,
            animation_duration: 0,
        }
    }

    pub fn get_cursor(&self, icon: CursorIcon, scale: i32) -> Rc<XCursor> {
        self.get_cursor_with_name(icon, scale).unwrap_or_else(|| {
            self.get_cursor_with_name(CursorIcon::Default, scale)
                .expect("default cursor must always be available")
        })
    }

    pub fn is_animated(&self, icon: CursorIcon, scale: i32) -> bool {
        self.get_cursor(icon, scale).is_animated()
    }

    fn get_cached_frames(&self, icon: CursorIcon, scale: i32) -> Rc<Vec<CursorFrame>> {
        let key = (icon, scale);
        Rc::clone(self.frame_cache.borrow_mut().entry(key).or_insert_with(|| {
            let cursor = self.get_cursor(icon, scale);
            Rc::new(
                cursor
                    .frames()
                    .iter()
                    .map(|frame| CursorFrame {
                        buffer: cursor_buffer(frame),
                        hotspot_x: frame.xhot as i32,
                        hotspot_y: frame.yhot as i32,
                    })
                    .collect(),
            )
        }))
    }

    pub fn render_element(
        &self,
        pointer_location: Point<f64, smithay::utils::Logical>,
        presentation: &ResolvedCursor,
        scale: i32,
        millis: u32,
        renderer: &mut GlesRenderer,
    ) -> Option<MemoryRenderBufferRenderElement<GlesRenderer>> {
        let icon = match presentation {
            ResolvedCursor::Hidden => return None,
            ResolvedCursor::Named(icon) => *icon,
            ResolvedCursor::Surface { .. } => return None,
            ResolvedCursor::DndIcon { cursor, .. } => {
                return self.render_element(pointer_location, cursor, scale, millis, renderer);
            }
        };

        let cursor = self.get_cursor(icon, scale);
        let frames = self.get_cached_frames(icon, scale);

        if frames.is_empty() {
            return None;
        }

        let (frame_idx, _frame) = cursor.frame(millis);
        let idx = frame_idx.min(frames.len() - 1);

        let cursor_frame = &frames[idx];
        let render_x = pointer_location.x - cursor_frame.hotspot_x as f64;
        let render_y = pointer_location.y - cursor_frame.hotspot_y as f64;
        let pos: Point<f64, Physical> = Point::from((render_x, render_y));

        MemoryRenderBufferRenderElement::from_buffer(
            renderer,
            pos,
            &cursor_frame.buffer,
            None,
            None,
            None,
            Kind::Cursor,
        )
        .map_err(|err| log::warn!("cursor texture import failed: {err}"))
        .ok()
    }
}

// KMS cursor buffers use ARGB8888 (BGRA bytes on little-endian machines).
// xcursor's RGBA bytes need conversion; its pixels_argb vector is empty for
// the built-in fallback, so use the same conversion for every source.
fn cursor_buffer(frame: &Image) -> MemoryRenderBuffer {
    let pixels: Vec<u8> = frame
        .pixels_rgba
        .chunks_exact(4)
        .flat_map(|pixel| {
            u32::from_be_bytes([pixel[3], pixel[0], pixel[1], pixel[2]]).to_ne_bytes()
        })
        .collect();
    MemoryRenderBuffer::from_slice(
        &pixels,
        Fourcc::Argb8888,
        (frame.width as i32, frame.height as i32),
        1,
        Transform::Normal,
        None,
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    fn image(size: u32, width: u32, delay: u32) -> Image {
        Image {
            size,
            width,
            height: width,
            xhot: 0,
            yhot: 0,
            delay,
            pixels_rgba: Vec::new(),
            pixels_argb: Vec::new(),
        }
    }

    #[test]
    fn cursor_pixels_preserve_color_alpha_and_fallback() {
        let mut frame = image(1, 1, 0);
        frame.pixels_rgba = vec![0x12, 0x34, 0x56, 0x78];
        let mut buffer = cursor_buffer(&frame);
        buffer
            .render()
            .draw(|pixels| {
                assert_eq!(pixels, &0x78123456u32.to_ne_bytes());
                Ok::<_, ()>(Vec::new())
            })
            .unwrap();

        let fallback = CursorManager::fallback_cursor();
        let mut buffer = cursor_buffer(&fallback.frames()[0]);
        buffer
            .render()
            .draw(|pixels| {
                assert_eq!(pixels.len(), 64 * 64 * 4);
                for (actual, rgba) in pixels
                    .chunks_exact(4)
                    .zip(FALLBACK_CURSOR_DATA.chunks_exact(4))
                {
                    assert_eq!(
                        u32::from_ne_bytes(actual.try_into().unwrap()),
                        u32::from_be_bytes([rgba[3], rgba[0], rgba[1], rgba[2]])
                    );
                }
                Ok::<_, ()>(Vec::new())
            })
            .unwrap();
    }

    #[test]
    fn selects_nominal_size_and_preserves_differently_sized_frames() {
        let cursor = XCursor::from_images(
            vec![image(24, 32, 10), image(32, 32, 10), image(24, 28, 20)],
            24,
        )
        .unwrap();
        assert_eq!(cursor.frames().len(), 2);
        assert!(cursor.frames().iter().all(|frame| frame.size == 24));
        assert_eq!(cursor.frame(0).1.width, 32);
        assert_eq!(cursor.frame(10).1.width, 28);
        assert_eq!(cursor.frame(29).0, 1);
        assert_eq!(cursor.frame(30).0, 0);
    }

    #[test]
    fn static_frames_do_not_request_animation_redraws() {
        for delays in [[0, 0], [0, 10], [10, 0]] {
            let cursor = XCursor::from_images(
                delays
                    .into_iter()
                    .map(|delay| image(24, 24, delay))
                    .collect(),
                24,
            )
            .unwrap();
            assert!(!cursor.is_animated());
            assert_eq!(cursor.frame(0).0, cursor.frame(100).0);
        }
    }

    #[test]
    fn large_frame_delays_and_nominal_sizes_do_not_overflow() {
        let cursor = XCursor::from_images(
            vec![image(u32::MAX, 24, u32::MAX), image(u32::MAX, 24, 10)],
            24,
        )
        .unwrap();
        assert_eq!(cursor.animation_duration, u64::from(u32::MAX) + 10);
        assert_eq!(cursor.frame(u32::MAX).0, 1);
        assert!(cursor.is_animated());
        assert!(XCursor::from_images(Vec::new(), 24).is_err());
    }
}
