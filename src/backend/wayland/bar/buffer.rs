use smithay::backend::allocator::Fourcc;
use smithay::backend::renderer::element::memory::MemoryRenderBuffer;
use smithay::utils::Transform;

use crate::types::{Point, Rect};

pub struct BarBuffer {
    pub buffer: MemoryRenderBuffer,
    pub position: Point,
}

#[derive(Clone)]
pub(super) struct RawBarBuffer {
    pub(super) pixels: Vec<u8>,
    pub(super) rect: Rect,
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
        let buffer = MemoryRenderBuffer::from_slice(
            &raw.pixels,
            Fourcc::Argb8888,
            (raw.rect.w, raw.rect.h),
            1,
            Transform::Normal,
            None,
        );
        BarBuffer {
            buffer,
            position: raw.rect.position(),
        }
    }
}
