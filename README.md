# Kick trigger — NUCLEO-L476RG

Rust/Embassy firmware for one conditioned bipolar sensor input. TIM6 TRGO
clocks ADC1 at nominal 50,000 samples/s; DMA1 channel 1 transfers 12-bit
samples into a 64-word circular buffer. Each half-buffer (32 samples,
640 us) is processed in order by a separate, hardware-independent `no_std`
detector. Diagnostic builds use a larger ring as described below.
PB0 produces an active-high 3.3 V trigger.

## Wiring

Use these **Arduino header labels and connector pin numbers** on the
NUCLEO-L476RG (MB1136). Connector pin numbers are not MCU package pin numbers.

| Signal | Board label | Header pin | MCU pin |
| --- | --- | --- | --- |
| Conditioned analog input | A0 | CN8 pin 1 | PA0 / ADC1_IN5 |
| Trigger output | A3 | CN8 pin 4 | PB0 (configured as digital output) |
| Common ground | GND | CN6 pin 6 or 7 | GND |
| Front-end supply | +3.3V | CN6 pin 4 | 3.3 V |
| Fault LED / optional scope point | D13 / LD2 | CN5 pin 6 | PA5 |

Mappings: ST [UM1724, NUCLEO-L476RG Arduino connector table](https://www.st.com/resource/en/user_manual/dm00105823.pdf).
Check the silkscreen and connector pin-1 marking before connecting.

Feed A0 from a buffered, protected analog front end centered at approximately
1.65 V. Keep the signal within 0–3.3 V, including transients, and connect
the source ground to board GND. Never connect a raw piezo directly: it can
generate high-voltage positive and negative spikes. Use attenuation, input
current limiting, rail clamps, midrail bias and a suitable buffer; choose
component values for your sensor. A simple potentiometer between 3.3 V and
GND with its wiper on A0 is useful for initial static checks. Add analog
anti-alias filtering below the 25 kHz Nyquist limit for the actual sensor.

Connect A3/PB0 to a high-impedance 3.3 V-compatible receiver or scope.
Use a level shifter/driver for 5 V-only inputs or loads. Add an external
pulldown if the receiver must remain low while the MCU is in reset.
Power/program through the on-board ST-LINK USB connection; retain the
board's normal ST-LINK-to-target jumpers.

## Build, test and flash

Install stable Rust and the current probe-rs tools, then run from this directory:

```powershell
rustup target add thumbv7em-none-eabihf
cargo install probe-rs-tools --locked
cargo test -p hit-detector --locked
cargo build -p kick-trigger --release --target thumbv7em-none-eabihf --locked
cargo run -p kick-trigger --release --target thumbv7em-none-eabihf --locked
```

The runner uses `probe-rs run --chip STM32L476RGTx` and the on-board ST-LINK.
Use `probe-rs list` to diagnose probe discovery; install the ST-LINK USB
driver on Windows if needed. Press RESET to restart. No serial console is
required. Debug pauses can overrun acquisition; reset after debugging.

Direct dependencies were checked against crates.io on 2026-10-21:
embassy-stm32 0.6.0, embassy-executor 0.10.0, embassy-time 0.5.1,
cortex-m 0.7.9, cortex-m-rt 0.7.7, panic-halt 1.0.0.
Cargo.lock records the resolved versions. Use `cargo update`, then repeat
tests and the target build when updating. Review the ADC workaround below
when changing embassy-stm32.

## Optional logging

Logging is disabled by default. Enable the compile-time `logging` feature
to send compact defmt messages over ST-LINK RTT; no UART wiring is needed:

```powershell
cargo run -p kick-trigger --release --target thumbv7em-none-eabihf --features logging --locked
```

The existing probe-rs runner reads and decodes RTT using the firmware ELF.
Output appears directly in the terminal running this command, not on a COM
port. Keep the command running to receive subsequent hit/fault messages.
Plain `cargo run` does not enable the `logging` feature. The project sets
`DEFMT_LOG=info` in `.cargo/config.toml`; without it defmt defaults to errors
only and filters out startup/hit messages at compile time.
If you previously built with a different filter and see no startup output,
force a rebuild:

```powershell
$env:DEFMT_LOG = "info"
cargo clean -p kick-trigger --target thumbv7em-none-eabihf
cargo run -p kick-trigger --release --target thumbv7em-none-eabihf --features logging --locked
```

Use a current probe-rs version that supports defmt 1.x. The optional direct
dependencies are defmt 1.2.0 and defmt-rtt 1.3.1, checked against crates.io
on 2026-10-21.

Logs report startup settings, acquisition start, each accepted hit (uptime
in microseconds, raw ADC code and rectified amplitude), and a latched
acquisition overrun. Hit timestamps describe processing time, not the exact
ADC conversion instant. On a fault, `ADC_OVR=true` indicates a hardware
overrun; otherwise the DMA consumer fell behind. Output is forced low before
the fault message is emitted. Panics still use panic-halt and are not logged.

The RTT backend has `disable-blocking-mode` enabled so a disconnected or slow
reader cannot block acquisition; full buffers may drop messages. Logs add
CPU work and short critical sections, so recheck timing on the board with
logging enabled. There are no per-sample or per-block messages. Builds without
`--features logging` omit the logger dependencies, RTT buffer, messages and
defmt linker script entirely; the detector library remains independent.

### Potentiometer / detector diagnostics

For input visibility even when no hits occur, enable `detector-diagnostics`:

```powershell
cargo run -p kick-trigger --release --target thumbv7em-none-eabihf --features detector-diagnostics --locked
```

This also enables `logging`. Every sample update contributes to a summary
printed approximately four times per second (12,544 samples / 250.88 ms).
It reports the latest raw ADC code, rectified amplitude, positive one-sample
rise, armed state, remaining holdoff samples and consecutive quiet samples.
Window statistics include raw minimum/maximum and peak amplitude/rise;
these peaks may occur on different samples, so they do not imply a hit.
No logging call runs inside `Detector::update`: the detector exposes a
feature-gated snapshot without a logging dependency, and firmware aggregates
it. Disabling this feature removes the snapshot bookkeeping and aggregation.
Normal `logging` continues to report only startup, hits and faults.

The ADC ring's successful `read()` fills the entire destination half-buffer;
its returned integer counts samples still queued, not samples copied. Firmware
processes every element of the filled buffer. Using that return value as a
slice length skips samples, slows diagnostic summaries, and breaks the
sample-based holdoff/release timing.

Diagnostic builds use a 256-sample DMA ring and 128-sample processing blocks
(2.56 ms) to absorb the cost of RTT encoding. Even nonblocking defmt RTT
masks interrupts during frame encoding; the original 4 MHz CPU and 64-sample
ring could overrun immediately after printing a diagnostic summary. The CPU
now runs at 80 MHz in all modes, while the ADC sample rate stays at 50 ksample/s.
Startup logs run before acquisition begins. Diagnostic mode adds up to 2.56 ms
batching latency and pulse widths can be about 2–4.56 ms plus processing and
logging overhead. Use normal builds for final trigger timing measurements.
If `ADC_OVR=false` still occurs, the DMA consumer has fallen behind; capture
the startup settings and preceding log lines. Actual timing needs checking
on the board; a passing build cannot establish the available runtime margin.

With the potentiometer, turn to roughly midscale: `raw` should be close to
2048. Hold within 50 counts of that baseline for at least 5 ms until
`armed=true`, then turn away. Amplitude should increase while the per-sample
rise usually stays below 25, explaining why a slow turn doesn't trigger.
At either endpoint, raw should approach 0 or 4095. A pot can still generate
occasional hits through abrupt motion or contact noise. Detection thresholds
are unchanged by this diagnostic feature.

Run `cargo test -p hit-detector --features diagnostics --locked` to include
the detector snapshot/state test on the host.

## Timing and implementation

The internal 4 MHz MSI feeds the main PLL: divide by 1, multiply by 40,
then divide the R output by 2 for 80 MHz SYSCLK. No external clock or board
solder bridge changes are required. AHB, APB1 and APB2 remain undivided, so
the CPU, TIM6 and ADC kernel run at 80 MHz. This is the rated maximum for
the STM32L476; see ST's [product specifications](https://www.st.com/en/microcontrollers-microprocessors/stm32l476rg.html).
The configuration uses the chip's reset voltage scaling Range 1 (required
for 80 MHz); Embassy configures four flash wait states before switching
SYSCLK. Run on the normally powered 3.3 V Nucleo board. Higher frequency
increases power consumption. A bootloader handing off without resetting
must restore Range 1 before starting this firmware.

