#![no_std]
#![no_main]

use core::default::Default;
use core::mem;
use defmt::*;
use display::{Display, MediaUi};
use embassy_executor::Spawner;
use embassy_futures::join::join;
use embassy_rp::bind_interrupts;
use embassy_rp::gpio::{Level, Output};
use embassy_rp::peripherals::{DMA_CH2, I2C0, I2C1, PIN_2, PIN_3, PIN_4, PIN_5, PIO0, PIO1, SPI0};
use embassy_rp::pio::{self, Pio};
use embassy_rp::pio_programs::i2s::{PioI2sOut, PioI2sOutProgram};
use embassy_rp::spi::{self, Spi};
use embassy_time::{Duration, Instant, Timer};
use embedded_sdmmc_async::{Controller, SdMmcSpi};
use file_reader::FileReader;
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
    let p = embassy_rp::init(Default::default());
    info!("Clock: {}", embassy_rp::clocks::clk_sys_freq());

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
    const BUFFER_SIZE: usize = 960;

    // Wait for sdcard
    let block_device = {
        loop {
            if let Ok(dev) = sd_card.acquire().await {
                break dev;
            }
            warn!("Could not init Sd card");
            Timer::after_millis(500).await;
        }
    };

    let timesource = file_reader::DummyTimeSource {};
    let mut sd_controller = Controller::new(block_device, timesource);

    loop {
        let mut file_reader: FileReader<'_, Spi<'_, SPI0, spi::Async>, Output<'_>, BUFFER_SIZE> =
            FileReader::new(sd_controller, "test.wav");
        file_reader.open().await;

        let sample_rate = file_reader.sample_rate();
        let bit_depth = file_reader.bit_depth();
        // let end = file_reader.end();
        let bit_rate = bit_depth as u32 * sample_rate;

        // create two audio buffers (back and front) which will take turns being
        // filled with new audio data and being sent to the pio fifo using dma
        // *2 is buffer swapping not stereo
        static DMA_BUFFER: StaticCell<[u32; BUFFER_SIZE * 2]> = StaticCell::new();
        let dma_buffer = DMA_BUFFER.init_with(|| [0u32; BUFFER_SIZE * 2]);
        let (mut back_buffer, mut front_buffer) = dma_buffer.split_at_mut(BUFFER_SIZE);

        loop {
            if file_reader.read() == file_reader.end() {
                info!("Reached end of audio file");
                break;
            }

            let start = Instant::now();

            join(
                // write to back
                async {
                    let mut read_buf = [0u8; BUFFER_SIZE];

                    // read a frame of audio data from the sd card
                    file_reader.read_exact(&mut read_buf).await;

                    // decode if necisary
                    // ...

                    // convert 8bit to 24bit
                    convert_8bit_to_24bit_packed(&read_buf, &mut back_buffer);
                },
                // read from front
                async {
                    // trigger transfer of front buffer data to the pio fifo
                    // but don't await the returned future, yet
                    let dma_future = i2s.write(front_buffer);

                    // now await the dma future. once the dma finishes, the next buffer needs to be queued
                    dma_future.await;
                },
            )
            .await;

            mem::swap(&mut back_buffer, &mut front_buffer);

            // // Synchronize the timing with the sample rate (e.g., 48kHz, 44.1kHz)
            // // Add a small delay to ensure the next buffer is ready at the right time.
            // Timer::after(Instant::now().duration_since(start) - Duration::from_hz(bit_rate.into()))
            //     .await;
        }

        // Close Audio File and get sd controller back
        sd_controller = file_reader.close();
    }
}

fn convert_8bit_to_24bit_packed(read_buf: &[u8], buffer: &mut [u32]) {
    // Ensure we have enough space in the output buffer
    defmt::assert!(buffer.len() >= read_buf.len());

    // Convert 8-bit audio to 24-bit packed into 32-bit words
    for (i, &sample) in read_buf.iter().enumerate() {
        // Pack the 8-bit sample into the lower 24 bits of a 32-bit word
        buffer[i] = (sample as u32) << 24; // Shift 8-bit sample to 24-bit space
    }
}
