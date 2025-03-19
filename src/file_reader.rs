use defmt::{Format, info, warn};
use embassy_rp::{
    gpio::Output,
    peripherals::SPI0,
    spi::{self, Spi},
};
use embedded_hal_bus::spi::ExclusiveDevice;
use embedded_sdmmc::asynchronous::{Directory, SdCard, TimeSource, Timestamp, Volume};
use heapless::{String, Vec};

pub const MAX_DIRS: usize = 4;
pub const MAX_FILES: usize = 4;
pub const MAX_VOLUMES: usize = 1;

#[derive(Debug, Format)]
pub struct Album<const MAX_FILES: usize> {
    name: String<11>,
    songs: Vec<String<11>, MAX_FILES>,
}

#[derive(Debug, Format)]
pub struct Artist<const MAX_DIRS: usize, const MAX_FILES: usize> {
    name: String<11>,
    albums: Vec<Album<MAX_FILES>, MAX_DIRS>,
}

pub struct DummyTimeSource {}
impl TimeSource for DummyTimeSource {
    fn get_timestamp(&self) -> Timestamp {
        Timestamp::from_calendar(2022, 1, 1, 0, 0, 0).unwrap()
    }
}

type Device = ExclusiveDevice<Spi<'static, SPI0, spi::Async>, Output<'static>, embassy_time::Delay>;
type SD = SdCard<Device, embassy_time::Delay>;
type Dir<'a> = Directory<'a, SD, DummyTimeSource, MAX_DIRS, MAX_FILES, MAX_VOLUMES>;

pub struct Library<'a> {
    volume: Volume<'a, SD, DummyTimeSource, MAX_DIRS, MAX_FILES, MAX_VOLUMES>,
    artists: Option<Vec<Artist<MAX_DIRS, MAX_FILES>, MAX_DIRS>>,
}

impl<'a> Library<'a> {
    pub fn new(volume: Volume<'a, SD, DummyTimeSource, MAX_DIRS, MAX_FILES, MAX_VOLUMES>) -> Self {
        Self {
            artists: None,
            volume,
        }
    }

    pub async fn discover_music(&mut self) {
        let root_dir = self.volume.open_root_dir().unwrap();

        let mut artist_names: Vec<String<11>, MAX_FILES> = Vec::new();
        root_dir
            .iterate_dir(|file| {
                if !artist_names.is_full() {
                    if file.attributes.is_directory() {
                        artist_names
                            .push(
                                String::from_utf8(Vec::from_slice(file.name.base_name()).unwrap())
                                    .unwrap(),
                            )
                            .unwrap()
                    }
                } else {
                    warn!("Too many songs in album. increase MAX_DIRS");
                }
            })
            .await
            .unwrap();

        let mut artists = Vec::new();

        for artist in artist_names {
            info!("looking for {}", artist.as_str());
            let artist_dir = root_dir.open_dir(artist.as_str()).await.unwrap();
            artists
                .push(Artist {
                    albums: get_artist(&artist_dir).await,
                    name: artist,
                })
                .unwrap()
        }

        root_dir.close().unwrap();

        self.artists = Some(artists)
    }

    pub fn artists(&self) -> Option<&Vec<Artist<MAX_DIRS, MAX_FILES>, MAX_DIRS>> {
        self.artists.as_ref()
    }
}

async fn get_artist<'a>(dir: &Dir<'a>) -> Vec<Album<MAX_DIRS>, MAX_DIRS> {
    let mut album_names: Vec<String<11>, MAX_FILES> = Vec::new();
    dir.iterate_dir(|file| {
        if !album_names.is_full() {
            if file.attributes.is_directory()
                && str::from_utf8(file.name.base_name()).unwrap() != "."
                || str::from_utf8(file.name.base_name()).unwrap() != ".."
            {
                album_names
                    .push(
                        String::from_utf8(Vec::from_slice(file.name.base_name()).unwrap()).unwrap(),
                    )
                    .unwrap()
            }
        }
    })
    .await
    .unwrap();

    let mut albums = Vec::new();

    for album in album_names {
        info!("Looking for dir: {}", album);
        let album_dir = dir.open_dir(album.as_str()).await.unwrap();
        if albums
            .push(Album {
                songs: get_album(&album_dir).await,
                name: album,
            })
            .is_err()
        {
            album_dir.close().unwrap();
            break;
        }
        album_dir.close().unwrap();
    }

    albums
}

async fn get_album<'a, const MAX_FILES: usize>(dir: &Dir<'a>) -> Vec<String<11>, MAX_FILES> {
    let mut songs: Vec<String<11>, MAX_FILES> = Vec::new();
    dir.iterate_dir(|file| {
        if !songs.is_full() {
            if !file.attributes.is_directory()
                && str::from_utf8(file.name.base_name()).unwrap() != "."
                || str::from_utf8(file.name.base_name()).unwrap() != ".."
            {
                songs
                    .push(
                        String::from_utf8(Vec::from_slice(file.name.base_name()).unwrap()).unwrap(),
                    )
                    .unwrap()
            }
        } else {
            warn!("Too many songs in album. increase MAX_FILES");
        }
    })
    .await
    .unwrap();

    songs
}
