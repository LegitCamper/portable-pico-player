#![no_std]
#![no_main]

use core::fmt::Write;
use core::ops::BitAndAssign;

use bt_hci::controller::ExternalController;
use cyw43_pio::PioSpi;
use defmt::*;
use embassy_executor::Spawner;
use embassy_futures::select::select;
use embassy_rp::bind_interrupts;
use embassy_rp::gpio::{Level, Output};
use embassy_rp::i2c::{self, Async, Config};
use embassy_rp::peripherals::{DMA_CH0, I2C0, PIO0};
use embassy_rp::pio::{self, Pio};
use embassy_time::{Duration, Timer};
use ssd1306::mode::{DisplayConfig, TerminalMode};
use ssd1306::prelude::{DisplayRotation, I2CInterface};
use ssd1306::size::DisplaySize128x32;
use ssd1306::{I2CDisplayInterface, Ssd1306};
use static_cell::StaticCell;
use trouble_host::{HostResources, prelude::*};
use {defmt_rtt as _, embassy_time as _, panic_probe as _};

mod ble;

bind_interrupts!(struct Irqs {
    PIO0_IRQ_0 => pio::InterruptHandler<PIO0>;
    I2C0_IRQ => i2c::InterruptHandler<I2C0>;
});

#[embassy_executor::task]
async fn cyw43_task(
    runner: cyw43::Runner<'static, Output<'static>, PioSpi<'static, PIO0, 0, DMA_CH0>>,
) -> ! {
    runner.run().await
}

#[embassy_executor::main]
async fn main(spawner: Spawner) {
    let p = embassy_rp::init(Default::default());

    // Set up I2C0 for the SSD1306 OLED Display
    let i2c0 = i2c::I2c::new_async(p.I2C0, p.PIN_1, p.PIN_0, Irqs, Config::default());
    unwrap!(spawner.spawn(display_task(i2c0)));

    // Release:
    #[cfg(not(debug_assertions))]
    let fw = include_bytes!("../cyw43-firmware/43439A0.bin");
    #[cfg(not(debug_assertions))]
    let clm = include_bytes!("../cyw43-firmware/43439A0_clm.bin");
    #[cfg(not(debug_assertions))]
    let btfw = include_bytes!("../cyw43-firmware/43439A0_btfw.bin");

    // Dev
    #[cfg(debug_assertions)]
    let fw = unsafe { core::slice::from_raw_parts(0x10100000 as *const u8, 224190) };
    #[cfg(debug_assertions)]
    let clm = unsafe { core::slice::from_raw_parts(0x10140000 as *const u8, 4752) };
    #[cfg(debug_assertions)]
    let btfw = unsafe { core::slice::from_raw_parts(0x10141400 as *const u8, 6164) };

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

    static STATE: StaticCell<cyw43::State> = StaticCell::new();
    let state = STATE.init(cyw43::State::new());
    let (_net_device, bt_device, mut control, runner) =
        cyw43::new_with_bluetooth(state, pwr, spi, fw, btfw).await;
    unwrap!(spawner.spawn(cyw43_task(runner)));
    control.init(clm).await;
    let controller: ble::ControllerT = ExternalController::new(bt_device);

    // Using a fixed "random" address can be useful for testing. In real scenarios, one would
    // use e.g. the MAC 6 byte array as the address (how to get that varies by the platform).
    let address: Address = Address::random([0xff, 0x8f, 0x1b, 0x05, 0xe4, 0xff]);
    info!("Our address = {:?}", address);

    static RESOURCES: StaticCell<ble::Resources> = StaticCell::new();
    let stack = trouble_host::new(controller, RESOURCES.init(HostResources::new()))
        .set_random_address(address);
    let Host {
        central: _,
        mut runner,
        mut peripheral,
        ..
    } = stack.build();

    // select(
    //     ble::ble_task(&mut runner),
    //     ble::le_audio_periphery_test(&mut peripheral, &stack),
    // )
    // .await;
}

#[embassy_executor::task]
async fn display_task(i2c0: embassy_rp::i2c::I2c<'static, I2C0, Async>) {
    let interface = I2CDisplayInterface::new(i2c0);
    let display =
        Ssd1306::new(interface, DisplaySize128x32, DisplayRotation::Rotate0).into_terminal_mode();

    let mut ui = MediaUi::new(display, 50);

    ui.draw();
}
pub struct MediaUi<'a> {
    display: Ssd1306<
        I2CInterface<embassy_rp::i2c::I2c<'a, I2C0, embassy_rp::i2c::Async>>,
        DisplaySize128x32,
        TerminalMode,
    >,
    paused: bool,
    song: &'a str,
    volume: u8,
}

impl<'a> MediaUi<'a> {
    const VOLUME: u8 = 0;
    const SONG: u8 = 2;
    const MEDIA_CONTROLS: u8 = 3;

    fn new(
        mut display: Ssd1306<
            I2CInterface<embassy_rp::i2c::I2c<'a, I2C0, embassy_rp::i2c::Async>>,
            DisplaySize128x32,
            TerminalMode,
        >,
        volume: u8,
    ) -> Self {
        display.init().unwrap();
        display.clear().unwrap();

        Self {
            display,
            paused: true,
            song: "Not Playing",
            volume,
        }
    }

    pub fn center_str(&self, text: &str) -> u8 {
        let (width, _height) = self.display.dimensions();
        let width = width / 8;

        (width - text.len() as u8) / 2
    }

    pub fn center_int(&self, num: u8) -> u8 {
        let (width, _height) = self.display.dimensions();
        let width = width / 8;

        if num < 10 {
            (width - num as u8) / 2
        } else if num < 100 {
            (width - 2 as u8) / 2
        } else {
            (width - 3 as u8) / 2
        }
    }

    pub fn center_width(&self, item_width: u8) -> u8 {
        let (width, _height) = self.display.dimensions();
        let width = width / 8;

        (width - item_width) / 2
    }

    fn draw(&mut self) {
        self.display
            .set_position(self.center_int(self.volume), Self::VOLUME)
            .unwrap();
        let vol = [self.volume];
        self.display
            .write_str(unsafe { core::str::from_utf8_unchecked(&vol) })
            .unwrap();
        self.display.write_str("%").unwrap();

        self.display
            .set_position(self.center_str(&self.song), Self::SONG)
            .unwrap();
        self.display.write_str(&self.song).unwrap();

        self.display
            .set_position(self.center_width(5), Self::MEDIA_CONTROLS)
            .unwrap();
        self.display.write_str("B P N").unwrap();
    }
}
