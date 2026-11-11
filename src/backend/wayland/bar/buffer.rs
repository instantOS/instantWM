use smithay::backend::allocator::Fourcc;
use smithay::backend::renderer::element::memory::MemoryRenderBuffer;
use smithay::utils::Transform;

use crate::bar::canvas::Canvas;
use crate::types::{Point, Size};

#[derive(Clone)]
pub struct BarBuffer {
    pub buffer: MemoryRenderBuffer,
    pub position: Point,
    pub logical_size: Size,
    pub raster_scale: i32,
}

#[derive(Clone)]
pub(super) struct RawBarBuffer {
    pub(super) canvas: Canvas,
    /// Where the canvas sits in the global coordinate space.
    pub(super) position: Point,
    pub(super) raster_scale: i32,
}

impl From<&RawBarBuffer> for BarBuffer {
    fn from(raw: &RawBarBuffer) -> Self {
        let size = raw.canvas.size();
        let buffer = MemoryRenderBuffer::from_slice(
            raw.canvas.as_slice(),
            Fourcc::Argb8888,
            (size.w, size.h),
            raw.raster_scale,
            Transform::Normal,
            None,
        );
        BarBuffer {
            buffer,
            position: raw.position,
            logical_size: Size::new(size.w / raw.raster_scale, size.h / raw.raster_scale),
            raster_scale: raw.raster_scale,
        }
    }
}
