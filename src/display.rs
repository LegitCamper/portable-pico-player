use core::fmt::Write;
use core::str::FromStr;
use embassy_rp::peripherals::I2C0;
use heapless::String;
use ssd1306::Ssd1306;
use ssd1306::mode::{DisplayConfig, TerminalMode};
use ssd1306::prelude::I2CInterface;
use ssd1306::size::DisplaySize128x32;

pub const SONG_NAME_LEN: usize = 20;

pub struct MediaUi {
    display: Ssd1306<
        I2CInterface<embassy_rp::i2c::I2c<'static, I2C0, embassy_rp::i2c::Async>>,
        DisplaySize128x32,
        TerminalMode,
    >,
    pub paused: bool,
    pub song: String<SONG_NAME_LEN>,
    pub volume: u8,
}

impl MediaUi {
    const VOLUME: u8 = 0;
    const SONG: u8 = 2;
    const MEDIA_CONTROLS: u8 = 3;

    pub fn new(
        mut display: Ssd1306<
            I2CInterface<embassy_rp::i2c::I2c<'static, I2C0, embassy_rp::i2c::Async>>,
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
            song: String::from_str("Not Playing").unwrap(),
            volume,
        }
    }

    fn center_str(&self, text: &str) -> u8 {
        let (width, _height) = self.display.dimensions();
        let width = width / 8;

        (width - text.len() as u8) / 2
    }

    fn center_int(&self, num: u8) -> u8 {
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

    fn center_width(&self, item_width: u8) -> u8 {
        let (width, _height) = self.display.dimensions();
        let width = width / 8;

        (width - item_width) / 2
    }

    pub fn draw(&mut self) {
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
