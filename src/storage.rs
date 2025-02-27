use defmt::*;
use embassy_rp::gpio::Output;
use embassy_rp::peripherals::SPI0;
use embassy_rp::spi::{self, Spi};
use embassy_sync::blocking_mutex::raw::RawMutex;
use embassy_sync::mutex::Mutex;
use embassy_time::Delay;
use embedded_hal_bus::spi::{ExclusiveDevice, NoDelay};
use embedded_sdmmc::sdcard::DummyCsPin;
use embedded_sdmmc::{
    DirEntry, Directory, File, Mode, SdCard, TimeSource, Timestamp, Volume, VolumeIdx,
    VolumeManager,
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
    ExclusiveDevice<spi::Spi<'static, SPI0, spi::Blocking>, DummyCsPin, NoDelay>,
    Output<'static>,
    Delay,
>;

// pub struct Library<'a> {
//     directory: Directory<'a, SD, DummyTimesource, MAX_DIRS, MAX_FILES, MAX_VOLUMES>,
// }

// impl<'a> Library<'a> {
//     pub fn new(dir: Directory<'a, SD, DummyTimesource, MAX_DIRS, MAX_FILES, MAX_VOLUMES>) -> Self {
//         Self { directory: dir }
//     }

//     pub fn list_files(&mut self) -> Vec<DirEntry, MAX_FILES> {
//         let mut files: Vec<DirEntry, MAX_FILES> = Vec::new();
//         self.directory
//             .iterate_dir(|file| files.push(file.clone()).unwrap())
//             .unwrap();

//         files
//     }
// }

// pub fn read_wav<'a>(
//     mut dir: Directory<'a, SD, DummyTimesource, MAX_DIRS, MAX_FILES, MAX_VOLUMES>,
//     file: &str,
// ) -> Wav<'a, SD, DummyTimesource, MAX_DIRS, MAX_FILES, MAX_VOLUMES> {
//     let file = dir.open_file_in_dir(file, Mode::ReadOnly).unwrap();

//     let wav = Wav::new(file).unwrap();
//     info!("[Library] Wav size: {}", wav.data.end);

//     wav
// }
