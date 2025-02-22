use crate::storage::{DummyTimesource, MAX_DIRS, MAX_FILES, MAX_VOLUMES, SD};
use defmt::*;
use embassy_rp::{
    peripherals::{PIN_14, PWM_SLICE7},
    pwm::{self, Config, Pwm, SetDutyCycle},
};
use embassy_time::{Duration, Timer};
use wavv::Data;

// Converts PCM audio data to PWM format
fn pcm_to_pwm(data: &Data, duty_cycle_min: u16, duty_cycle_max: u16) -> u16 {
    match data {
        Data::BitDepth8(sample) => {
            if 0x80 > *sample {
                return duty_cycle_min;
            }

            ((*sample as u32 - 0x80) * (duty_cycle_max - duty_cycle_min) as u32 + 0x8000) as u16
                / 0x80
        }
        Data::BitDepth16(sample) => {
            ((*sample as i32 - 0x8000) * (duty_cycle_max - duty_cycle_min) as i32 + 0x8000) as u16
                / 0x8000
        }
        Data::BitDepth24(sample) => {
            ((*sample as i32 - 0x8000) * (duty_cycle_max - duty_cycle_min) as i32 + 0x8000) as u16
                / 0x8000
        }
    }
}

#[embassy_executor::task]
pub async fn run(slice7: PWM_SLICE7, pin14: PIN_14, mut library: super::storage::Library) {
    let mut c = pwm::Config::default();
    let clock_freq_hz = embassy_rp::clocks::clk_sys_freq();
    let desired_freq_hz = 25_000; // 25kHz base frequency for PWM
    let divider = 16u8;
    let period = (clock_freq_hz / (desired_freq_hz * divider as u32)) as u16 - 1;

    c.top = period;
    c.divider = divider.into();

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
                        pwm.set_duty_cycle(pcm_to_pwm(&sample, c.top / 4, c.top));
                    }
                },
            )
            .await;

        info!("Playing again");
    }
}
