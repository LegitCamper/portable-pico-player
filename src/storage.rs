use defmt::*;
use embassy_rp::gpio::Output;
use embassy_rp::peripherals::SPI0;
use embassy_rp::spi::{self, Spi};
use embassy_time::{Delay, Duration, Timer};
use embedded_hal_bus::spi::{ExclusiveDevice, NoDelay};
use embedded_sdmmc::SdCard;
use embedded_sdmmc::sdcard::DummyCsPin;
use {defmt_rtt as _, embassy_time as _, panic_probe as _};

struct DummyTimesource();

impl embedded_sdmmc::TimeSource for DummyTimesource {
    fn get_timestamp(&self) -> embedded_sdmmc::Timestamp {
        embedded_sdmmc::Timestamp {
            year_since_1970: 0,
            zero_indexed_month: 0,
            zero_indexed_day: 0,
            hours: 0,
            minutes: 0,
            seconds: 0,
        }
    }
}

#[embassy_executor::task]
pub async fn storage_task(
    sdcard: SdCard<
        ExclusiveDevice<Spi<'static, SPI0, spi::Blocking>, DummyCsPin, NoDelay>,
        Output<'static>,
        Delay,
    >,
) {
    // Now let's look for volumes (also known as partitions) on our block device.
    // To do this we need a Volume Manager. It will take ownership of the block device.
    let mut volume_mgr = embedded_sdmmc::VolumeManager::new(sdcard, DummyTimesource());

    // Try and access Volume 0 (i.e. the first partition).
    // The volume object holds information about the filesystem on that volume.
    let mut volume0 = volume_mgr
        .open_volume(embedded_sdmmc::VolumeIdx(0))
        .unwrap();
    info!("Volume 0: {:?}", defmt::Debug2Format(&volume0));

    // Open the root directory (mutably borrows from the volume).
    let mut root_dir = volume0.open_root_dir().unwrap();

    // Open a file called "MY_FILE.TXT" in the root directory
    // This mutably borrows the directory.
    let mut my_file = root_dir
        .open_file_in_dir("MY_FILE.TXT", embedded_sdmmc::Mode::ReadOnly)
        .unwrap();

    // Print the contents of the file
    while !my_file.is_eof() {
        let mut buf = [0u8; 32];
        if let Ok(n) = my_file.read(&mut buf) {
            info!("{:a}", buf[..n]);
        }
    }

    loop {}
}
