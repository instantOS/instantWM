//! Projection of a source output's scene onto a mirror head.
//!
//! A realized mirror is not part of the space. Its frames are the source's
//! render elements, cropped to the source's content, scaled uniformly and
//! centered in the mirror's framebuffer. The mirror head keeps its own mode and
//! transform; its output scale is pinned to the source's (see
//! `OutputTransaction::apply_mirrors`) because the elements are built at that
//! scale and each output's damage tracker measures elements at its own scale.

use smithay::backend::renderer::element::Element;
use smithay::backend::renderer::element::utils::{
    CropRenderElement, Relocate, RelocateRenderElement, RescaleRenderElement,
};
use smithay::output::Output;
use smithay::utils::{Logical, Physical, Point, Rectangle, Scale, Size};

use crate::backend::wayland::compositor::WaylandState;
use crate::config::config_toml::MirrorFit;

pub type MirroredElement<E> = RelocateRenderElement<RescaleRenderElement<CropRenderElement<E>>>;

/// How a source output's content maps into a mirror head's framebuffer.
///
/// Both sizes are physical and in each output's transformed orientation, the
/// space render elements are positioned in; each damage tracker applies its
/// output's transform afterwards, so the heads may be rotated differently.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct MirrorProjection {
    /// Source content area that elements are cropped to.
    content: Size<i32, Physical>,
    /// Uniform factor from source to mirror pixels.
    scale: f64,
    /// Mirror position of the scaled content's top-left corner. Negative
    /// components crop (cover), positive ones leave bars (contain).
    offset: Point<i32, Physical>,
}

impl MirrorProjection {
    /// Fit `content` into `target`, letterboxed (`Contain`) or cropped to fill
    /// (`Cover`), centered either way.
    pub fn fit(
        content: Size<i32, Physical>,
        target: Size<i32, Physical>,
        fit: MirrorFit,
    ) -> Option<Self> {
        if content.w <= 0 || content.h <= 0 || target.w <= 0 || target.h <= 0 {
            return None;
        }
        let ratio_w = f64::from(target.w) / f64::from(content.w);
        let ratio_h = f64::from(target.h) / f64::from(content.h);
        let scale = match fit {
            MirrorFit::Contain => ratio_w.min(ratio_h),
            MirrorFit::Cover => ratio_w.max(ratio_h),
        };
        let scaled_w = (f64::from(content.w) * scale).round() as i32;
        let scaled_h = (f64::from(content.h) * scale).round() as i32;
        Some(Self {
            content,
            scale,
            offset: Point::from(((target.w - scaled_w) / 2, (target.h - scaled_h) / 2)),
        })
    }

    /// Wrap one element built for the source at `element_scale`. Elements
    /// entirely outside the source's content are dropped; the crop keeps
    /// content that overhangs the source out of a contain mirror's bars.
    pub fn project<E: Element>(
        &self,
        element: E,
        element_scale: Scale<f64>,
    ) -> Option<MirroredElement<E>> {
        let cropped = CropRenderElement::from_element(
            element,
            element_scale,
            Rectangle::from_size(self.content),
        )?;
        let scaled = RescaleRenderElement::from_element(cropped, Point::default(), self.scale);
        Some(RelocateRenderElement::from_element(
            scaled,
            self.offset,
            Relocate::Relative,
        ))
    }

    /// Map a mirror framebuffer point back into the source's content, or
    /// `None` inside a contain mirror's bars.
    pub fn unproject(&self, point: Point<f64, Physical>) -> Option<Point<f64, Physical>> {
        let source = Point::<f64, Physical>::from((
            (point.x - f64::from(self.offset.x)) / self.scale,
            (point.y - f64::from(self.offset.y)) / self.scale,
        ));
        (source.x >= 0.0
            && source.y >= 0.0
            && source.x < f64::from(self.content.w)
            && source.y < f64::from(self.content.h))
        .then_some(source)
    }
}

