use audio_parser::AudioFile;
use embedded_hal::digital::v2::OutputPin;
use embedded_hal_async::spi::SpiBus;

use embedded_sdmmc_async::{
    BlockSpi, Controller, Directory, Mode, TimeSource, Timestamp, Volume, VolumeIdx,
};

pub struct FileReader<'a, SPI, CS, const CHUNK_LEN: usize>
where
    SPI: SpiBus<u8>,
    CS: OutputPin,
{
    sd_controller: Controller<BlockSpi<'a, SPI, CS>, DummyTimeSource>,
    pub file: Option<AudioFile<CHUNK_LEN>>,
    volume: Option<Volume>,
    dir: Option<Directory>,
    read_index: usize,
    file_name: &'static str,
}

pub struct DummyTimeSource {}
impl TimeSource for DummyTimeSource {
    fn get_timestamp(&self) -> Timestamp {
        Timestamp::from_calendar(2022, 1, 1, 0, 0, 0).unwrap()
    }
}

impl<'a, SPI, CS, const CHUNK_LEN: usize> FileReader<'a, SPI, CS, CHUNK_LEN>
where
    SPI: SpiBus<u8>,
    CS: OutputPin,
{
    pub fn new(
        sd_controller: Controller<BlockSpi<'a, SPI, CS>, DummyTimeSource>,
        file_name: &'static str,
    ) -> Self {
        Self {
            sd_controller,
            file: None,
            volume: None,
            dir: None,
            read_index: 0,
            file_name,
        }
    }

    pub async fn open(&mut self) {
        let mut volume = match self.sd_controller.get_volume(VolumeIdx(0)).await {
            Ok(volume) => volume,
            Err(e) => {
                panic!("Error getting volume: {:?}", e);
            }
        };

        let dir = self.sd_controller.open_root_dir(&volume).unwrap();
        let file = self
            .sd_controller
            .open_file_in_dir(&mut volume, &dir, self.file_name, Mode::ReadOnly)
            .await
            .unwrap();

        let audio_file = AudioFile::new_wav(file, &mut self.sd_controller, &volume)
            .await
            .unwrap();

        self.file = Some(audio_file);
        self.volume = Some(volume);
        self.dir = Some(dir);
        self.read_index = 0;
    }

    pub async fn read_exact(&mut self, buf: &mut [u8]) {
        self.file
            .as_mut()
            .unwrap()
            .read_exact(&mut self.sd_controller, &self.volume.as_ref().unwrap(), buf)
            .await;
    }

    pub fn close(self) -> Controller<BlockSpi<'a, SPI, CS>, DummyTimeSource> {
        self.sd_controller
    }
}
