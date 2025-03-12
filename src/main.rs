#![no_std]
#![no_main]

use bbqueue::BBBuffer;
use byteorder::{ByteOrder, LittleEndian};
use core::default::Default;
use core::mem;
use cyw43_pio::PioSpi;
use defmt::*;
use display::{Display, MediaUi};
use embassy_executor::{Executor, Spawner};
use embassy_futures::select::select;
use embassy_rp::bind_interrupts;
use embassy_rp::gpio::{Level, Output};
use embassy_rp::i2c;
use embassy_rp::multicore::{Stack, spawn_core1};
use embassy_rp::peripherals::{DMA_CH2, I2C0, I2C1, PIN_2, PIN_3, PIN_4, PIN_5, PIO0, PIO1, SPI0};
use embassy_rp::pio::{self, Pio};
use embassy_rp::pio_programs::i2s::{PioI2sOut, PioI2sOutProgram};
use embassy_rp::spi::{self, Spi};
use embassy_sync::blocking_mutex::raw::CriticalSectionRawMutex;
use embassy_sync::channel::Channel;
use embassy_time::{Delay, Duration, Timer};
use embedded_sdmmc_async::{BlockDevice, Mode, SdMmcSpi, VolumeIdx};
use file_reader::FileReader;
use heapless::Vec;
use mipidsi::interface::SpiInterface;
use mipidsi::models::ST7789;
use mipidsi::options::{ColorInversion, Orientation};
use static_cell::StaticCell;
use {defmt_rtt as _, panic_probe as _};

// mod ble;
mod display;
mod file_reader;

bind_interrupts!(struct Irqs {
    // i2s
    PIO0_IRQ_0 => pio::InterruptHandler<PIO0>;
    // bluetooth
    PIO1_IRQ_0 => pio::InterruptHandler<PIO1>;
});

#[embassy_executor::main]
async fn main(spawner: Spawner) {
    let mut p = embassy_rp::init(Default::default());

    // Set up SPI1 for ST7789 TFT Display
    let mut buffer = [0u8; 4096];
    let display = Display::new(
        Output::new(p.PIN_15, Level::High),
        p.SPI1,
        p.PIN_10,
        p.PIN_11,
        Output::new(p.PIN_13, Level::Low),
        Output::new(p.PIN_14, Level::Low),
        &mut buffer,
    );
    Timer::after_secs(4).await;

    let mut media_ui = MediaUi::new(display);
    media_ui.init();

    // Set up SPI0 for the Micro SD reader
    let sdcard = {
        let mut config = spi::Config::default();
        config.frequency = 16_000_000;
        let spi = spi::Spi::new(
            p.SPI0,
            p.PIN_2,
            p.PIN_3,
            p.PIN_4,
            p.DMA_CH1,
            p.DMA_CH2,
            spi::Config::default(),
        );
        let cs = Output::new(p.PIN_5, Level::High);

        let sd_card = SdMmcSpi::new(spi, cs);
        sd_card
    };

    // i2s DAC
    {
        const SAMPLE_RATE: u32 = 8_000;
        const BIT_DEPTH: u32 = 8;
        const CHANNELS: u32 = 1;

        // Setup pio state machine for i2s output
        let Pio {
            mut common, sm0, ..
        } = Pio::new(p.PIO0, Irqs);

        let bit_clock_pin = p.PIN_18;
        let left_right_clock_pin = p.PIN_19;
        let data_pin = p.PIN_20;

        let program = PioI2sOutProgram::new(&mut common);
        let i2s = PioI2sOut::new(
            &mut common,
            sm0,
            p.DMA_CH0,
            data_pin,
            bit_clock_pin,
            left_right_clock_pin,
            SAMPLE_RATE,
            BIT_DEPTH,
            CHANNELS,
            &program,
        );
        unwrap!(spawner.spawn(reader(sdcard, i2s)))
    }
}

#[embassy_executor::task]
async fn reader(
    mut sd_card: SdMmcSpi<Spi<'static, SPI0, spi::Async>, Output<'static>>,
    mut i2s: PioI2sOut<'static, PIO0, 0>,
) {
    let block_device = {
        loop {
            if let Ok(dev) = sd_card.acquire().await {
                break dev;
            }
            warn!("Could not init Sd card");
            Timer::after_millis(500).await;
        }
    };
    let mut file_reader = FileReader::new(block_device, "test.wav");
    file_reader.open().await;

    // create two audio buffers (back and front) which will take turns being
    // filled with new audio data and being sent to the pio fifo using dma
    const BUFFER_SIZE: usize = 100;
    static DMA_BUFFER: StaticCell<[u32; BUFFER_SIZE]> = StaticCell::new();
    let dma_buffer = DMA_BUFFER.init_with(|| [0u32; BUFFER_SIZE]);
    let (mut back_buffer, mut front_buffer) = dma_buffer.split_at_mut(BUFFER_SIZE);

    loop {
        // trigger transfer of front buffer data to the pio fifo
        // but don't await the returned future, yet
        let dma_future = i2s.write(front_buffer);

        let mut read_buf = [0u8; BUFFER_SIZE / 3];
        // read a frame of audio data from the sd card
        file_reader.read_exact(&mut read_buf).await;

        // decode if necisary
        // ...

        // convert 8bit to 24bit
        for (dma, read) in back_buffer.iter_mut().zip(read_buf) {
            let mut result = 0;
            for i in 0..4 {
                result |= (read as u32) << (i * 8);
            }
            *dma = result
        }

        // now await the dma future. once the dma finishes, the next buffer needs to be queued
        // within DMA_DEPTH / SAMPLE_RATE = 8 / 48000 seconds = 166us
        dma_future.await;
        mem::swap(&mut back_buffer, &mut front_buffer);
    }
}

// // Ui and media playback
// #[embassy_executor::task]
// async fn core1_task(
//     mut display: display::DISPLAY,
//     backlight: Output<'static>,
//     // mut dac: MCP4725<i2c::I2c<'static, I2C1, i2c::Async>>,
//     pwr: Output<'static>,
//     spi: PioSpi<'static, PIO1, 0, DMA_CH2>,
// ) {
//     info!("hello from core 0");

//     let mut media_ui = Display::new(display, backlight);
//     media_ui.center_str("Loading...");
//     Timer::after_secs(2).await;
//     // media_ui.sleep();

//     // info!("playing music");
//     // loop {
//     //     let samples = CHANNEL.receive().await;
//     //     if let DataBulk::BitDepth8(samples) = samples {
//     //         for sample in samples {
//     //             dac.set_dac_fast(PowerDown::Normal, sample.into()).ok();
//     //             Timer::after(Duration::from_hz(8000)).await;
//     //         }
//     //     }
//     // }
// }
