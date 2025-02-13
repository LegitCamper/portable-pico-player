use defmt::*;
use embassy_rp::gpio::Output;
use embassy_rp::peripherals::SPI0;
use embassy_rp::spi::{self, Spi};
use embassy_time::{Delay, Duration, Timer};
use embedded_hal_bus::spi::{ExclusiveDevice, NoDelay};
use embedded_sdmmc::sdcard::DummyCsPin;
use embedded_sdmmc::{BlockDevice, DirEntry, Directory, SdCard, Volume, VolumeManager};
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

const MAX_DIRS: usize = 10;
const MAX_FILES: usize = 10;

pub struct Library {
    volume_mgr: VolumeManager<
        SdCard<
            ExclusiveDevice<
                embassy_rp::spi::Spi<'static, SPI0, embassy_rp::spi::Blocking>,
                DummyCsPin,
                NoDelay,
            >,
            Output<'static>,
            Delay,
        >,
        DummyTimesource,
        4,
        4,
        1,
    >,
}

impl Library {
    pub fn new(
        sdcard: SdCard<
            ExclusiveDevice<Spi<'static, SPI0, spi::Blocking>, DummyCsPin, NoDelay>,
            Output<'static>,
            Delay,
        >,
    ) -> Self {
        // Now let's look for volumes (also known as partitions) on our block device.
        // To do this we need a Volume Manager. It will take ownership of the block device.
        let mut volume_mgr = embedded_sdmmc::VolumeManager::new(sdcard, DummyTimesource());

        // Try and access Volume 0 (i.e. the first partition).
        // The volume object holds information about the filesystem on that volume.
        let volume0 = volume_mgr
            .open_volume(embedded_sdmmc::VolumeIdx(0))
            .unwrap();
        info!("Volume 0: {:?}", defmt::Debug2Format(&volume0));
        drop(volume0);

        Self { volume_mgr }
    }

    pub fn list_files(&mut self) {
        let mut volume0 = self
            .volume_mgr
            .open_volume(embedded_sdmmc::VolumeIdx(0))
            .unwrap();
        // Open the root directory (mutably borrows from the volume).
        let mut root_dir = volume0.open_root_dir().unwrap();

        let prntr = |dir: &DirEntry| {
            info!(
                "Dir: {}",
                core::str::from_utf8(dir.name.base_name()).unwrap()
            )
        };

        root_dir.iterate_dir(prntr).unwrap();
    }

    pub fn print_test(&mut self) {
        let mut volume0 = self
            .volume_mgr
            .open_volume(embedded_sdmmc::VolumeIdx(0))
            .unwrap();
        // Open the root directory (mutably borrows from the volume).
        let mut root_dir = volume0.open_root_dir().unwrap();

        let mut my_file = root_dir
            .open_file_in_dir("MY_FILE.TXT", embedded_sdmmc::Mode::ReadOnly)
            .unwrap();

        while !my_file.is_eof() {
            let mut buf = [0u8; 32];
            if let Ok(n) = my_file.read(&mut buf) {
                info!("{:a}", buf[..n]);
            }
        }
    }
}
