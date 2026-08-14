use super::*;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum HotCornerState {
    #[default]
    Ready,
    Latched {
        monitor_id: MonitorId,
    },
}

impl HotCornerState {
    /// Update the hot-corner latch. Returns the monitor whose corner was
    /// entered when the caller should toggle the edge scratchpad.
    pub fn update(
        &mut self,
        monitor_id: Option<MonitorId>,
        inside_activation_zone: bool,
        inside_keep_zone: bool,
    ) -> Option<MonitorId> {
        match *self {
            Self::Ready => {
                let monitor_id = monitor_id.filter(|_| inside_activation_zone)?;
                *self = Self::Latched { monitor_id };
                Some(monitor_id)
            }
            Self::Latched {
                monitor_id: latched_monitor,
            } if monitor_id == Some(latched_monitor) && inside_keep_zone => None,
            Self::Latched { .. } => {
                // Rearm only. Requiring a subsequent sample before activation
                // prevents a single jump between monitor corners from firing
                // two actions at once.
                *self = Self::Ready;
                None
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn fires_once_until_pointer_leaves_keep_zone() {
        let monitor = MonitorId::from_raw(7);
        let mut state = HotCornerState::default();

        assert_eq!(state.update(Some(monitor), true, true), Some(monitor));
        assert_eq!(state.update(Some(monitor), true, true), None);
        assert_eq!(state.update(Some(monitor), false, true), None);
        assert_eq!(state.update(Some(monitor), false, false), None);
        assert_eq!(state.update(Some(monitor), true, true), Some(monitor));
    }

    #[test]
    fn rearms_without_firing_when_pointer_jumps_between_monitors() {
        let first = MonitorId::from_raw(1);
        let second = MonitorId::from_raw(2);
        let mut state = HotCornerState::default();

        assert_eq!(state.update(Some(first), true, true), Some(first));
        assert_eq!(state.update(Some(second), true, true), None);
        assert_eq!(state.update(Some(second), true, true), Some(second));
    }
}
