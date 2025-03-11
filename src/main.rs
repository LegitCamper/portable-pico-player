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
use wavv::{DataBulk, Wav};
use {defmt_rtt as _, panic_probe as _};

// mod ble;
mod display;
mod file_reader;

const AUDIO_FRAME_BYTES_LEN: usize = NUM_SAMPLES * mem::size_of::<Sample>();
const BB_BYTES_LEN: usize = AUDIO_FRAME_BYTES_LEN * 6;
const NUM_SAMPLES: usize = 960; // for both left and right channels
const FILE_FRAME_LEN: usize = 400; // containing 2 channels of audio
static BB: BBBuffer<BB_BYTES_LEN> = BBBuffer::new();
type Sample = i16;

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

        let mut sd_card = SdMmcSpi::new(spi, cs);
        sd_card
    };

    // used for sending data between tasks
    let (producer, mut consumer) = BB.try_split().unwrap();

    unwrap!(spawner.spawn(reader(sdcard, producer)));

    // i2s DAC
    {
        const SAMPLE_RATE: u32 = 48_000;
        const BIT_DEPTH: u32 = 16;
        const CHANNELS: u32 = 2;

        // Setup pio state machine for i2s output
        let Pio {
            mut common, sm0, ..
        } = Pio::new(p.PIO0, Irqs);

        let bit_clock_pin = p.PIN_18;
        let left_right_clock_pin = p.PIN_19;
        let data_pin = p.PIN_20;

        let program = PioI2sOutProgram::new(&mut common);
        let mut i2s = PioI2sOut::new(
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

        // create two audio buffers (back and front) which will take turns being
        // filled with new audio data and being sent to the pio fifo using dma
        const BUFFER_SIZE: usize = 960;
        static DMA_BUFFER: StaticCell<[u32; BUFFER_SIZE * 2]> = StaticCell::new();
        let dma_buffer = DMA_BUFFER.init_with(|| [0u32; BUFFER_SIZE * 2]);
        let (mut back_buffer, mut front_buffer) = dma_buffer.split_at_mut(BUFFER_SIZE);

        loop {
            // trigger transfer of front buffer data to the pio fifo
            // but don't await the returned future, yet
            let dma_future = i2s.write(front_buffer);

            // fill back buffer with fresh audio samples before awaiting the dma future
            for s in back_buffer.iter_mut() {
                // lock free read
                match consumer.read() {
                    Ok(read) => *s = u32::from_be_bytes(read.buf().try_into().unwrap()),
                    Err(_) => {
                        // decoding cannot keep up with playback speed - play silence instead
                        info!("silence");
                        *s = 0
                    }
                };
            }

            // now await the dma future. once the dma finishes, the next buffer needs to be queued
            // within DMA_DEPTH / SAMPLE_RATE = 8 / 48000 seconds = 166us
            dma_future.await;
            mem::swap(&mut back_buffer, &mut front_buffer);
        }
    }
}

#[embassy_executor::task]
async fn reader(
    mut sd_card: SdMmcSpi<Spi<'static, SPI0, spi::Async>, Output<'static>>,
    mut producer: bbqueue::Producer<'static, BB_BYTES_LEN>,
) {
    let block_device = sd_card.acquire().await.unwrap();
    let mut file_reader = FileReader::new(block_device, "test.wav");
    file_reader.open().await;

    let mut dec_in_buffer = [0; FILE_FRAME_LEN];
    let mut dec_out_buffer = [0; NUM_SAMPLES / 2];

    loop {
        match producer.grant_exact(AUDIO_FRAME_BYTES_LEN) {
            Ok(mut wgr) => {
                // read a frame of audio data from the sd card
                if !file_reader.read_exact(&mut dec_in_buffer).await {
                    // start reading the file again
                    info!("start reading the file again");
                    continue;
                }

                // set num bytes to be committed (otherwise wgr.buf() may contain the wrong number of bytes)
                wgr.to_commit(AUDIO_FRAME_BYTES_LEN);

                // the pcm buffer (wgr) has L-R audio samples interleved
                encode_to_out_buf(&dec_out_buffer, &mut wgr.buf()[2..]);
            }
            Err(_) => {
                // input queue full, this is normal
                // the i2s interrupt firing should free up space
                Timer::after(Duration::from_micros(1000)).await;
            }
        }
    }
}

fn encode_to_out_buf(decoder_buf: &[i16], pcm_buf: &mut [u8]) {
    // take 2 bytes at a time and skip every second chunk
    // we do this because this buffer is for stereo audio with L-R samples interleved
    for (src, dst) in decoder_buf
        .iter()
        .zip(pcm_buf.chunks_exact_mut(2).step_by(2))
    {
        LittleEndian::write_i16(dst, *src);
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
