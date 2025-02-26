use defmt::*;
use embassy_rp::gpio::Output;
use embassy_rp::peripherals::SPI0;
use embassy_rp::spi::{self, Spi};
use embassy_time::Delay;
use embedded_hal_bus::spi::ExclusiveDevice;
use embedded_sdmmc::asynchronous::{
    DirEntry, Mode, SdCard, TimeSource, Timestamp, VolumeIdx, VolumeManager,
};
use heapless::Vec;
use wavv::Wav;

use crate::{CHANNEL, DATASIZE};

pub struct DummyTimesource();

impl TimeSource for DummyTimesource {
    fn get_timestamp(&self) -> Timestamp {
        Timestamp {
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
        embassy_rp::spi::Spi<'static, SPI0, embassy_rp::spi::Async>,
        Output<'static>,
        Delay,
    >,
    Delay,
>;

pub struct Library {
    volume_mgr: VolumeManager<SD, DummyTimesource, MAX_DIRS, MAX_FILES, MAX_VOLUMES>,
}

impl Library {
    pub fn new(
        sdcard: SdCard<
            ExclusiveDevice<Spi<'static, SPI0, spi::Async>, Output<'static>, Delay>,
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
        let volume0 = self.volume_mgr.open_volume(VolumeIdx(0)).await.unwrap();
        let root = volume0.open_root_dir().unwrap();
        let music = root.open_dir("music").await.unwrap();

        let file = music.open_file_in_dir(file, Mode::ReadOnly).await.unwrap();

        let wav = Wav::new(file).await.unwrap();
        info!("[Library] Wav size: {}", wav.data.end);
        action(wav).await;
    }

    pub async fn list_files(&mut self) -> Vec<DirEntry, MAX_FILES> {
        let volume0 = self.volume_mgr.open_volume(VolumeIdx(0)).await.unwrap();
        let root_dir = volume0.open_root_dir().unwrap();

        let mut files: Vec<DirEntry, MAX_FILES> = Vec::new();
        root_dir
            .iterate_dir(|file| files.push(file.clone()).unwrap())
            .await
            .unwrap();

        files
    }
}
