use core::{any::Any, str::FromStr};

use defmt::*;
use embassy_rp::{
    gpio::Output,
    peripherals::SPI1,
    spi::{Blocking, Spi},
};
use embassy_time::Delay;
use embedded_graphics::{
    draw_target::DrawTarget,
    mono_font::{MonoTextStyle, ascii::FONT_10X20},
    pixelcolor::Rgb565,
    prelude::{Point, RgbColor, *},
    text::Text,
};
use embedded_hal_bus::spi::ExclusiveDevice;
use heapless::String;
use mipidsi::{Display, NoResetPin, interface::SpiInterface, models::ST7789};

pub const SONG_NAME_LEN: usize = 20;

// ST7789 TFT Display diamentions
pub const W: i32 = 320;
pub const H: i32 = 240;

/// Noop `OutputPin` implementation.
///
/// This is passed to `ExclusiveDevice`, because the CS pin is handle in
/// hardware.
pub struct NoCs;

impl embedded_hal::digital::OutputPin for NoCs {
    fn set_low(&mut self) -> Result<(), Self::Error> {
        Ok(())
    }

    fn set_high(&mut self) -> Result<(), Self::Error> {
        Ok(())
    }
}

impl embedded_hal::digital::ErrorType for NoCs {
    type Error = core::convert::Infallible;
}

pub type DISPLAY = Display<
    SpiInterface<
        'static,
        ExclusiveDevice<Spi<'static, SPI1, Blocking>, Output<'static>, Delay>,
        Output<'static>,
    >,
    ST7789,
    NoResetPin,
>;

pub struct MediaUi {
    display: DISPLAY,
    backlight: Output<'static>,
    pub paused: bool,
    pub song: String<SONG_NAME_LEN>,
    pub volume: u8,
}

impl MediaUi {
    const VOLUME: u8 = 0;
    const SONG: u8 = 2;
    const MEDIA_CONTROLS: u8 = 3;

    // Text
    const char_w: u8 = 10;
    const char_h: u8 = 20;

    pub fn new(mut display: DISPLAY, backlight: Output<'static>) -> Self {
        display.clear(Rgb565::WHITE).unwrap();
        Self {
            display,
            backlight,
            paused: true,
            song: String::from_str("Not Playing").unwrap(),
            volume: 100,
        }
    }

    pub fn sleep(&mut self) {
        self.backlight.set_low();
        self.display.clear(Rgb565::BLACK).unwrap();
        self.display.sleep(&mut Delay).unwrap();
    }

    pub fn wake(&mut self) {
        self.backlight.set_high();
        self.display.clear(Rgb565::WHITE).unwrap();
    }

    pub fn destroy(self) -> (DISPLAY, Output<'static>) {
        (self.display, self.backlight)
    }

    pub fn center_str(&mut self, text: &str) {
        if let Err(e) = Text::new(
            text,
            Point::new(100, 100),
            MonoTextStyle::new(&FONT_10X20, Rgb565::RED),
        )
        .draw(&mut self.display)
        {
            error!("Could not write to display",)
        }
    }

    // fn center_int(&self, num: u8) -> u8 {
    //     let (width, _height) = self.display.dimensions();
    //     let width = width / 8;

    //     if num < 10 {
    //         (width - num as u8) / 2
    //     } else if num < 100 {
    //         (width - 2 as u8) / 2
    //     } else {
    //         (width - 3 as u8) / 2
    //     }
    // }

    // fn center_width(&self, item_width: u8) -> u8 {
    //     let (width, _height) = self.display.dimensions();
    //     let width = width / 8;

    //     (width - item_width) / 2
    // }

    // pub fn draw(&mut self) {
    //     self.display
    //         .set_position(self.center_int(self.volume), Self::VOLUME)
    //         .unwrap();
    //     let vol = [self.volume];
    //     self.display
    //         .write_str(unsafe { core::str::from_utf8_unchecked(&vol) })
    //         .unwrap();
    //     self.display.write_str("%").unwrap();

    //     self.display
    //         .set_position(self.center_str(&self.song), Self::SONG)
    //         .unwrap();
    //     self.display.write_str(&self.song).unwrap();

    //     self.display
    //         .set_position(self.center_width(5), Self::MEDIA_CONTROLS)
    //         .unwrap();
    //     self.display.write_str("B P N").unwrap();
    // }
}
