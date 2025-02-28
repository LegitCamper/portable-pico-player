use core::str::FromStr;
use defmt::*;
use embassy_rp::{
    gpio::{Drive, Output},
    peripherals::{PIN_10, PIN_11, SPI1},
    spi::{self, Blocking, Spi},
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
use mipidsi::{
    Display as MipiDisplay, NoResetPin,
    interface::SpiInterface,
    models::ST7789,
    options::{ColorInversion, Orientation},
};

pub const SONG_NAME_LEN: usize = 25;

pub struct MediaUi<'a> {
    display: Display<'a>,
    pub paused: bool,
    pub song: String<SONG_NAME_LEN>,
    pub volume: u8,
}

impl<'a> MediaUi<'a> {
    pub fn new(mut display: Display<'a>) -> Self {
        display.display.clear(Rgb565::WHITE).unwrap();
        Self {
            paused: true,
            song: String::from_str("Not Playing").unwrap(),
            volume: 100,
            display,
        }
    }

    pub fn sleep(&mut self) {
        info!("[Display] not being used, going to sleep");
        self.display.sleep();
    }

    pub fn wake(&mut self) {
        self.display.wake();
    }

    pub fn deep_sleep(&mut self) {
        info!("[Display] entering deep sleep");
        self.display.deep_sleep();
    }

    pub fn wake_deep(&mut self) {
        self.display.wake_deep();
    }

    pub fn center_str(&mut self, text: &str) {
        Text::new(
            text,
            Point::new(100, 100),
            MonoTextStyle::new(&FONT_10X20, Rgb565::BLACK),
        )
        .draw(&mut self.display.display)
        .unwrap();
    }
}

pub type DISPLAY<'a> = MipiDisplay<
    SpiInterface<
        'a,
        ExclusiveDevice<Spi<'static, SPI1, Blocking>, Output<'static>, Delay>,
        Output<'static>,
    >,
    ST7789,
    NoResetPin,
>;

pub struct Display<'a> {
    pwr: Output<'static>,
    display: DISPLAY<'a>,
}

impl<'a> Display<'a> {
    // ST7789 TFT Display diamentions
    pub const W: i32 = 320;
    pub const H: i32 = 240;

    pub fn new(
        mut pwr: Output<'static>,
        spi: SPI1,
        clk: PIN_10,
        mosi: PIN_11,
        dc: Output<'static>,
        cs: Output<'static>,
        buffer: &'a mut [u8],
    ) -> Self {
        pwr.set_slew_rate(embassy_rp::gpio::SlewRate::Fast);
        pwr.set_high();
        let mut config = spi::Config::default();
        config.frequency = 2_000_000;
        let spi = Spi::new_blocking_txonly(spi, clk, mosi, config);
        let spi_dev = ExclusiveDevice::new(spi, cs, Delay);
        let interface = SpiInterface::new(spi_dev, dc, buffer);
        let orientation = Orientation::new().rotate(mipidsi::options::Rotation::Deg90);
        let display = mipidsi::Builder::new(ST7789, interface)
            .orientation(orientation)
            .display_size(Self::H as u16, Self::W as u16)
            .invert_colors(ColorInversion::Inverted)
            .init(&mut Delay)
            .unwrap();
        Self { pwr, display }
    }

    fn wake(&mut self) {
        self.display.wake(&mut Delay).unwrap()
    }

    fn sleep(&mut self) {
        self.display.sleep(&mut Delay).unwrap()
    }

    fn wake_deep(&mut self) {
        self.pwr.set_high();
    }

    fn deep_sleep(&mut self) {
        self.pwr.set_low();
    }
}

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
