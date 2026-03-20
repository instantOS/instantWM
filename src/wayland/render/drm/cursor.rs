//! Cursor loading and rendering for the standalone DRM/KMS backend.
//!
//! On the DRM backend the compositor must render the cursor itself (there is
//! no host compositor to delegate to).  `CursorManager` loads xcursor
//! theme images lazily on demand and caches them for efficient rendering.
//! Animation is supported for cursor themes that provide animated cursors.

use std::cell::RefCell;
use std::collections::HashMap;
use std::env;
use std::fs::File;
use std::io::Read;
use std::rc::Rc;

use anyhow::Context;
use smithay::backend::allocator::Fourcc;
use smithay::backend::renderer::element::Kind;
use smithay::backend::renderer::element::texture::{TextureBuffer, TextureRenderElement};
use smithay::backend::renderer::gles::{GlesRenderer, GlesTexture};
use smithay::input::pointer::CursorIcon;
use smithay::utils::{Physical, Point, Transform};
use xcursor::CursorTheme;
use xcursor::parser::{Image, parse_xcursor};

use crate::wayland::common::CursorPresentation;

static FALLBACK_CURSOR_DATA: &[u8] = include_bytes!("cursor.rgba");

struct CursorFrame {
    buffer: TextureBuffer<GlesTexture>,
    hotspot_x: i32,
    hotspot_y: i32,
}

#[derive(Clone)]
pub struct XCursor {
    images: Vec<Image>,
    animation_duration: u32,
}

impl XCursor {
    pub fn frames(&self) -> &[Image] {
        &self.images
    }

    pub fn frame(&self, millis: u32) -> (usize, &Image) {
        if self.animation_duration == 0 || self.images.len() <= 1 {
            return (0, &self.images[0]);
        }

        let millis = millis % self.animation_duration;
        let mut accumulated = 0;

        for (i, img) in self.images.iter().enumerate() {
            if accumulated + img.delay > millis {
                return (i, img);
            }
            accumulated += img.delay;
        }

        (0, &self.images[0])
    }

    pub fn is_animated(&self) -> bool {
        self.images.len() > 1
    }

    pub fn hotspot(image: &Image) -> Point<i32, Physical> {
        (image.xhot as i32, image.yhot as i32).into()
    }
}

type XCursorCache = HashMap<(CursorIcon, i32), Option<Rc<XCursor>>>;

pub struct CursorManager {
    theme: CursorTheme,
    size: u8,
    named_cursor_cache: RefCell<XCursorCache>,
    frame_cache: RefCell<HashMap<(CursorIcon, i32), Rc<Vec<CursorFrame>>>>,
}

impl CursorManager {
    pub fn new(theme: &str, size: u8) -> Self {
        Self::ensure_env(theme, size);

        let theme = CursorTheme::load(theme);

        Self {
            theme,
            size,
            named_cursor_cache: Default::default(),
            frame_cache: Default::default(),
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

        let mut images = parse_xcursor(&buf).context("error parsing cursor icon file")?;

        if images.is_empty() {
            anyhow::bail!("no images in cursor");
        }

        let (width, height) = images
            .iter()
            .min_by_key(|image| (size - image.size as i32).abs())
            .map(|image| (image.width, image.height))
            .unwrap();

        images.retain(|image| image.width == width && image.height == height);

        let animation_duration = images.iter().fold(0, |acc, img| acc + img.delay);

        Ok(XCursor {
            images,
            animation_duration,
        })
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
                .unwrap()
        })
    }

    pub fn is_animated(&self, icon: CursorIcon, scale: i32) -> bool {
        self.get_cursor(icon, scale).is_animated()
    }

    fn get_cached_frames(
        &self,
        renderer: &mut GlesRenderer,
        icon: CursorIcon,
        scale: i32,
    ) -> Rc<Vec<CursorFrame>> {
        let key = (icon, scale);
        if !self.frame_cache.borrow().contains_key(&key) {
            let cursor = self.get_cursor(icon, scale);
            let frames: Vec<CursorFrame> = cursor
                .frames()
                .iter()
                .filter_map(|frame| {
                    let buf = TextureBuffer::from_memory(
                        renderer,
                        &frame.pixels_rgba,
                        Fourcc::Abgr8888,
                        (frame.width as i32, frame.height as i32),
                        false,
                        1,
                        Transform::Normal,
                        None,
                    )
                    .ok()?;
                    Some(CursorFrame {
                        buffer: buf,
                        hotspot_x: frame.xhot as i32,
                        hotspot_y: frame.yhot as i32,
                    })
                })
                .collect();

            if frames.is_empty() {
                let fallback = self.create_fallback_frame(renderer);
                self.frame_cache
                    .borrow_mut()
                    .insert(key, Rc::new(vec![fallback]));
            } else {
                self.frame_cache.borrow_mut().insert(key, Rc::new(frames));
            }
        }

        Rc::clone(self.frame_cache.borrow().get(&key).unwrap())
    }

    fn create_fallback_frame(&self, renderer: &mut GlesRenderer) -> CursorFrame {
        const W: usize = 8;
        const H: usize = 8;
        let pixels: Vec<u8> = vec![0xFF; W * H * 4];
        let buffer = TextureBuffer::from_memory(
            renderer,
            &pixels,
            Fourcc::Abgr8888,
            (W as i32, H as i32),
            false,
            1,
            Transform::Normal,
            None,
        )
        .expect("create_fallback_frame: from_memory failed");

        CursorFrame {
            buffer,
            hotspot_x: 0,
            hotspot_y: 0,
        }
    }

    pub fn render_element(
        &self,
        pointer_location: Point<f64, smithay::utils::Logical>,
        presentation: &CursorPresentation,
        scale: i32,
        millis: u32,
        renderer: &mut GlesRenderer,
    ) -> Option<TextureRenderElement<GlesTexture>> {
        let icon = match presentation {
            CursorPresentation::Hidden => return None,
            CursorPresentation::Named(icon) => *icon,
            CursorPresentation::Surface { .. } => return None,
            CursorPresentation::DndIcon { cursor, .. } => {
                return self.render_element(pointer_location, cursor, scale, millis, renderer);
            }
        };

        let cursor = self.get_cursor(icon, scale);
        let frames = self.get_cached_frames(renderer, icon, scale);

        if frames.is_empty() {
            return None;
        }

        let (frame_idx, _frame) = cursor.frame(millis);
        let idx = frame_idx.min(frames.len() - 1);

        let cursor_frame = &frames[idx];
        let render_x = pointer_location.x - cursor_frame.hotspot_x as f64;
        let render_y = pointer_location.y - cursor_frame.hotspot_y as f64;
        let pos: Point<f64, Physical> = Point::from((render_x, render_y));

        Some(TextureRenderElement::from_texture_buffer(
            pos,
            &cursor_frame.buffer,
            None,
            None,
            None,
            Kind::Cursor,
        ))
    }
}
