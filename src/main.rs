#![no_std]
#![no_main]

#[cfg(feature = "bluetooth")]
use bt_hci::controller::ExternalController;
#[cfg(feature = "bluetooth")]
use embassy_futures::select::select;
use embedded_graphics::mock_display::ColorMapping;
use embedded_graphics::pixelcolor::{Rgb565, Rgb666};
use embedded_graphics::prelude::RgbColor;
use embedded_graphics_core::draw_target::DrawTarget;
use embedded_hal::delay::DelayNs;
use embedded_sdmmc::{BlockDevice, Mode, VolumeIdx, VolumeManager};
use mipidsi::interface::SpiInterface;
use mipidsi::models::ST7789;
use mipidsi::options::{ColorInversion, Orientation};
#[cfg(feature = "bluetooth")]
use trouble_host::{Address, Host, HostResources};

use cyw43_pio::PioSpi;
use defmt::*;
use display::MediaUi;
use embassy_executor::Executor;
use embassy_rp::bind_interrupts;
use embassy_rp::gpio::{Level, Output};
use embassy_rp::i2c;
use embassy_rp::multicore::{Stack, spawn_core1};
use embassy_rp::peripherals::{DMA_CH2, I2C0, I2C1, PIN_2, PIN_3, PIN_4, PIN_5, PIO0, PIO1, SPI0};
use embassy_rp::pio::{self, Pio};
use embassy_rp::spi::{self, Spi};
use embassy_sync::blocking_mutex::raw::CriticalSectionRawMutex;
use embassy_sync::channel::Channel;
use embassy_time::{Delay, Duration, Timer};
use embedded_hal_bus::spi::{ExclusiveDevice, NoDelay};
use embedded_sdmmc::sdcard::{DummyCsPin, SdCard};
use static_cell::StaticCell;
use wavv::{DataBulk, Wav};
use {defmt_rtt as _, panic_probe as _};

#[cfg(feature = "bluetooth")]
mod ble;
mod display;
mod storage;

static mut CORE1_STACK: Stack<4096> = Stack::new();
static EXECUTOR0: StaticCell<Executor> = StaticCell::new();
static EXECUTOR1: StaticCell<Executor> = StaticCell::new();

bind_interrupts!(struct Irqs {
    // bluetooth
    PIO1_IRQ_0 => pio::InterruptHandler<PIO1>;
});

#[cortex_m_rt::entry]
fn main() -> ! {
    let p = embassy_rp::init(Default::default());

    // Sets up Bluetooth and Trouble
    let (spi, pwr) = {
        let pwr = Output::new(p.PIN_23, Level::Low);
        let cs = Output::new(p.PIN_25, Level::High);
        let mut pio = Pio::new(p.PIO1, Irqs);
        let spi = PioSpi::new(
            &mut pio.common,
            pio.sm0,
            cyw43_pio::DEFAULT_CLOCK_DIVIDER,
            pio.irq0,
            cs,
            p.PIN_24,
            p.PIN_29,
            p.DMA_CH2,
        );
        (spi, pwr)
    };

    // Set up SPI1 for ST7789 TFT Display
    let (display, backlight) = {
        let mut config = spi::Config::default();
        config.frequency = 2_000_000;
        let spi = Spi::new_blocking(p.SPI1, p.PIN_10, p.PIN_11, p.PIN_12, config);
        let dc = Output::new(p.PIN_13, Level::Low);
        let cs = Output::new(p.PIN_14, Level::Low);
        let spi_dev = ExclusiveDevice::new(spi, cs, Delay);
        static BUFFER: StaticCell<[u8; 512]> = StaticCell::new();
        let buffer = BUFFER.init([0; 512]);
        let interface = SpiInterface::new(spi_dev, dc, buffer);
        let display = mipidsi::Builder::new(ST7789, interface)
            .orientation(Orientation::new().rotate(mipidsi::options::Rotation::Deg90))
            .display_size(display::H as u16, display::W as u16)
            .invert_colors(ColorInversion::Inverted)
            .init(&mut Delay)
            .unwrap();

        (display, Output::new(p.PIN_15, Level::Low))
    };

    // spawning ui and io task
    spawn_core1(
        p.CORE1,
        unsafe { &mut *core::ptr::addr_of_mut!(CORE1_STACK) },
        move || {
            let executor1 = EXECUTOR1.init(Executor::new());
            executor1
                .run(|spawner| unwrap!(spawner.spawn(core1_task(display, backlight, pwr, spi))));
        },
    );

    // Set up SPI0 for the Micro SD reader
    let sdcard = {
        let mut config = spi::Config::default();
        config.frequency = 400_000;
        let spi = spi::Spi::new_blocking(p.SPI0, p.PIN_2, p.PIN_3, p.PIN_4, config);
        // Use a dummy cs pin here, for embedded-hal SpiDevice compatibility reasons
        let spi_dev = ExclusiveDevice::new_no_delay(spi, DummyCsPin);
        // Real cs pin
        let cs = Output::new(p.PIN_5, Level::High);

        let sdcard = SdCard::new(spi_dev, cs, embassy_time::Delay);
        // Now that the card is initialized, the SPI clock can go faster
        let mut config = spi::Config::default();
        config.frequency = 16_000_000;
        sdcard.spi(|dev| dev.bus_mut().set_config(&config));
        sdcard
    };

    // spawning compute task
    let executor0 = EXECUTOR0.init(Executor::new());
    executor0.run(|spawner| unwrap!(spawner.spawn(core0_task(sdcard))));
}