/// Physical size of an output's current mode in its transformed orientation.
pub fn transformed_mode_size(output: &Output) -> Option<Size<i32, Physical>> {
    let mode = output.current_mode()?;
    Some(output.current_transform().transform_size(mode.size))
}

/// The source output and projection for a realized mirror head.
pub fn mirror_projection(
    state: &WaylandState,
    mirror: &Output,
) -> Option<(Output, MirrorProjection)> {
    let name = mirror.name();
    let source_name = state.runtime.realized_mirrors.get(&name)?;
    let source = state
        .space
        .outputs()
        .find(|output| output.name() == *source_name)?
        .clone();
    let fit = state.runtime.mirror_of.fit_of(&name).unwrap_or_default();
    let projection = MirrorProjection::fit(
        transformed_mode_size(&source)?,
        transformed_mode_size(mirror)?,
        fit,
    )?;
    Some((source, projection))
}

/// Map a physical point on a realized mirror head (transformed orientation)
/// to the logical point of the source content it shows.
pub fn mirror_point_to_logical(
    state: &WaylandState,
    mirror: &Output,
    point: Point<f64, Physical>,
) -> Option<Point<f64, Logical>> {
    let (source, projection) = mirror_projection(state, mirror)?;
    let origin = state.space.output_geometry(&source)?.loc.to_f64();
    let scale = source.current_scale().fractional_scale();
    Some(projection.unproject(point)?.to_logical(scale) + origin)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn size(w: i32, h: i32) -> Size<i32, Physical> {
        Size::from((w, h))
    }

    #[test]
    fn identical_sizes_project_one_to_one() {
        let projection =
            MirrorProjection::fit(size(1920, 1080), size(1920, 1080), MirrorFit::Contain).unwrap();
        assert_eq!(projection.scale, 1.0);
        assert_eq!(projection.offset, Point::from((0, 0)));
    }

    #[test]
    fn same_aspect_scales_without_bars() {
        let projection =
            MirrorProjection::fit(size(2560, 1440), size(1920, 1080), MirrorFit::Contain).unwrap();
        assert_eq!(projection.scale, 0.75);
        assert_eq!(projection.offset, Point::from((0, 0)));
    }

    #[test]
    fn contain_letterboxes_an_aspect_mismatch() {
        // 16:9 onto 16:10: fit the width, bars of (1200 - 1080) / 2 above and below.
        let projection =
            MirrorProjection::fit(size(2560, 1440), size(1920, 1200), MirrorFit::Contain).unwrap();
        assert_eq!(projection.scale, 0.75);
        assert_eq!(projection.offset, Point::from((0, 60)));
    }

    #[test]
    fn cover_crops_an_aspect_mismatch() {
        // 16:9 onto 16:10: fill the height; the 2133px wide content loses
        // (2133 - 1920) / 2 on each side.
        let projection =
            MirrorProjection::fit(size(2560, 1440), size(1920, 1200), MirrorFit::Cover).unwrap();
        assert!((projection.scale - 1200.0 / 1440.0).abs() < 1e-12);
        assert_eq!(projection.offset, Point::from((-106, 0)));
    }

    #[test]
    fn degenerate_sizes_have_no_projection() {
        assert!(
            MirrorProjection::fit(size(0, 1080), size(1920, 1080), MirrorFit::Contain).is_none()
        );
        assert!(MirrorProjection::fit(size(1920, 1080), size(1920, 0), MirrorFit::Cover).is_none());
    }

    #[test]
    fn unproject_inverts_the_fit_and_rejects_bars() {
        let projection =
            MirrorProjection::fit(size(2560, 1440), size(1920, 1200), MirrorFit::Contain).unwrap();

        assert_eq!(
            projection.unproject(Point::from((960.0, 600.0))),
            Some(Point::from((1280.0, 720.0)))
        );
        assert_eq!(projection.unproject(Point::from((960.0, 30.0))), None);
        assert_eq!(projection.unproject(Point::from((960.0, 1170.0))), None);
    }
}
