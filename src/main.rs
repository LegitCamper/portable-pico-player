#![no_std]
#![no_main]

#[cfg(feature = "bluetooth")]
use bt_hci::controller::ExternalController;
#[cfg(feature = "bluetooth")]
use embassy_futures::select::select;
use embedded_sdmmc::{BlockDevice, Mode, VolumeIdx, VolumeManager};
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
use embassy_rp::spi;
use embassy_sync::blocking_mutex::raw::CriticalSectionRawMutex;
use embassy_sync::channel::Channel;
use embassy_time::{Delay, Duration, Timer};
use embedded_hal_bus::spi::ExclusiveDevice;
use embedded_sdmmc::sdcard::{DummyCsPin, SdCard};
use mcp4725::{MCP4725, PowerDown};
use ssd1306::prelude::{DisplayRotation, I2CInterface};
use ssd1306::size::DisplaySize128x32;
use ssd1306::{I2CDisplayInterface, Ssd1306};
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
    // oled
    I2C0_IRQ => i2c::InterruptHandler<I2C0>;
    // dac
    PIO0_IRQ_0 => pio::InterruptHandler<PIO0>;
    I2C1_IRQ => i2c::InterruptHandler<I2C1>;
    // bluetooth
    PIO1_IRQ_0 => pio::InterruptHandler<PIO1>;
});

#[cortex_m_rt::entry]
fn main() -> ! {
    let p = embassy_rp::init(Default::default());

    // Set up I2C0 for the SSD1306 OLED Display
    let display = {
        let i2c0 = i2c::I2c::new_async(p.I2C0, p.PIN_1, p.PIN_0, Irqs, i2c::Config::default());
        let interface = I2CDisplayInterface::new(i2c0);
        Ssd1306::new(interface, DisplaySize128x32, DisplayRotation::Rotate0).into_terminal_mode()
    };

    // Set up I2C1 for the MCP4725 12-Bit DAC
    let mut conf = i2c::Config::default();
    conf.frequency = 1_000_000;
    let i2c1 = i2c::I2c::new_async(p.I2C1, p.PIN_15, p.PIN_14, Irqs, conf);
    let dac = MCP4725::new(i2c1, 0b010);

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

    // spawning ui and io task
    spawn_core1(
        p.CORE1,
        unsafe { &mut *core::ptr::addr_of_mut!(CORE1_STACK) },
        move || {
            let executor1 = EXECUTOR1.init(Executor::new());
            executor1.run(|spawner| unwrap!(spawner.spawn(core1_task(display, dac, pwr, spi))));
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
        info!("Card size is {} bytes", sdcard.num_bytes().unwrap());

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
static CHANNEL: Channel<CriticalSectionRawMutex, wavv::DataBulk<DATASIZE>, 50> = Channel::new();

// Ui and media playback
#[embassy_executor::task]
async fn core1_task(
    display: Ssd1306<
        I2CInterface<i2c::I2c<'static, I2C0, i2c::Async>>,
        DisplaySize128x32,
        ssd1306::mode::TerminalMode,
    >,
    mut dac: MCP4725<i2c::I2c<'static, I2C1, i2c::Async>>,
    pwr: Output<'static>,
    spi: PioSpi<'static, PIO1, 0, DMA_CH2>,
) {
    info!("hello from core 0");
    let mut media_ui = MediaUi::new(display, 25);
    media_ui.draw();

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

    info!("playing music");
    loop {
        let samples = CHANNEL.receive().await;
        if let DataBulk::BitDepth8(samples) = samples {
            for sample in samples {
                dac.set_dac(PowerDown::Normal, sample.into()).ok();
                Timer::after(Duration::from_hz(8000)).await;
            }
        }
    }
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

    loop {
        let file = music_dir
            .open_file_in_dir("test.wav", Mode::ReadOnly)
            .unwrap();
        let mut wav = Wav::new(file).unwrap();
        info!("[Library] Wav size: {}", wav.data.end);
        info!(
            "File Info:\nsample_rate: {}, num_channels: {}, bit_depth: {}",
            wav.fmt.sample_rate, wav.fmt.num_channels, wav.fmt.bit_depth
        );

        while !wav.is_end() {
            let samples = wav.next_n::<DATASIZE>().unwrap();
            CHANNEL.send(samples).await;
        }
    }
}
