use std::{
    sync::{Arc, Mutex},
    time::{Duration, SystemTime},
};

pub(super) const IDLE_TIMEOUT: Duration = Duration::from_secs(3600);

#[derive(Clone)]
pub(crate) struct Activity(Arc<Mutex<Clock>>);

struct Clock {
    latest: SystemTime,
    armed: bool,
    elapsed: bool,
}

impl Default for Activity {
    fn default() -> Self {
        Self(Arc::new(Mutex::new(Clock {
            latest: SystemTime::now(),
            armed: false,
            elapsed: false,
        })))
    }
}

impl Activity {
    #[cfg(feature = "remote-probe")]
    pub(crate) fn probe_elapsed_hour(&self) -> Result<(), String> {
        let mut clock = self.0.lock().map_err(|_| "Activity unavailable.")?;
        if !clock.armed {
            return Err("Remote activity is not armed.".into());
        }
        clock.latest = SystemTime::now() - IDLE_TIMEOUT - Duration::from_secs(1);
        Ok(())
    }
    pub(crate) fn record(&self) {
        self.record_at(SystemTime::now());
    }
    fn record_at(&self, now: SystemTime) {
        if let Ok(mut clock) = self.0.lock() {
            // A wake gesture must not erase an already elapsed idle deadline.
            clock.elapsed |= clock.armed && idle_since(clock.latest, now);
            clock.latest = now;
        }
    }
    pub(super) fn activate(&self) {
        self.activate_at(SystemTime::now());
    }
    fn activate_at(&self, now: SystemTime) {
        if let Ok(mut clock) = self.0.lock() {
            clock.latest = now;
            clock.armed = true;
            clock.elapsed = false;
        }
    }
    pub(super) fn restore(&self) {
        self.restore_at(SystemTime::now());
    }
    fn restore_at(&self, now: SystemTime) {
        if let Ok(mut clock) = self.0.lock() {
            // Background authorization recovery is not user activity.
            if !clock.armed {
                clock.latest = now;
                clock.armed = true;
            }
        }
    }
    pub(super) fn deactivate(&self) {
        if let Ok(mut clock) = self.0.lock() {
            clock.armed = false;
            clock.elapsed = false;
        }
    }

    pub(super) fn is_idle(&self, now: SystemTime) -> bool {
        self.0
            .lock()
            .is_ok_and(|clock| clock.armed && (clock.elapsed || idle_since(clock.latest, now)))
    }
}

fn idle_since(latest: SystemTime, now: SystemTime) -> bool {
    now.duration_since(latest)
        .is_ok_and(|idle| idle > IDLE_TIMEOUT)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn any_terminal_or_window_activity_restarts_the_global_hour() {
        let start = SystemTime::now();
        let activity = Activity::default();
        activity.activate_at(start);
        let other_window = activity.clone();
        assert!(!activity.is_idle(start + IDLE_TIMEOUT));
        assert!(activity.is_idle(start + IDLE_TIMEOUT + Duration::from_secs(1)));
        other_window.record_at(start + Duration::from_secs(3500));
        assert!(!activity.is_idle(start + Duration::from_secs(7100)));
        assert!(activity.is_idle(start + Duration::from_secs(7101)));
        assert!(!activity.is_idle(start - Duration::from_secs(1)));
    }

    #[test]
    fn first_wake_activity_preserves_elapsed_idle_until_explicit_resume() {
        let start = SystemTime::now();
        let activity = Activity::default();
        assert!(!activity.is_idle(start + IDLE_TIMEOUT + Duration::from_secs(1)));
        activity.activate_at(start);
        let wake = start + IDLE_TIMEOUT + Duration::from_secs(1);
        activity.record_at(wake);
        assert!(activity.is_idle(wake));
        activity.record_at(wake + Duration::from_secs(1));
        assert!(activity.is_idle(wake + Duration::from_secs(1)));
        activity.activate_at(wake + Duration::from_secs(2));
        assert!(!activity.is_idle(wake + Duration::from_secs(2)));
        activity.deactivate();
        activity.record_at(wake + IDLE_TIMEOUT + Duration::from_secs(3));
        assert!(!activity.is_idle(wake + IDLE_TIMEOUT + Duration::from_secs(3)));
    }

    #[test]
    fn automatic_authorization_restore_preserves_the_original_idle_deadline() {
        let start = SystemTime::now();
        let activity = Activity::default();
        activity.restore_at(start);
        activity.restore_at(start + Duration::from_secs(3000));
        assert!(!activity.is_idle(start + IDLE_TIMEOUT));
        assert!(activity.is_idle(start + IDLE_TIMEOUT + Duration::from_secs(1)));
        activity.record_at(start + IDLE_TIMEOUT + Duration::from_secs(1));
        activity.restore_at(start + Duration::from_secs(4000));
        assert!(activity.is_idle(start + Duration::from_secs(4000)));
    }
}
