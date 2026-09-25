use smithay::backend::allocator::Fourcc;
use smithay::backend::renderer::element::memory::MemoryRenderBuffer;
use smithay::utils::Transform;

use crate::bar::canvas::Canvas;
use crate::types::Point;

pub struct BarBuffer {
    pub buffer: MemoryRenderBuffer,
    pub position: Point,
}

#[derive(Clone)]
pub(super) struct RawBarBuffer {
    pub(super) canvas: Canvas,
    /// Where the canvas sits in the global coordinate space.
    pub(super) position: Point,
}

impl Clone for BarBuffer {
    fn clone(&self) -> Self {
        Self {
            buffer: self.buffer.clone(),
            position: self.position,
        }
    }
}

impl From<&RawBarBuffer> for BarBuffer {
    fn from(raw: &RawBarBuffer) -> Self {
        let size = raw.canvas.size();
        let buffer = MemoryRenderBuffer::from_slice(
            raw.canvas.as_slice(),
            Fourcc::Argb8888,
            (size.w, size.h),
            1,
            Transform::Normal,
            None,
        );
        BarBuffer {
            buffer,
            position: raw.position,
        }
    }
}
