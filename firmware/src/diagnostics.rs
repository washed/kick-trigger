//! Aggregate every detector update, but emit only one RTT message per ~250 ms.
use heapless::Vec;
use hit_detector::{Recorder, RecorderData, Snapshot};
use postcard::to_vec;

pub struct Diagnostics {
    samples: u32,
    min: u16,
    max: u16,
    peak_amplitude: u16,
    peak_rise: u16,
    recorder: Recorder,
}

impl Diagnostics {
    pub fn new() -> Self {
        Self {
            samples: 0,
            min: u16::MAX,
            max: 0,
            peak_amplitude: 0,
            peak_rise: 0,
            recorder: Recorder::new(),
        }
    }

    pub async fn update(&mut self, raw: u16, state: Snapshot) {
        self.samples += 1;
        self.recorder.record(raw).await;
        self.min = self.min.min(raw);
        self.max = self.max.max(raw);
        self.peak_amplitude = self.peak_amplitude.max(state.amplitude);
        self.peak_rise = self.peak_rise.max(state.rise);
    }

    // Called after pulse maintenance at the end of a DMA block.
    pub fn log_if_due(&mut self, raw: u16, state: Snapshot) {
        if self.samples < 12_500 {
            return;
        }
        defmt::info!(
            "detector: raw={}, amplitude={}, rise={}, armed={}, holdoff_remaining={}, quiet_samples={}; window_samples={}, raw_min={}, raw_max={}, peak_amplitude={}, peak_rise={}",
            raw, state.amplitude, state.rise, state.armed, state.holdoff_remaining,
            state.quiet_samples, self.samples, self.min, self.max,
            self.peak_amplitude, self.peak_rise
        );
        *self = Self::new();
    }

    pub async fn log_recorder(&self) {
        let data = self.recorder.dump().await;
        let rec_data = RecorderData(&data);
        let postcard_data: Vec<u8, 10000> = to_vec(&rec_data).unwrap();
        defmt::info!("{}", postcard_data.as_slice());
    }
}
