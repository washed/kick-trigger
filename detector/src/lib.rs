#![no_std]

/// All thresholds use absolute ADC counts relative to `baseline`.
#[derive(Clone, Copy, Debug)]
pub struct Config {
    pub baseline: u16,
    pub amplitude: u16,
    /// Minimum increase in rectified amplitude over one sample (20 us).
    pub rise: u16,
    pub release: u16,
    pub holdoff_samples: u32,
    pub release_samples: u32,
}

pub struct Detector {
    config: Config,
    previous: u16,
    holdoff: u32,
    quiet: u32,
    armed: bool,
    #[cfg(feature = "diagnostics")]
    last_rise: u16,
}

/// State after the most recent update. No logging backend is needed.
#[cfg(feature = "diagnostics")]
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Snapshot {
    pub amplitude: u16,
    pub rise: u16,
    pub armed: bool,
    pub holdoff_remaining: u32,
    pub quiet_samples: u32,
}

impl Detector {
    pub fn new(config: Config) -> Self {
        assert!(config.release < config.amplitude);
        assert!(config.release_samples > 0);
        assert!(config.amplitude > 0 && config.rise > 0);
        Self {
            config,
            previous: 0,
            holdoff: 0,
            quiet: 0,
            armed: false,
            #[cfg(feature = "diagnostics")]
            last_rise: 0,
        }
    }

    /// Feed every sample in order. Startup also requires a quiet release period.
    /// After a hit, holdoff completes before the release counter starts.
    pub fn update(&mut self, sample: u16) -> bool {
        let amplitude = sample.abs_diff(self.config.baseline);
        let rise = amplitude.saturating_sub(self.previous);
        #[cfg(feature = "diagnostics")]
        {
            self.last_rise = rise;
        }
        self.previous = amplitude;
        if !self.armed {
            if self.holdoff > 0 {
                self.holdoff -= 1;
                self.quiet = 0;
            } else if amplitude <= self.config.release {
                self.quiet = self.quiet.saturating_add(1);
                if self.quiet >= self.config.release_samples {
                    self.armed = true;
                }
            } else {
                self.quiet = 0;
            }
            return false;
        }
        if amplitude >= self.config.amplitude && rise >= self.config.rise {
            self.armed = false;
            self.holdoff = self.config.holdoff_samples;
            self.quiet = 0;
            return true;
        }
        false
    }

    #[cfg(feature = "diagnostics")]
    pub fn snapshot(&self) -> Snapshot {
        Snapshot {
            amplitude: self.previous,
            rise: self.last_rise,
            armed: self.armed,
            holdoff_remaining: self.holdoff,
            quiet_samples: self.quiet,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    fn detector() -> Detector {
        Detector::new(Config {
            baseline: 2048,
            amplitude: 100,
            rise: 30,
            release: 20,
            holdoff_samples: 3,
            release_samples: 2,
        })
    }
    fn arm(d: &mut Detector) {
        assert!(!d.update(2048));
        assert!(!d.update(2048));
    }
    #[test]
    fn positive_and_negative_hits() {
        for sample in [2200, 1896] {
            let mut d = detector();
            arm(&mut d);
            assert!(d.update(sample));
        }
    }
    #[test]
    fn rejects_noise_and_slow_rise() {
        let mut d = detector();
        arm(&mut d);
        for sample in (2048..2300).step_by(10) {
            assert!(!d.update(sample));
        }
    }
    #[test]
    fn requires_holdoff_then_continuous_release() {
        let mut d = detector();
        arm(&mut d);
        assert!(d.update(2200));
        for _ in 0..3 {
            assert!(!d.update(2048));
        }
        assert!(!d.update(2048));
        assert!(!d.update(2200)); // interrupts release, cannot retrigger
        arm(&mut d);
        assert!(d.update(2200));
    }
    #[test]
    fn startup_loud_signal_cannot_trigger() {
        let mut d = detector();
        for _ in 0..100 {
            assert!(!d.update(4095));
        }
        arm(&mut d);
        assert!(d.update(0));
    }

    #[cfg(feature = "diagnostics")]
    #[test]
    fn snapshot_tracks_updates_and_rearming() {
        let mut d = detector();
        assert_eq!(d.snapshot().holdoff_remaining, 0);
        assert!(!d.snapshot().armed);
        arm(&mut d);
        assert!(d.snapshot().armed);
        assert!(d.update(2200));
        assert_eq!(
            d.snapshot(),
            Snapshot {
                amplitude: 152,
                rise: 152,
                armed: false,
                holdoff_remaining: 3,
                quiet_samples: 0
            }
        );
        for _ in 0..3 {
            d.update(2048);
        }
        assert_eq!(d.snapshot().holdoff_remaining, 0);
        assert_eq!(d.snapshot().rise, 0);
        d.update(2048);
        assert_eq!(d.snapshot().quiet_samples, 1);
        assert!(!d.snapshot().armed);
        d.update(2048);
        assert!(d.snapshot().armed);
    }
}
