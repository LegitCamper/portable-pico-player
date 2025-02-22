use crate::storage::{DummyTimesource, MAX_DIRS, MAX_FILES, MAX_VOLUMES, SD};
use core::f32::consts::PI;
use defmt::*;
use embassy_rp::{
    peripherals::{PIN_14, PWM_SLICE7},
    pwm::{self, Config, Pwm, SetDutyCycle},
};
use embassy_time::{Duration, Timer};

// Play a note with correct duty cycle
pub async fn play_note<'a>(pwm: &mut Pwm<'a>, frequency: u32, duration_ms: u64) {
    if frequency == 0 {
        Timer::after(Duration::from_millis(duration_ms)).await;
        return;
    }

    let clock_freq_hz = embassy_rp::clocks::clk_sys_freq();
    let divider = 16u8;
    let period = (clock_freq_hz / (frequency * divider as u32)) as u16 - 1;

    let mut c = Config::default();
    c.top = period;
    c.divider = divider.into();

    pwm.set_config(&c);

    pwm.set_duty_cycle(period / 2); // 50% duty cycle for a square wave

    Timer::after(Duration::from_millis(duration_ms)).await;

    // Stop PWM after note duration
    pwm.set_duty_cycle(0);
    Timer::after(Duration::from_millis(50)).await; // Small pause between notes
}

#[embassy_executor::task]
pub async fn run(slice7: PWM_SLICE7, pin14: PIN_14, mut library: super::storage::Library) {
    let c = pwm::Config::default();
    let clock_freq_hz = embassy_rp::clocks::clk_sys_freq();
    let desired_freq_hz = 25_000; // 25kHz base frequency for PWM
    let divider = 16u8;
    let period = (clock_freq_hz / (desired_freq_hz * divider as u32)) as u16 - 1;

    let mut config = Config::default();
    config.top = period;
    config.divider = divider.into();

    let mut pwm = Pwm::new_output_a(slice7, pin14, c.clone());
    pwm.set_duty_cycle_fully_off().unwrap();

    library.list_files();

    loop {
        library
            .play_wav(
                "test.wav",
                async |wav: &mut wavv::Wav<
                    SD,
                    DummyTimesource,
                    MAX_DIRS,
                    MAX_FILES,
                    MAX_VOLUMES,
                >| {
                    while !wav.is_end() {
                        let sample = wav.next().unwrap();

                        match sample {
                            wavv::Data::BitDepth8(s) => {
                                info!("sample: {}", s);
                                play_note(&mut pwm, s.into(), 2).await;
                            }
                            wavv::Data::BitDepth16(s) => {
                                info!("sample: {}", s);
                                // play_note(&mut pwm, s.into(), 2).await;
                            }
                            wavv::Data::BitDepth24(s) => {
                                info!("sample: {}", s);
                                // play_note(&mut pwm, s.into(), 2).await;
                            }
                        }
                    }
                },
            )
            .await;
    }
}
