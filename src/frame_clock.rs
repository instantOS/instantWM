use std::num::NonZeroU64;
use std::time::Duration;

/// Frame clock for tracking presentation times and scheduling renders.
///
/// This tracks the display's refresh interval and the last presentation time
/// to calculate the optimal time to render the next frame, reducing latency
/// by rendering closer to the actual presentation deadline.
#[derive(Debug)]
pub struct FrameClock {
    last_presentation_time: Option<Duration>,
    refresh_interval_ns: Option<NonZeroU64>,
}

impl FrameClock {
    pub fn new(refresh_interval: Option<Duration>) -> Self {
        let refresh_interval_ns = if let Some(interval) = &refresh_interval {
            // Sub-second interval expected
            if interval.as_secs() > 0 {
                None
            } else {
                NonZeroU64::new(interval.subsec_nanos().into())
            }
        } else {
            None
        };

        Self {
            last_presentation_time: None,
            refresh_interval_ns,
        }
    }

    pub fn refresh_interval(&self) -> Option<Duration> {
        self.refresh_interval_ns
            .map(|r| Duration::from_nanos(r.get()))
    }

    pub fn presented(&mut self, presentation_time: Duration) {
        if presentation_time.is_zero() {
            return;
        }
        self.last_presentation_time = Some(presentation_time);
    }

    pub fn next_presentation_time(&self) -> Duration {
        let now = monotonic_time();

        let Some(refresh_interval_ns) = self.refresh_interval_ns else {
            return now;
        };
        let Some(last_presentation_time) = self.last_presentation_time else {
            return now;
        };

        let refresh_interval_ns = refresh_interval_ns.get();

        if now <= last_presentation_time {
            // Got an early VBlank
            let mut next = now + Duration::from_nanos(refresh_interval_ns);
            if next < last_presentation_time {
                next = last_presentation_time + Duration::from_nanos(refresh_interval_ns);
            }
            return next;
        }

        let since_last = now - last_presentation_time;
        let since_last_ns =
            since_last.as_secs() * 1_000_000_000 + u64::from(since_last.subsec_nanos());
        let to_next_ns = (since_last_ns / refresh_interval_ns + 1) * refresh_interval_ns;

        last_presentation_time + Duration::from_nanos(to_next_ns)
    }
}

pub fn monotonic_time() -> Duration {
    let mut ts = libc::timespec {
        tv_sec: 0,
        tv_nsec: 0,
    };
    // SAFETY: CLOCK_MONOTONIC is valid, &mut ts is valid pointer
    unsafe {
        libc::clock_gettime(libc::CLOCK_MONOTONIC, &mut ts);
    }
    Duration::new(ts.tv_sec as u64, ts.tv_nsec as u32)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn no_refresh_interval_returns_now() {
        let clock = FrameClock::new(None);
        let now = monotonic_time();
        let next = clock.next_presentation_time();
        assert!(next >= now);
    }

    #[test]
    fn no_presentation_time_returns_now() {
        let clock = FrameClock::new(Some(Duration::from_micros(16667)));
        let now = monotonic_time();
        let next = clock.next_presentation_time();
        assert!(next >= now);
    }

    #[test]
    fn calculates_next_presentation() {
        let mut clock = FrameClock::new(Some(Duration::from_micros(16667)));
        let base = monotonic_time();
        clock.presented(base);

        let next = clock.next_presentation_time();
        assert!(next > base);
        // Should be within one refresh interval of base
        assert!(next - base <= Duration::from_micros(16667 * 2));
    }
}
