use defmt::*;
use embassy_rp::gpio::Output;
use embassy_rp::peripherals::SPI0;
use embassy_rp::spi::{self, Spi};
use embassy_time::Delay;
use embedded_hal_bus::spi::{ExclusiveDevice, NoDelay};
use embedded_sdmmc::sdcard::DummyCsPin;
use embedded_sdmmc::{DirEntry, SdCard, VolumeManager};
use heapless::Vec;
use wavv::Wav;

use crate::{CHANNEL, DATASIZE};

pub struct DummyTimesource();

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

pub const MAX_DIRS: usize = 4;
pub const MAX_FILES: usize = 4;
pub const MAX_VOLUMES: usize = 1;

pub type SD = SdCard<
    ExclusiveDevice<
        embassy_rp::spi::Spi<'static, SPI0, embassy_rp::spi::Blocking>,
        DummyCsPin,
        NoDelay,
    >,
    Output<'static>,
    Delay,
>;

pub struct Library {
    volume_mgr: VolumeManager<SD, DummyTimesource, MAX_DIRS, MAX_FILES, MAX_VOLUMES>,
}

impl Library {
    pub fn new(
        sdcard: SdCard<
            ExclusiveDevice<Spi<'static, SPI0, spi::Blocking>, DummyCsPin, NoDelay>,
            Output<'static>,
            Delay,
        >,
    ) -> Self {
        Self {
            volume_mgr: VolumeManager::new(sdcard, DummyTimesource()),
        }
    }

    pub async fn read_wav(
        &mut self,
        file: &str,
        mut action: impl async FnMut(Wav<SD, DummyTimesource, MAX_DIRS, MAX_FILES, MAX_VOLUMES>),
    ) {
        let mut volume0 = self
            .volume_mgr
            .open_volume(embedded_sdmmc::VolumeIdx(0))
            .unwrap();
        // Open the root directory (mutably borrows from the volume).
        let mut root = volume0.open_root_dir().unwrap();

        let mut music = root.open_dir("music").unwrap();

        let file = music
            .open_file_in_dir(file, embedded_sdmmc::Mode::ReadOnly)
            .unwrap();

        let wav = Wav::new(file).unwrap();
        info!("[Library] Wav size: {}", wav.data.end);
        action(wav).await;
    }

    pub fn list_files(&mut self) -> Vec<DirEntry, MAX_FILES> {
        let mut volume0 = self
            .volume_mgr
            .open_volume(embedded_sdmmc::VolumeIdx(0))
            .unwrap();
        // Open the root directory (mutably borrows from the volume).
        let mut root_dir = volume0.open_root_dir().unwrap();

        let mut files: Vec<DirEntry, MAX_FILES> = Vec::new();
        root_dir
            .iterate_dir(|file| files.push(file.clone()).unwrap())
            .unwrap();

        files
    }
}