const DATASIZE: usize = 64;
static CHANNEL: Channel<CriticalSectionRawMutex, wavv::DataBulk<DATASIZE>, 5> = Channel::new();

// Ui and media playback
#[embassy_executor::task]
async fn core1_task(
    mut display: display::DISPLAY,
    backlight: Output<'static>,
    // mut dac: MCP4725<i2c::I2c<'static, I2C1, i2c::Async>>,
    pwr: Output<'static>,
    spi: PioSpi<'static, PIO1, 0, DMA_CH2>,
) {
    info!("hello from core 0");

    let mut media_ui = MediaUi::new(display, backlight);
    Timer::after_secs(2).await;
    media_ui.sleep();

    // Configure bluetooth
    #[cfg(feature = "bluetooth")]
    {
        // Release:
        #[cfg(not(debug_assertions))]
        let (fw, clm, btfw) = (
            include_bytes!("../cyw43-firmware/43439A0.bin"),
            include_bytes!("../cyw43-firmware/43439A0_clm.bin"),
            include_bytes!("../cyw43-firmware/43439A0_btfw.bin"),
        );

        // Dev
        #[cfg(debug_assertions)]
        let (fw, clm, btfw) = (
            unsafe { core::slice::from_raw_parts(0x10100000 as *const u8, 224190) },
            unsafe { core::slice::from_raw_parts(0x10140000 as *const u8, 4752) },
            unsafe { core::slice::from_raw_parts(0x10141400 as *const u8, 6164) },
        );

        static STATE: StaticCell<cyw43::State> = StaticCell::new();
        let state = STATE.init(cyw43::State::new());
        let (_net_device, bt_device, mut control, runner) =
            cyw43::new_with_bluetooth(state, pwr, spi, fw, btfw).await;
        control.init(clm).await;
        let controller: ble::ControllerT = ExternalController::new(bt_device);

        let address: Address = Address::random([0xff, 0x8f, 0x1b, 0x05, 0xe4, 0xff]);
        static RESOURCES: StaticCell<ble::Resources> = StaticCell::new();
        let stack = trouble_host::new(controller, RESOURCES.init(HostResources::new()))
            .set_random_address(address);
        let Host {
            central,
            mut runner,
            peripheral,
            ..
        } = stack.build();

        select(
            runner.run(),
            ble::run(&mut runner, central, peripheral).await,
        )
        .await;
    }

    // info!("playing music");
    // loop {
    //     let samples = CHANNEL.receive().await;
    //     if let DataBulk::BitDepth8(samples) = samples {
    //         for sample in samples {
    //             dac.set_dac_fast(PowerDown::Normal, sample.into()).ok();
    //             Timer::after(Duration::from_hz(8000)).await;
    //         }
    //     }
    // }
}

// File system & Decoding
#[embassy_executor::task]
async fn core0_task(sdcard: storage::SD) {
    info!("hello from core 1");
    while let Err(_) = sdcard.num_bytes() {
        info!("Sdcard not found, looking again in 1 second");
        Timer::after_secs(1).await;
    }
    info!("sd size:{}", sdcard.num_bytes().unwrap());

    let mut volume_mgr = VolumeManager::new(sdcard, storage::DummyTimesource());
    let mut volume0 = volume_mgr.open_volume(VolumeIdx(0)).unwrap();
    let mut root_dir = volume0.open_root_dir().unwrap();
    let mut music_dir = root_dir.open_dir("music").unwrap();
    info!("reading test file");

    // loop {
    //     let file = music_dir
    //         .open_file_in_dir("test.wav", Mode::ReadOnly)
    //         .unwrap();
    //     let mut wav = Wav::new(file).unwrap();
    //     info!("[Library] Wav size: {}", wav.data.end);
    //     info!(
    //         "File Info:\nsample_rate: {}, num_channels: {}, bit_depth: {}",
    //         wav.fmt.sample_rate, wav.fmt.num_channels, wav.fmt.bit_depth
    //     );

    //     while !wav.is_end() {
    //         let samples = wav.next_n::<DATASIZE>().unwrap();
    //         CHANNEL.send(samples).await;
    //     }
    // }
}
