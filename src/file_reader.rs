use audio_parser::AudioFile;
use defmt::info;
use embedded_hal::digital::v2::OutputPin;
use embedded_hal_async::spi::SpiBus;

use embedded_sdmmc_async::{
    BlockSpi, Controller, Directory, Mode, TimeSource, Timestamp, Volume, VolumeIdx,
};

const SD_CARD_CHUNK_LEN: usize = 512;

pub struct FileReader<'a, SPI, CS>
where
    SPI: SpiBus<u8>,
    CS: OutputPin,
{
    file_buffer: [u8; SD_CARD_CHUNK_LEN],
    sd_controller: Controller<BlockSpi<'a, SPI, CS>, DummyTimeSource>,
    file: Option<AudioFile>,
    volume: Option<Volume>,
    dir: Option<Directory>,
    read_index: usize,
    file_name: &'static str,
}

struct DummyTimeSource {}
impl TimeSource for DummyTimeSource {
    fn get_timestamp(&self) -> Timestamp {
        Timestamp::from_calendar(2022, 1, 1, 0, 0, 0).unwrap()
    }
}

impl<'a, SPI, CS> FileReader<'a, SPI, CS>
where
    SPI: SpiBus<u8>,
    CS: OutputPin,
{
    pub fn new(block_device: BlockSpi<'a, SPI, CS>, file_name: &'static str) -> Self {
        let timesource = DummyTimeSource {};
        let sd_controller = Controller::new(block_device, timesource);
        let file_buffer = [0u8; SD_CARD_CHUNK_LEN];

        Self {
            file_buffer,
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

        let mut audio_file = AudioFile::new_wav(file, &mut self.sd_controller, &volume)
            .await
            .unwrap();

        audio_file
            .read_exact(&mut self.sd_controller, &volume, &mut self.file_buffer)
            .await;

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

    // fn close(&mut self) {
    //     if let Some(file) = self.file.take() {
    //         self.sd_controller
    //             .close_file(self.volume.as_ref().unwrap(), file)
    //             .unwrap();
    //     }

    //     if let Some(dir) = self.dir.take() {
    //         self.sd_controller
    //             .close_dir(self.volume.as_ref().unwrap(), dir);
    //     }
    // }
}
