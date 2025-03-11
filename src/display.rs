use core::str::FromStr;
use defmt::*;
use embassy_rp::{
    gpio::Output,
    peripherals::{PIN_10, PIN_11, SPI1},
    spi::{self, Blocking, Spi},
};
use embassy_time::Delay;
use embedded_graphics::{
    draw_target::DrawTarget,
    mono_font::{MonoTextStyle, ascii::FONT_10X20},
    pixelcolor::Rgb565,
    prelude::{Point, RgbColor, *},
    primitives::{Line, PrimitiveStyle, Rectangle, Triangle},
    text::Text,
};
use embedded_hal_bus::spi::ExclusiveDevice;
use format_no_std::*;
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
    const STATUS_BAR: i32 = 40;
    const SPEAKER: Point = Point::new(40, Self::STATUS_BAR);
    const VOLUME: Point = Point::new(60, Self::STATUS_BAR + 10);
    const BATTERY: Point = Point::new(Display::W - 65, Self::STATUS_BAR);
    const SONG_ROW: i32 = 100;
    const PLAYED_ROW: i32 = 125;
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

    pub fn init(&mut self) {
        self.draw_speaker();
        self.draw_volume(99);
        self.draw_battery(100);
        self.draw_song("Truth Hurts - Sawyer Bristol");
        self.draw_played(0);
    }

    fn draw_speaker(&mut self) {
        let style = PrimitiveStyle::with_fill(Rgb565::BLACK);
        Rectangle::new(
            Point::new(Self::SPEAKER.x, Self::SPEAKER.y),
            Size::new(6, 10),
        )
        .into_styled(style)
        .draw(&mut self.display.display)
        .unwrap();
        Triangle::new(
            Point::new(Self::SPEAKER.x, Self::SPEAKER.y + 5),
            Point::new(Self::SPEAKER.x + 10, Self::SPEAKER.y - 5),
            Point::new(Self::SPEAKER.x + 10, Self::SPEAKER.y + 15),
        )
        .into_styled(style)
        .draw(&mut self.display.display)
        .unwrap();
    }

    fn draw_volume(&mut self, volume: u8) {
        let style = MonoTextStyle::new(&FONT_10X20, Rgb565::BLACK);
        let mut buf = [0u8; 3];

        Text::new("%", Self::VOLUME, style)
            .draw(&mut self.display.display)
            .unwrap();
        Text::new(
            show(&mut buf, format_args!("{}", volume)).unwrap(),
            Point::new(Self::VOLUME.x + 12, Self::VOLUME.y),
            style,
        )
        .draw(&mut self.display.display)
        .unwrap();
    }

    fn draw_battery(&mut self, battery: u8) {
        let color = match battery {
            0..=20 => Rgb565::RED,
            21..=30 => Rgb565::new(255, 165, 0),
            91..=100 => Rgb565::GREEN,
            _ => Rgb565::BLACK,
        };
        Rectangle::new(
            Point::new(Self::BATTERY.x, Self::BATTERY.y),
            Size::new(30, 15),
        )
        .into_styled(PrimitiveStyle::with_stroke(color, 3))
        .draw(&mut self.display.display)
        .unwrap();
        Rectangle::new(
            Point::new(Self::BATTERY.x + 30, Self::BATTERY.y + 4),
            Size::new(3, 7),
        )
        .into_styled(PrimitiveStyle::with_fill(color))
        .draw(&mut self.display.display)
        .unwrap();
        let size = if battery > 90 { 30 } else { battery as u32 / 3 };
        Rectangle::new(
            Point::new(Self::BATTERY.x, Self::BATTERY.y),
            Size::new(size, 15),
        )
        .into_styled(PrimitiveStyle::with_fill(color))
        .draw(&mut self.display.display)
        .unwrap();
    }

    fn draw_song(&mut self, song: &str) {
        let style = MonoTextStyle::new(&FONT_10X20, Rgb565::BLACK);
        Text::new(
            song,
            Point::new((Display::W - (song.len() as i32 * 10)) / 2, Self::SONG_ROW),
            style,
        )
        .draw(&mut self.display.display)
        .unwrap();
    }

    pub fn draw_played(&mut self, played: u8) {
        Rectangle::new(
            Point::new(0, Self::PLAYED_ROW - 7),
            Size::new(Display::W as u32, 15),
        )
        .into_styled(PrimitiveStyle::with_fill(Rgb565::WHITE))
        .draw(&mut self.display.display)
        .unwrap();

        Line::new(
            Point::new(20, Self::PLAYED_ROW),
            Point::new(Display::W - 20, Self::PLAYED_ROW),
        )
        .into_styled(PrimitiveStyle::with_stroke(Rgb565::BLACK, 4))
        .draw(&mut self.display.display)
        .unwrap();
        Rectangle::new(
            Point::new(
                ((played as f32 / 100 as f32) * (Display::W - 40) as f32) as i32 + 20,
                Self::PLAYED_ROW - 7,
            ),
            Size::new(15, 15),
        )
        .into_styled(PrimitiveStyle::with_fill(Rgb565::BLACK))
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
        config.frequency = 16_000_000;
        let spi = Spi::new_blocking_txonly(spi, clk, mosi, config);
        let spi_dev = ExclusiveDevice::new(spi, cs, Delay).unwrap();
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
