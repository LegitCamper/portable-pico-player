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
use embassy_rp::peripherals::{DMA_CH0, I2C0, I2C1, PIO0};
use embassy_rp::pio::{self, Pio};
use embassy_time::{Duration, Timer};
use ssd1306::mode::{DisplayConfig, TerminalMode};
use ssd1306::prelude::{DisplayRotation, I2CInterface};
use ssd1306::size::DisplaySize128x32;
use ssd1306::{I2CDisplayInterface, Ssd1306};
use static_cell::StaticCell;
use trouble_host::{HostResources, prelude::*};
use {defmt_rtt as _, embassy_time as _, panic_probe as _};

#[embassy_executor::task]
pub async fn display_task(i2c0: embassy_rp::i2c::I2c<'static, I2C0, Async>) {
    let interface = I2CDisplayInterface::new(i2c0);
    let display =
        Ssd1306::new(interface, DisplaySize128x32, DisplayRotation::Rotate0).into_terminal_mode();

    let mut ui = MediaUi::new(display, 50);

    ui.draw();
}

struct MediaUi<'a> {
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
