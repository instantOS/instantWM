//! Pointer-lock and confinement resolution for pointer motion.

use smithay::input::pointer::PointerHandle;
use smithay::reexports::wayland_server::protocol::wl_surface::WlSurface;
use smithay::utils::{Logical, Point};
use smithay::wayland::compositor::RegionAttributes;
use smithay::wayland::pointer_constraints::{PointerConstraint, with_pointer_constraint};

use crate::backend::wayland::compositor::WaylandState;
use crate::backend::wayland::compositor::window::hit_test::SurfaceFocus;

#[derive(Default)]
pub(super) enum ActivePointerConstraint {
    #[default]
    None,
    Locked,
    Confined {
        surface: WlSurface,
        surface_loc: Point<i32, Logical>,
        region: Option<RegionAttributes>,
    },
}

impl ActivePointerConstraint {
    pub(super) fn under(
        pointer: &PointerHandle<WaylandState>,
        current_surface: Option<&SurfaceFocus>,
        pointer_location: Point<f64, Logical>,
    ) -> Self {
        let Some((surface, surface_loc)) = current_surface else {
            return Self::default();
        };
        let mut resolved = Self::None;
        with_pointer_constraint(surface, pointer, |constraint| {
            let Some(constraint) = constraint else {
                return;
            };
            if !constraint.is_active()
                || !pointer_is_in_region(&constraint, pointer_location, *surface_loc)
            {
                return;
            }

            match &*constraint {
                PointerConstraint::Locked(_) => {
                    resolved = Self::Locked;
                }
                PointerConstraint::Confined(confine) => {
                    resolved = Self::Confined {
                        surface: surface.clone(),
                        surface_loc: *surface_loc,
                        region: confine.region().cloned(),
                    };
                }
            }
        });
        resolved
    }

    pub(super) fn is_locked(&self) -> bool {
        matches!(self, Self::Locked)
    }

    /// Whether a candidate motion remains inside an active confinement.
    pub(super) fn allows_motion_to(
        &self,
        candidate_surface: Option<&SurfaceFocus>,
        candidate_location: Point<f64, Logical>,
    ) -> bool {
        let Self::Confined {
            surface: confined_surface,
            surface_loc,
            region,
        } = self
        else {
            return true;
        };
        if candidate_surface.is_none_or(|(surface, _)| surface != confined_surface) {
            return false;
        }
        region.as_ref().is_none_or(|region| {
            region.contains((candidate_location - surface_loc.to_f64()).to_i32_round())
        })
    }
}

pub(super) fn activate_under(
    pointer: &PointerHandle<WaylandState>,
    current_surface: Option<&SurfaceFocus>,
    pointer_location: Point<f64, Logical>,
) {
    let Some((surface, surface_loc)) = current_surface else {
        return;
    };
    with_pointer_constraint(surface, pointer, |constraint| {
        let Some(constraint) = constraint else {
            return;
        };
        if !constraint.is_active()
            && pointer_is_in_region(&constraint, pointer_location, *surface_loc)
        {
            constraint.activate();
        }
    });
}

fn pointer_is_in_region(
    constraint: &PointerConstraint,
    pointer_location: Point<f64, Logical>,
    surface_loc: Point<i32, Logical>,
) -> bool {
    constraint.region().is_none_or(|region| {
        region.contains((pointer_location - surface_loc.to_f64()).to_i32_round())
    })
}
