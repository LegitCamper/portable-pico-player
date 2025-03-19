use core::str::FromStr;

use audio_parser::AudioFile;
use defmt::{Format, info, warn};
use embassy_rp::{
    gpio::Output,
    peripherals::SPI0,
    spi::{self, Spi},
};
use embedded_hal_bus::spi::ExclusiveDevice;
use embedded_sdmmc::asynchronous::{
    DirEntry, Directory, LfnBuffer, Mode, SdCard, ShortFileName, TimeSource, Timestamp, Volume,
};
use heapless::{String, Vec};

pub const MAX_DIRS: usize = 4;
pub const MAX_FILES: usize = 4;
pub const MAX_VOLUMES: usize = 1;
// Max file or dir name string len
pub const MAX_NAME_LEN: usize = 25;

#[derive(Debug, Format)]
pub struct Album<const MAX_FILES: usize> {
    pub name: String<MAX_NAME_LEN>,
    pub songs: Vec<String<MAX_NAME_LEN>, MAX_FILES>,
}

#[derive(Debug, Format)]
pub struct Artist<const MAX_DIRS: usize, const MAX_FILES: usize> {
    pub name: String<MAX_NAME_LEN>,
    pub albums: Vec<Album<MAX_FILES>, MAX_DIRS>,
}

pub struct DummyTimeSource {}
impl TimeSource for DummyTimeSource {
    fn get_timestamp(&self) -> Timestamp {
        Timestamp::from_calendar(2022, 1, 1, 0, 0, 0).unwrap()
    }
}

type Device = ExclusiveDevice<Spi<'static, SPI0, spi::Async>, Output<'static>, embassy_time::Delay>;
pub type SD = SdCard<Device, embassy_time::Delay>;
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

    pub fn get_root_dir(&self) -> Dir {
        self.volume.open_root_dir().unwrap()
    }

    pub async fn discover_music(&mut self) {
        let root_dir = self.volume.open_root_dir().unwrap();

        let mut artist_names: Vec<String<MAX_NAME_LEN>, MAX_FILES> = Vec::new();

        let mut buf = [0u8; MAX_NAME_LEN];
        let mut lfn_buffer = LfnBuffer::new(&mut buf);
        root_dir
            .iterate_dir_lfn(&mut lfn_buffer, |entry, lfn| {
                if !artist_names.is_full() {
                    if entry.attributes.is_directory() && !ignore_name(&entry.name) {
                        artist_names.push(get_name(entry, lfn)).unwrap()
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
    let mut album_names: Vec<String<MAX_NAME_LEN>, MAX_FILES> = Vec::new();

    let mut buf = [0u8; MAX_NAME_LEN];
    let mut lfn_buffer = LfnBuffer::new(&mut buf);
    dir.iterate_dir_lfn(&mut lfn_buffer, |entry, lfn| {
        if !album_names.is_full() {
            if entry.attributes.is_directory() && !ignore_name(&entry.name) {
                album_names.push(get_name(entry, lfn)).unwrap()
            }
        } else {
            warn!("Too many songs in album. increase MAX_DIRS");
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

async fn get_album<'a, const MAX_FILES: usize>(
    dir: &Dir<'a>,
) -> Vec<String<MAX_NAME_LEN>, MAX_FILES> {
    let mut songs: Vec<String<MAX_NAME_LEN>, MAX_FILES> = Vec::new();

    let mut buf = [0u8; MAX_NAME_LEN];
    let mut lfn_buffer = LfnBuffer::new(&mut buf);
    dir.iterate_dir_lfn(&mut lfn_buffer, |entry, lfn| {
        if !songs.is_full() {
            if !entry.attributes.is_directory() && !ignore_name(&entry.name) {
                songs.push(get_name(entry, lfn)).unwrap()
            }
        } else {
            warn!("Too many songs in album. increase MAX_FILES");
        }
    })
    .await
    .unwrap();

    songs
}

fn get_name<'a>(entry: &DirEntry, lfn: Option<&'a str>) -> String<MAX_NAME_LEN> {
    if let Some(lfn) = lfn {
        String::from_str(lfn).unwrap()
    } else {
        String::from_utf8(Vec::from_slice(entry.name.base_name()).unwrap()).unwrap()
    }
}

fn ignore_name(name: &ShortFileName) -> bool {
    let name = str::from_utf8(name.base_name()).unwrap();
    name == "." || name == ".." || name.contains("TRASH")
}
