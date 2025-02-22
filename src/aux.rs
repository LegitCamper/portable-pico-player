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

// Define the note frequencies (in Hz)
const C4: u32 = 261;
const G4: u32 = 392;
const A4: u32 = 440;
const F4: u32 = 349;
const E4: u32 = 330;
const D4: u32 = 294;

const NOTE_DURATION_MS: u64 = 500; // Duration of each note in milliseconds

// Twinkle Twinkle melody sequence
const MELODY: [(u32, u64); 14] = [
    (C4, NOTE_DURATION_MS),
    (C4, NOTE_DURATION_MS),
    (G4, NOTE_DURATION_MS),
    (G4, NOTE_DURATION_MS),
    (A4, NOTE_DURATION_MS),
    (A4, NOTE_DURATION_MS),
    (G4, NOTE_DURATION_MS * 2),
    (F4, NOTE_DURATION_MS),
    (F4, NOTE_DURATION_MS),
    (E4, NOTE_DURATION_MS),
    (E4, NOTE_DURATION_MS),
    (D4, NOTE_DURATION_MS),
    (D4, NOTE_DURATION_MS),
    (C4, NOTE_DURATION_MS * 2),
];

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

    // loop {
    //     for &(freq, duration) in MELODY.iter() {
    //         play_note(&mut pwm, freq, duration).await;
    //     }
    //     Timer::after_millis(500).await;
    // }

    library.list_files();

    loop {
        library
            .play_wav(
                "test.wav",
                async |buf: &mut wavv::Wav<
                    SD,
                    DummyTimesource,
                    MAX_DIRS,
                    MAX_FILES,
                    MAX_VOLUMES,
                >| {
                    play_note(&mut pwm, 2, 2).await;
                },
            )
            .await;
    }
}
