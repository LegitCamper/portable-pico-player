#![no_std]
#![no_main]
#![feature(inherent_str_constructors)]
#![feature(impl_trait_in_assoc_type)]

use audio_parser::AudioFile;
use core::default::Default;
use defmt::{info, unwrap, warn};
use display::{Display, MediaUi};
use embassy_executor::Spawner;
use embassy_rp::bind_interrupts;
use embassy_rp::gpio::{Level, Output};
use embassy_rp::peripherals::{DMA_CH2, I2C0, I2C1, PIN_2, PIN_3, PIN_4, PIN_5, PIO0, PIO1, SPI0};
use embassy_rp::pio::{self, Pio};
use embassy_rp::pio_programs::i2s::{PioI2sOut, PioI2sOutProgram};
use embassy_rp::spi::{self, Spi};
use embassy_time::Timer;
use embedded_hal_bus::spi::ExclusiveDevice;
use embedded_sdmmc::asynchronous::{File, SdCard, ShortFileName, VolumeIdx, VolumeManager};
use {defmt_rtt as _, panic_probe as _};

// mod ble;
mod audio_playback;
use audio_playback::play_file;
mod display;
mod file_reader;
use file_reader::{DummyTimeSource, Library, MAX_DIRS, MAX_FILES, MAX_VOLUMES, SD};

bind_interrupts!(struct Irqs {
    // i2s
    PIO0_IRQ_0 => pio::InterruptHandler<PIO0>;
    // bluetooth
    PIO1_IRQ_0 => pio::InterruptHandler<PIO1>;
});

#[embassy_executor::main]
async fn main(spawner: Spawner) {
    let p = embassy_rp::init(Default::default());
    info!("Clock: {}", embassy_rp::clocks::clk_sys_freq());

    // Set up SPI1 for ST7789 TFT Display
    let mut buffer = [0u8; 4096];
    let display = Display::new(
        Output::new(p.PIN_15, Level::High),
        p.SPI1,
        p.PIN_10,
        p.PIN_11,
        Output::new(p.PIN_13, Level::Low),
        Output::new(p.PIN_14, Level::Low),
        &mut buffer,
    );
    Timer::after_secs(4).await;

    let mut media_ui = MediaUi::new(display);
    media_ui.init();

    // Set up SPI0 for the Micro SD reader
    let sdcard = {
        let mut config = spi::Config::default();
        config.frequency = 400_000;
        let spi = spi::Spi::new(
            p.SPI0, p.PIN_2, p.PIN_3, p.PIN_4, p.DMA_CH1, p.DMA_CH2, config,
        );
        let cs = Output::new(p.PIN_5, Level::High);

        let device = ExclusiveDevice::new(spi, cs, embassy_time::Delay).unwrap();
        let sdcard = SdCard::new(device, embassy_time::Delay);

        // Now that the card is initialized, the SPI clock can go faster
        let mut config = spi::Config::default();
        config.frequency = 32_000_000;
        sdcard.spi(|dev| dev.bus_mut().set_config(&config));
        sdcard
    };

    // i2s DAC
    let i2s = {
        const SAMPLE_RATE: u32 = 8_000;
        const BIT_DEPTH: u32 = 16; // this is the highest bit depth for stereo?
        const CHANNELS: u32 = 2;

        // Setup pio state machine for i2s output
        let Pio {
            mut common, sm0, ..
        } = Pio::new(p.PIO0, Irqs);

        let bit_clock_pin = p.PIN_18; // bclk
        let left_right_clock_pin = p.PIN_19; // wsel
        let data_pin = p.PIN_20; // din

        let _deem = Output::new(p.PIN_8, Level::High);

        let program = PioI2sOutProgram::new(&mut common);
        PioI2sOut::new(
            &mut common,
            sm0,
            p.DMA_CH0,
            data_pin,
            bit_clock_pin,
            left_right_clock_pin,
            SAMPLE_RATE,
            BIT_DEPTH,
            CHANNELS,
            &program,
        )
    };
    unwrap!(spawner.spawn(reader(sdcard, i2s)))
}

#[embassy_executor::task]
async fn reader(sdcard: SD, mut i2s: PioI2sOut<'static, PIO0, 0>) {
    let volume_mgr = VolumeManager::<_, _, MAX_DIRS, MAX_FILES, MAX_VOLUMES>::new_with_limits(
        sdcard,
        DummyTimeSource {},
        5000,
    );

    // Wait for sdcard
    let volume = {
        info!("Waiting for sd card...");
        loop {
            if let Ok(vol) = volume_mgr.open_volume(VolumeIdx(0)).await {
                break vol;
            }
            warn!("Could not init Sd card");
            Timer::after_millis(50).await;
        }
    };

    let mut library: Library = Library::new(volume);
    info!("indexing music");
    library.discover_music().await;
    info!("music: discovererd {:?}", library.artists());

    let root = library.get_root_dir();

    loop {
        let artist = &library.artists().as_ref().unwrap()[0];
        let artist_dir = root.open_dir(artist.name.as_str()).await.unwrap();
        let album = &artist.albums[0];
        let album_dir = artist_dir.open_dir(album.name.as_str()).await.unwrap();
        let song_name = &album.songs[3];
        let file = album_dir
            .open_file_in_dir(
                ShortFileName::create_from_str(song_name.as_str()).unwrap(),
                embedded_sdmmc::asynchronous::Mode::ReadOnly,
            )
            .await
            .unwrap();

        let mut audio_file: AudioFile<SD, DummyTimeSource, MAX_DIRS, MAX_FILES, MAX_VOLUMES> =
            AudioFile::new_wav(file).await.unwrap();

        info!("playing {}", song_name);
        play_file(&mut i2s, &mut audio_file).await;

        audio_file.destroy().close().await.unwrap();
        album_dir.close().unwrap();
        artist_dir.close().unwrap();
    }
}
