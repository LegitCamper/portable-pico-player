use crate::storage::{DummyTimesource, MAX_DIRS, MAX_FILES, MAX_VOLUMES, SD};
use defmt::*;
use embassy_rp::{
    i2c::{self, I2c},
    peripherals::I2C1,
};
use embassy_time::{Duration, Instant, Timer};
use mcp4725::{MCP4725, PowerDown};
use wavv::DataBulk;

#[embassy_executor::task]
pub async fn run(i2c1: I2c<'static, I2C1, i2c::Async>, mut library: super::storage::Library) {
    library.list_files();

    let mut dac = MCP4725::new(i2c1, 0b010);

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
                    info!(
                        "File Info:\nsample_rate: {}, num_channels: {}, bit_depth: {}",
                        wav.fmt.sample_rate, wav.fmt.num_channels, wav.fmt.bit_depth
                    );

                    let hz_duration = Duration::from_hz(wav.fmt.sample_rate.into());

                    while !wav.is_end() {
                        let samples = wav.next_n::<1_500>().unwrap();

                        info!("file: {}/{}", wav.file.offset(), wav.file.length());

                        if let DataBulk::BitDepth8(samples) = samples {
                            for sample in samples {
                                let now = Instant::now();
                                dac.set_dac_fast(PowerDown::Normal, sample.into()).ok();

                                let since = Instant::now().duration_since(now);
                                if since > hz_duration {
                                    Timer::after(since - hz_duration).await;
                                } else {
                                    Timer::after(hz_duration - since).await;
                                }
                            }
                        }
                    }
                },
            )
            .await;

        info!("Playing again");
    }
}
