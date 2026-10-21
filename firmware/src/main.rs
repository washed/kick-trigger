#![no_std]
#![no_main]

#[cfg(feature = "logging")]
use defmt_rtt as _;
use embassy_stm32::{
    adc::{Adc, AdcChannel, SampleTime},
    bind_interrupts, dma,
    gpio::{Level, Output, Speed},
    pac, peripherals,
    time::Hertz,
    timer::low_level::{RoundTo, Timer},
};
use embassy_time::{Duration, Instant};
use hit_detector::{Config, Detector};
use panic_halt as _;

#[cfg(feature = "detector-diagnostics")]
mod diagnostics;

bind_interrupts!(struct Irqs { DMA1_CHANNEL1 => dma::InterruptHandler<peripherals::DMA1_CH1>; });

const DETECTION: Config = Config {
    baseline: 16,
    amplitude: 1000,
    rise: 25,
    release: 750,
    holdoff_samples: 50,
    release_samples: 25,
};

// Diagnostic frames are encoded with interrupts masked. Allow more DMA
// headroom in this mode; normal builds retain their low-latency blocks.
#[cfg(feature = "detector-diagnostics")]
const DMA_SAMPLES: usize = 256;
#[cfg(not(feature = "detector-diagnostics"))]
const DMA_SAMPLES: usize = 64;

#[embassy_executor::main]
async fn main(_spawner: embassy_executor::Spawner) {
    // MSI 4 MHz / 1 * 40 / 2 = 80 MHz, the STM32L476 maximum.
    // TIM6 still triggers at 50 kHz nominally: PSC=0, ARR=1599.
    let mut config = embassy_stm32::Config::default();
    config.rcc.msi = Some(embassy_stm32::rcc::MSIRange::RANGE4M);
    config.rcc.pll = Some(embassy_stm32::rcc::Pll {
        source: embassy_stm32::rcc::PllSource::MSI,
        prediv: embassy_stm32::rcc::PllPreDiv::DIV1,
        mul: embassy_stm32::rcc::PllMul::MUL40,
        divp: None,
        divq: None,
        divr: Some(embassy_stm32::rcc::PllRDiv::DIV2),
    });
    config.rcc.sys = embassy_stm32::rcc::Sysclk::PLL1_R;
    config.rcc.mux.adcsel = pac::rcc::vals::Adcsel::SYS;
    let p = embassy_stm32::init(config);
    let mut pulse = Output::new(p.PB0, Level::Low, Speed::Low);
    let mut fault = Output::new(p.PA5, Level::Low, Speed::Low);
    #[cfg(feature = "logging")]
    defmt::info!(
        "kick-trigger: 50000 samples/s, ADC PA0/A0, pulse PB0/A3; baseline={}, amplitude={}, rise={}, release={}, holdoff={} samples, quiet={} samples",
        DETECTION.baseline, DETECTION.amplitude, DETECTION.rise, DETECTION.release,
        DETECTION.holdoff_samples, DETECTION.release_samples
    );
    let timer = Timer::new(p.TIM6);
    timer.set_frequency(Hertz(50_000), RoundTo::Slower);
    timer.generate_update_event(); // load prescaler before enabling TRGO
    timer
        .regs_basic()
        .cr2()
        .modify(|r| r.set_mms(pac::timer::vals::Mms::UPDATE));

    let adc = Adc::new(p.ADC1);
    let mut storage = [0u16; DMA_SAMPLES];
    let mut adc = adc.into_ring_buffered(
        p.DMA1_CH1,
        &mut storage,
        Irqs,
        // 47.5 / 80 MHz = 594 ns acquisition, slightly longer than the
        // previous 12.5 / 24 MHz = 521 ns for the analog front end.
        [(p.PA0.degrade_adc(), SampleTime::CYCLES47_5)].into_iter(),
        embassy_stm32::adc::CONTINUOUS,
    );
    // Embassy 0.6's ADC v3 ring API ignores its trigger argument. Override
    // only CONT/EXTSEL/EXTEN after configuration, before ADSTART.
    // RM0351: regular EXTSEL 13 = TIM6_TRGO.
    pac::ADC1.cfgr().modify(|r| {
        r.set_cont(false);
        r.set_extsel(13);
        r.set_exten(1); // rising edge
    });
    let mut samples = [0u16; DMA_SAMPLES / 2];
    let mut detector = Detector::new(DETECTION);
    #[cfg(feature = "detector-diagnostics")]
    let mut diagnostics = diagnostics::Diagnostics::new();
    let mut pulse_until: Option<Instant> = None;
    #[cfg(feature = "logging")]
    defmt::info!(
        "starting acquisition: CPU=80 MHz, DMA={} samples; pulse width 2 ms plus block scheduling",
        DMA_SAMPLES
    );
    // Finish initialization and startup logging before samples start arriving.
    adc.start();
    timer.start();
    loop {
        // read() fills the entire half-buffer. Its usize is the number of
        // samples STILL QUEUED in the DMA ring, not the number copied.
        // Discard it here so it cannot accidentally truncate processing.
        let result = adc.read(&mut samples).await.map(|_| ()).map_err(|_| ());
        let result = if pac::ADC1.isr().read().ovr() {
            Err(())
        } else {
            result
        };
        match result {
            Ok(()) => {
                for &sample in &samples {
                    if detector.update(sample) {
                        pulse.set_high();
                        pulse_until = Some(Instant::now() + Duration::from_millis(2));
                        #[cfg(feature = "logging")]
                        defmt::info!(
                            "hit: uptime_us={}, raw={}, amplitude={}",
                            Instant::now().as_micros(),
                            sample,
                            sample.abs_diff(DETECTION.baseline)
                        );
                    }
                    #[cfg(feature = "detector-diagnostics")]
                    diagnostics.update(sample, detector.snapshot());
                }
                if pulse_until.is_some_and(|deadline| Instant::now() >= deadline) {
                    pulse.set_low();
                    pulse_until = None;
                }
                #[cfg(feature = "detector-diagnostics")]
                diagnostics.log_if_due(samples[samples.len() - 1], detector.snapshot());
            }
            Err(_) => {
                // Missing samples invalidate detector history. Fail closed;
                // latch the LED and require reset to restart acquisition.
                timer.stop();
                pulse.set_low();
                fault.set_high();
                #[cfg(feature = "logging")]
                defmt::error!(
                    "acquisition overrun: ADC_OVR={}; timer stopped, output low, reset required",
                    pac::ADC1.isr().read().ovr()
                );
                loop {
                    embassy_time::Timer::after_secs(1).await;
                }
            }
        }
    }
}
