use audio_parser::AudioFile;
use defmt::Format;
use embedded_hal::digital::v2::OutputPin;
use embedded_hal_async::spi::SpiBus;
use embedded_sdmmc_async::{
    BlockSpi, Controller, Directory, Mode, TimeSource, Timestamp, Volume, VolumeIdx,
};
use heapless::{String, Vec};

#[derive(Debug, Format)]
pub struct Album<const MAX_FILES: usize> {
    songs: Vec<String<11>, MAX_FILES>,
}

#[derive(Debug, Format)]
pub struct Artist<const MAX_DIRS: usize, const MAX_FILES: usize> {
    albums: Vec<Album<MAX_FILES>, MAX_DIRS>,
}

pub struct DummyTimeSource {}
impl TimeSource for DummyTimeSource {
    fn get_timestamp(&self) -> Timestamp {
        Timestamp::from_calendar(2022, 1, 1, 0, 0, 0).unwrap()
    }
}

pub struct Library<
    'a,
    SPI,
    CS,
    const MAX_DIRS: usize,
    const MAX_FILES: usize,
    const CHUNK_LEN: usize,
> where
    SPI: SpiBus<u8>,
    CS: OutputPin,
{
    sd_controller: Controller<BlockSpi<'a, SPI, CS>, DummyTimeSource>,
    artists: Option<Vec<Artist<MAX_DIRS, MAX_FILES>, MAX_DIRS>>,
    file: Option<AudioFile<CHUNK_LEN>>,
    volume: Option<Volume>,
    dir: Option<Directory>,
    read_index: usize,
}

impl<'a, SPI, CS, const MAX_DIRS: usize, const MAX_FILES: usize, const CHUNK_LEN: usize>
    Library<'a, SPI, CS, MAX_DIRS, MAX_FILES, CHUNK_LEN>
where
    SPI: SpiBus<u8>,
    CS: OutputPin,
{
    pub fn new(sd_controller: Controller<BlockSpi<'a, SPI, CS>, DummyTimeSource>) -> Self {
        Self {
            sd_controller,
            artists: None,
            file: None,
            volume: None,
            dir: None,
            read_index: 0,
        }
    }

    pub async fn discover_music(&mut self) {
        let volume = match self.sd_controller.get_volume(VolumeIdx(0)).await {
            Ok(volume) => volume,
            Err(e) => {
                panic!("Error getting volume: {:?}", e);
            }
        };

        let root_dir = self.sd_controller.open_root_dir(&volume).unwrap();

        let mut artist_names: Vec<String<11>, MAX_FILES> = Vec::new();
        self.sd_controller
            .iterate_dir(&volume, &root_dir, |dir| {
                artist_names
                    .push(
                        String::from_utf8(Vec::from_slice(dir.name.base_name()).unwrap()).unwrap(),
                    )
                    .unwrap()
            })
            .await
            .unwrap();

        let mut artists = Vec::new();

        for artist in artist_names {
            let artist_dir = self
                .sd_controller
                .open_dir(&volume, &root_dir, &artist)
                .await
                .unwrap();

            artists
                .push(
                    get_artist(
                        &artist_dir,
                        &mut self.sd_controller,
                        &volume,
                        artist.as_str(),
                    )
                    .await,
                )
                .unwrap()
        }

        self.sd_controller.close_dir(&volume, root_dir);

        self.artists = Some(artists)
    }

    pub fn artists(&self) -> Option<&Vec<Artist<MAX_DIRS, MAX_FILES>, MAX_DIRS>> {
        self.artists.as_ref()
    }

    pub async fn open(&mut self, file_name: &'a str) {
        let mut volume = match self.sd_controller.get_volume(VolumeIdx(0)).await {
            Ok(volume) => volume,
            Err(e) => {
                panic!("Error getting volume: {:?}", e);
            }
        };

        let dir = self.sd_controller.open_root_dir(&volume).unwrap();
        let u_dir = self
            .sd_controller
            .open_dir(&volume, &dir, "Unknown")
            .await
            .unwrap();
        let file = self
            .sd_controller
            .open_file_in_dir(&mut volume, &u_dir, file_name, Mode::ReadOnly)
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

    pub fn end(&self) -> usize {
        self.file.as_ref().unwrap().end
    }

    pub fn read(&self) -> usize {
        self.file.as_ref().unwrap().read
    }

    pub fn start(&self) -> usize {
        self.file.as_ref().unwrap().start
    }

    pub fn sample_rate(&self) -> u32 {
        self.file.as_ref().unwrap().sample_rate
    }

    pub fn bit_depth(&self) -> u16 {
        self.file.as_ref().unwrap().bit_depth
    }

    pub fn channels(&self) -> u16 {
        self.file.as_ref().unwrap().num_channels
    }

    pub fn close(&mut self) {
        // if let Some(audio_file) = self.file.as_mut() {
        //     self.sd_controller
        //         .close_file(self.volume.as_ref().unwrap(), audio_file.destroy())
        //         .unwrap();
        // };
        self.file = None;

        if let Some(dir) = self.dir.take() {
            self.sd_controller
                .close_dir(self.volume.as_ref().unwrap(), dir);
        }
    }
}

async fn get_artist<'a, SPI, CS, const MAX_DIRS: usize, const MAX_FILES: usize>(
    root_dir: &Directory,
    sd_controller: &mut Controller<BlockSpi<'a, SPI, CS>, DummyTimeSource>,
    volume: &Volume,
    album: &str,
) -> Artist<MAX_DIRS, MAX_FILES>
where
    SPI: SpiBus<u8>,
    CS: OutputPin,
{
    let album_dir = sd_controller
        .open_dir(&volume, &root_dir, &album)
        .await
        .unwrap();

    let mut album_names: Vec<String<11>, MAX_FILES> = Vec::new();
    sd_controller
        .iterate_dir(&volume, &album_dir, |dir| {
            album_names
                .push(String::from_utf8(Vec::from_slice(dir.name.base_name()).unwrap()).unwrap())
                .unwrap()
        })
        .await
        .unwrap();

    let mut albums = Vec::new();

    for album in album_names {
        let album_dir = sd_controller
            .open_dir(&volume, &root_dir, &album)
            .await
            .unwrap();

        albums
            .push(get_album(&album_dir, sd_controller, volume, album).await)
            .unwrap()
    }

    sd_controller.close_dir(&volume, album_dir);

    Artist { albums }
}

async fn get_album<'a, SPI, CS, const MAX_FILES: usize>(
    artist_dir: &Directory,
    sd_controller: &mut Controller<BlockSpi<'a, SPI, CS>, DummyTimeSource>,
    volume: &Volume,
    album: String<11>,
) -> Album<MAX_FILES>
where
    SPI: SpiBus<u8>,
    CS: OutputPin,
{
    let album_dir = sd_controller
        .open_dir(&volume, &artist_dir, &album)
        .await
        .unwrap();

    let mut songs: Vec<String<11>, MAX_FILES> = Vec::new();
    sd_controller
        .iterate_dir(&volume, &album_dir, |dir| {
            songs
                .push(String::from_utf8(Vec::from_slice(dir.name.base_name()).unwrap()).unwrap())
                .unwrap()
        })
        .await
        .unwrap();

    sd_controller.close_dir(&volume, album_dir);

    Album { songs }
}