TIM6 PSC=0, ARR=1599 gives 20 us per trigger. MSI oscillator tolerance still
affects the absolute rate; the PLL does not improve its accuracy.
ADC uses one rank with 47.5 sample cycles plus 12.5 conversion cycles
(0.75 us at 80 MHz), comfortably within the trigger interval. Its acquisition
time is about 594 ns, slightly longer than the previous 521 ns at 24 MHz.
80 MHz ADC operation is within the Range 1 limit in the
[STM32L476 datasheet](https://www.st.com/resource/en/datasheet/stm32l476rc.pdf).
TIM2 is reserved
for Embassy's time driver. PA0 selects its ADC channel through the HAL.

Embassy STM32 0.6.0's ADC v3 ring implementation configures continuous
conversion and ignores the trigger argument. After creating the ring and
before starting it, firmware clears CONT and sets EXTSEL=13 (TIM6_TRGO),
EXTEN=1 (rising edge) through the PAC. DMA is armed before the timer starts.
See ST [RM0351, ADC external triggers](https://www.st.com/resource/en/reference_manual/rm0351-stm32l47xxx-stm32l48xxx-stm32l49xxx-and-stm32l4axxx-advanced-armbased-32bit-mcus-stmicroelectronics.pdf).

Detection never sleeps to produce a pulse: it continues draining DMA while
checking a 2 ms deadline after each block. Without diagnostics, pulse duration is approximately
2–2.64 ms plus scheduling overhead; detection adds up to 640 us batching
latency plus processing. This is a software-timed pulse, not precision PWM.
The holdoff exceeds the pulse width with the defaults. A new accepted hit
extends the deadline if tuned to allow overlapping pulses.

DMA consumer overrun or ADC hardware overrun latches LD2 on, stops TIM6,
and forces the output low until reset. Panics halt; a panic during a pulse
can leave the output high. Add a watchdog/hardware pulse limiter for systems
that require a guaranteed maximum width. No hardware validation is claimed
by a successful build.

## Detection and tuning

Edit `DETECTION` in `firmware/src/main.rs`. The detector computes
`abs(sample - baseline)` and its positive increase over the previous sample.
A hit requires both amplitude and rise thresholds on the same sample.
It starts disarmed and requires a continuous quiet interval before arming.
After a hit, it waits the complete holdoff, then requires another continuous
quiet interval; any sample above release resets that quiet counter.

| Setting | Default | Meaning |
| --- | --- | --- |
| baseline | 2048 | Fixed midrail ADC code; measure and adjust to actual idle input |
| amplitude | 180 | Minimum rectified amplitude, about 145 mV at 3.3 V |
| rise | 25 | Minimum increase over one 20 us sample, about 20 mV |
| release | 50 | Quiet band around baseline, about 40 mV |
| holdoff_samples | 1500 | 30 ms suppression after a hit |
| release_samples | 250 | 5 ms continuous quiet after holdoff |

There is no adaptive baseline or envelope filter. Drift outside the release
band prevents rearming; slow attacks may fail the rise threshold even if
large. One millisecond equals 50 samples. Keep release below amplitude,
and rise/release_samples nonzero. Tune with captured sensor traces:

1. Measure idle bias/noise; set baseline and a release band above noise.
2. Set amplitude above incidental vibration and below your weakest intended hit.
3. Lower rise for softer/slower attacks; raise it to reject gradual movement.
4. Set holdoff beyond the sensor's ringing, then adjust release time so quiet
   gaps within ringing cannot cause duplicate hits. Check fast repeated hits.

`detector/src/lib.rs` accepts raw samples without MCU dependencies. Run host
tests with `cargo test`; add captured 12-bit sample sequences as regression
tests when tuning. Calls must preserve every sample and order across blocks.

## Bench verification

1. Apply a steady midrail input; LD2 should stay off and A3 low.
2. Inject a safe, fast pulse around midrail after at least 5 ms quiet. Scope
   A0 and A3: expect one pulse, then no retrigger during ringing/holdoff.
3. Repeat with opposite polarity, slow ramps, sustained loud input and two
   hits separated by the holdoff plus release interval.
4. Confirm DMA half-buffer cadence is 640 us (2.56 ms with diagnostics) using debugger trace or temporary
   instrumentation. Check TIM6 PSC/ARR and ADC CFGR if the cadence differs.
5. Deliberately stall processing to verify the latched overrun fault and reset
   recovery. Measure pulse width and latency under your final workload.
