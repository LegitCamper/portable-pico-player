#![no_std]
#![no_main]
#![feature(async_trait_bounds)]

use bt_hci::controller::ExternalController;
use cyw43_pio::PioSpi;
use defmt::*;
use display::MediaUi;
use embassy_executor::Executor;
use embassy_futures::select::select;
use embassy_rp::bind_interrupts;
use embassy_rp::gpio::{Level, Output};
use embassy_rp::i2c;
use embassy_rp::multicore::{Stack, spawn_core1};
use embassy_rp::peripherals::{DMA_CH0, I2C0, I2C1, PIO0, PIO1, SPI0};
use embassy_rp::pio::{self, Pio};
use embassy_rp::spi;
use embassy_sync::blocking_mutex::raw::CriticalSectionRawMutex;
use embassy_sync::channel::Channel;
use embassy_time::Timer;
use embedded_hal_bus::spi::{ExclusiveDevice, NoDelay};
use embedded_sdmmc::sdcard::{DummyCsPin, SdCard};
use mcp4725::{MCP4725, PowerDown};
use ssd1306::prelude::{DisplayRotation, I2CInterface};
use ssd1306::size::DisplaySize128x32;
use ssd1306::{I2CDisplayInterface, Ssd1306};
use static_cell::StaticCell;
use trouble_host::{Address, Host, HostResources};
use wavv::DataBulk;
use {defmt_rtt as _, panic_probe as _};

#[cfg(feature = "bluetooth")]
mod ble;
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

#[cortex_m_rt::entry]
fn main() -> ! {
    let p = embassy_rp::init(Default::default());

    // Set up SPI0 for the Micro SD reader
    let sdcard = {
        let mut config = spi::Config::default();
        config.frequency = 1_00_000;
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
    spawn_core1(
        p.CORE1,
        unsafe { &mut *core::ptr::addr_of_mut!(CORE1_STACK) },
        move || {
            let executor1 = EXECUTOR1.init(Executor::new());
            executor1.run(|spawner| unwrap!(spawner.spawn(core1_task(sdcard))));
        },
    );

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
        let mut pio = Pio::new(p.PIO0, Irqs);
        let spi = PioSpi::new(
            &mut pio.common,
            pio.sm0,
            cyw43_pio::DEFAULT_CLOCK_DIVIDER,
            pio.irq0,
            cs,
            p.PIN_24,
            p.PIN_29,
            p.DMA_CH0,
        );
        (spi, pwr)
    };

    // Spawn main core task
    let executor0 = EXECUTOR0.init(Executor::new());
    executor0.run(|spawner| unwrap!(spawner.spawn(core0_task(display, dac, pwr, spi))));
}

const DATASIZE: usize = 10;
static CHANNEL: Channel<CriticalSectionRawMutex, wavv::DataBulk<DATASIZE>, 5> = Channel::new();

// Ui and media playback
#[embassy_executor::task]
async fn core0_task(
    display: Ssd1306<
        I2CInterface<i2c::I2c<'static, I2C0, i2c::Async>>,
        DisplaySize128x32,
        ssd1306::mode::TerminalMode,
    >,
    mut dac: MCP4725<i2c::I2c<'static, I2C1, i2c::Async>>,
    pwr: Output<'static>,
    spi: PioSpi<'static, PIO0, 0, DMA_CH0>,
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
        if let Ok(samples) = CHANNEL.try_receive() {
            if let DataBulk::BitDepth8(samples) = samples {
                for sample in samples {
                    info!("sample:{}", sample);
                    dac.set_dac_fast(PowerDown::Normal, sample.into()).ok();
                }
            }
        }
    }
}

// File system & Decoding
#[embassy_executor::task]
async fn core1_task(
    sdcard: SdCard<
        ExclusiveDevice<spi::Spi<'static, SPI0, spi::Blocking>, DummyCsPin, NoDelay>,
        Output<'static>,
        embassy_time::Delay,
    >,
) {
    info!("hello from core 1");
    while let Err(_) = sdcard.num_bytes() {
        info!("Sdcard not found");
        Timer::after_secs(1).await;
    }
    let mut library = storage::Library::new(sdcard);
    info!("reading test file");
    loop {
        library
            .read_wav("test.wav", async |mut wav| {
                // info!(
                //     "File Info:\nsample_rate: {}, num_channels: {}, bit_depth: {}",
                //     wav.fmt.sample_rate, wav.fmt.num_channels, wav.fmt.bit_depth
                // );

                // while !wav.is_end() {
                //     info!("reading sample batch");
                //     let samples = wav.next_n::<DATASIZE>().unwrap();
                //     CHANNEL.send(samples).await;
                // }
            })
            .await;
    }
}
