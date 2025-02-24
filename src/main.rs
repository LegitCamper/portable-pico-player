#![no_std]
#![no_main]
#![feature(async_trait_bounds)]

// use bt_hci::controller::ExternalController;
use cyw43_pio::PioSpi;
use defmt::*;
use display::MediaUi;
use embassy_executor::Executor;
use embassy_rp::bind_interrupts;
use embassy_rp::gpio::{Level, Output};
use embassy_rp::i2c;
use embassy_rp::peripherals::{DMA_CH0, I2C0, I2C1, PIO0, PIO1};
use embassy_rp::pio;
use embassy_rp::spi;
use embassy_sync::blocking_mutex::raw::CriticalSectionRawMutex;
use embassy_sync::channel::Channel;
use embedded_hal_bus::spi::ExclusiveDevice;
use embedded_sdmmc::sdcard::{DummyCsPin, SdCard};
use mcp4725::{MCP4725, PowerDown};
use ssd1306::prelude::DisplayRotation;
use ssd1306::size::DisplaySize128x32;
use ssd1306::{I2CDisplayInterface, Ssd1306};
use static_cell::StaticCell;
use storage::Library;
use wavv::DataBulk;
// use trouble_host::{HostResources, prelude::*};
use embassy_rp::multicore::{Stack, spawn_core1};
use {defmt_rtt as _, panic_probe as _};

// mod ble;
mod display;
mod storage;

static mut CORE1_STACK: Stack<4096> = Stack::new();
static EXECUTOR0: StaticCell<Executor> = StaticCell::new();
static EXECUTOR1: StaticCell<Executor> = StaticCell::new();

bind_interrupts!(struct Irqs {
    PIO0_IRQ_0 => pio::InterruptHandler<PIO0>;
    I2C0_IRQ => i2c::InterruptHandler<I2C0>;
    PIO1_IRQ_0 => pio::InterruptHandler<PIO1>;
    I2C1_IRQ => i2c::InterruptHandler<I2C1>;
});

#[embassy_executor::task]
async fn cyw43_task(
    runner: cyw43::Runner<'static, Output<'static>, PioSpi<'static, PIO0, 0, DMA_CH0>>,
) -> ! {
    runner.run().await
}

#[cortex_m_rt::entry]
fn main() -> ! {
    let p = embassy_rp::init(Default::default());

    // Set up SPI0 for the Micro SD reader
    let library = {
        let mut config = spi::Config::default();
        config.frequency = 1_00_000;
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

        // unwrap!(spawner.spawn(storage_task(sdcard)));
        storage::Library::new(sdcard)
    };

    // spawning compute task
    spawn_core1(
        p.CORE1,
        unsafe { &mut *core::ptr::addr_of_mut!(CORE1_STACK) },
        move || {
            let executor1 = EXECUTOR1.init(Executor::new());
            executor1.run(|spawner| unwrap!(spawner.spawn(core1_task(library))));
        },
    );

    // Set up I2C0 for the SSD1306 OLED Display
    let display = {
        let i2c0 = i2c::I2c::new_async(p.I2C0, p.PIN_1, p.PIN_0, Irqs, i2c::Config::default());
        let interface = I2CDisplayInterface::new(i2c0);
        let display = Ssd1306::new(interface, DisplaySize128x32, DisplayRotation::Rotate0)
            .into_terminal_mode();
        MediaUi::new(display, 50)
    };

    // Set up I2C1 for the MCP4725 12-Bit DAC
    let mut conf = i2c::Config::default();
    conf.frequency = 1_000_000;
    let i2c1 = i2c::I2c::new_async(p.I2C1, p.PIN_15, p.PIN_14, Irqs, conf);
    let dac = MCP4725::new(i2c1, 0b010);

    // Spawn main core task
    let executor0 = EXECUTOR0.init(Executor::new());
    executor0.run(|spawner| unwrap!(spawner.spawn(core0_task(display, dac))));

    // Sets up Bluetooth and Trouble
    // {
    //     // Release:
    //     #[cfg(not(debug_assertions))]
    //     let fw = include_bytes!("../cyw43-firmware/43439A0.bin");
    //     #[cfg(not(debug_assertions))]
    //     let clm = include_bytes!("../cyw43-firmware/43439A0_clm.bin");
    //     #[cfg(not(debug_assertions))]
    //     let btfw = include_bytes!("../cyw43-firmware/43439A0_btfw.bin");

    //     // Dev
    //     #[cfg(debug_assertions)]
    //     let fw = unsafe { core::slice::from_raw_parts(0x10100000 as *const u8, 224190) };
    //     #[cfg(debug_assertions)]
    //     let clm = unsafe { core::slice::from_raw_parts(0x10140000 as *const u8, 4752) };
    //     #[cfg(debug_assertions)]
    //     let btfw = unsafe { core::slice::from_raw_parts(0x10141400 as *const u8, 6164) };

    //     let pwr = Output::new(p.PIN_23, Level::Low);
    //     let cs = Output::new(p.PIN_25, Level::High);
    //     let mut pio = Pio::new(p.PIO0, Irqs);
    //     let spi = PioSpi::new(
    //         &mut pio.common,
    //         pio.sm0,
    //         cyw43_pio::DEFAULT_CLOCK_DIVIDER,
    //         pio.irq0,
    //         cs,
    //         p.PIN_24,
    //         p.PIN_29,
    //         p.DMA_CH0,
    //     );

    //     static STATE: StaticCell<cyw43::State> = StaticCell::new();
    //     let state = STATE.init(cyw43::State::new());
    //     let (_net_device, bt_device, mut control, runner) =
    //         cyw43::new_with_bluetooth(state, pwr, spi, fw, btfw).await;
    //     unwrap!(spawner.spawn(cyw43_task(runner)));
    //     control.init(clm).await;
    //     let controller: ble::ControllerT = ExternalController::new(bt_device);

    //     let address: Address = Address::random([0xff, 0x8f, 0x1b, 0x05, 0xe4, 0xff]);
    //     static RESOURCES: StaticCell<ble::Resources> = StaticCell::new();
    //     let stack = trouble_host::new(controller, RESOURCES.init(HostResources::new()))
    //         .set_random_address(address);
    //     let Host {
    //         central,
    //         mut runner,
    //         peripheral,
    //         ..
    //     } = stack.build();

    //     ble::run(&mut runner, central, peripheral).await;
    // }
}

static CHANNEL: Channel<CriticalSectionRawMutex, wavv::DataBulk<10>, 5> = Channel::new();

// Ui and media playback
#[embassy_executor::task]
async fn core0_task(mut display: MediaUi, mut dac: MCP4725<i2c::I2c<'static, I2C1, i2c::Async>>) {
    display.draw();

    info!("playing music");
    loop {
        let samples = CHANNEL.receive().await;

        if let DataBulk::BitDepth8(samples) = samples {
            for sample in samples {
                dac.set_dac_fast(PowerDown::Normal, sample.into()).ok();
            }
        }
    }
}

// File system & Decoding
#[embassy_executor::task]
async fn core1_task(mut library: Library) {
    info!("reading test file");
    loop {
        library.read_wav("test.wav").await;
    }
}
