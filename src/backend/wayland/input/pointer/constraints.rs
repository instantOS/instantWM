//! Pointer-lock and confinement resolution for pointer motion.

use smithay::input::pointer::PointerHandle;
use smithay::reexports::wayland_server::Resource;
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

/// Protocol gate for activating a freshly created pointer constraint.
///
/// A lock is only meaningful while the constrained surface holds pointer
/// focus: Smithay delivers relative motion through its pointer focus, so
/// activating against a stale or absent focus (games create constraints
/// around fullscreen transitions) strands the lock — the client is locked
/// while receiving no pointer stream at all. niri enforces the same
/// invariant: refresh focus, then activate only if the focus belongs to the
/// constrained client and the pointer sits inside the constraint region.
///
/// When this gate declines, the pointer-motion path (`activate_under`)
/// activates the constraint on a later event once focus and hover agree.
pub(crate) fn new_constraint_should_activate(
    pointer_focus: Option<&WlSurface>,
    constraint_region: Option<&RegionAttributes>,
    pointer_location: Point<f64, Logical>,
    surface: &WlSurface,
    surface_origin: Option<Point<f64, Logical>>,
) -> bool {
    let Some(focus) = pointer_focus else {
        return false;
    };
    if !focus.id().same_client_as(&surface.id()) {
        return false;
    }
    let Some(surface_origin) = surface_origin else {
        return false;
    };
    constraint_region.is_none_or(|region| {
        region.contains((pointer_location - surface_origin.to_f64()).to_i32_round())
    })
}

#[cfg(test)]
mod tests {
    mod activation_gate {
        use super::super::new_constraint_should_activate;
        use smithay::reexports::wayland_server::Resource;
        use smithay::reexports::wayland_server::backend::ObjectId;
        use smithay::reexports::wayland_server::protocol::wl_surface::WlSurface;
        use smithay::utils::Point;

        use crate::backend::wayland::compositor::new_event_loop_and_state;

        fn null_surface(state: &crate::backend::wayland::compositor::WaylandState) -> WlSurface {
            WlSurface::from_id(&state.display_handle.clone(), ObjectId::null()).unwrap()
        }

        #[test]
        fn a_lock_without_pointer_focus_must_not_activate() {
            let (_event_loop, state) = new_event_loop_and_state();
            let surface = null_surface(&state);
            assert!(!new_constraint_should_activate(
                None,
                None,
                Point::from((0.0, 0.0)),
                &surface,
                Some(Point::from((0.0, 0.0))),
            ));
        }

        #[test]
        fn a_lock_for_another_client_must_not_activate() {
            // The null focus surface belongs to no client, so it can never
            // match the constrained surface's client.
            let (_event_loop, state) = new_event_loop_and_state();
            let surface = null_surface(&state);
            let focus = null_surface(&state);
            assert!(!new_constraint_should_activate(
                Some(&focus),
                None,
                Point::from((0.0, 0.0)),
                &surface,
                Some(Point::from((0.0, 0.0))),
            ));
        }

        #[test]
        fn a_lock_without_a_mapped_surface_origin_must_not_activate() {
            let (_event_loop, state) = new_event_loop_and_state();
            let surface = null_surface(&state);
            let focus = null_surface(&state);
            assert!(!new_constraint_should_activate(
                Some(&focus),
                None,
                Point::from((0.0, 0.0)),
                &surface,
                None,
            ));
        }
    }
}
