use smithay::backend::allocator::Fourcc;
use smithay::backend::renderer::element::memory::MemoryRenderBuffer;
use smithay::utils::Transform;

pub struct BarBuffer {
    pub buffer: MemoryRenderBuffer,
    pub x: i32,
    pub y: i32,
}

#[derive(Clone)]
pub(super) struct RawBarBuffer {
    pub(super) pixels: Vec<u8>,
    pub(super) width: i32,
    pub(super) height: i32,
    pub(super) x: i32,
    pub(super) y: i32,
}

impl Clone for BarBuffer {
    fn clone(&self) -> Self {
        Self {
            buffer: self.buffer.clone(),
            x: self.x,
            y: self.y,
        }
    }
}

pub(super) fn raw_to_bar_buffer(raw: &RawBarBuffer) -> BarBuffer {
    let buffer = MemoryRenderBuffer::from_slice(
        &raw.pixels,
        Fourcc::Argb8888,
        (raw.width, raw.height),
        1,
        Transform::Normal,
        None,
    );
    BarBuffer {
        buffer,
        x: raw.x,
        y: raw.y,
    }
}
