#![no_std]
#![no_main]
#![feature(inherent_str_constructors)]

use core::default::Default;
use core::mem;
use defmt::*;
use display::{Display, MediaUi};
use embassy_executor::Spawner;
use embassy_futures::join::join;
use embassy_futures::select::select;
use embassy_rp::bind_interrupts;
use embassy_rp::gpio::{Level, Output};
use embassy_rp::peripherals::{DMA_CH2, I2C0, I2C1, PIN_2, PIN_3, PIN_4, PIN_5, PIO0, PIO1, SPI0};
use embassy_rp::pio::{self, Pio};
use embassy_rp::pio_programs::i2s::{PioI2sOut, PioI2sOutProgram};
use embassy_rp::spi::{self, Spi};
use embassy_time::{Duration, Instant, Timer, with_timeout};
use embedded_hal_bus::spi::ExclusiveDevice;
use embedded_sdmmc::asynchronous::{BlockDevice, SdCard, VolumeIdx, VolumeManager};
use heapless::{String, Vec};
use static_cell::StaticCell;
use {defmt_rtt as _, panic_probe as _};

// mod ble;
mod display;
mod file_reader;
use file_reader::{DummyTimeSource, Library};

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
    let sdcard_block_device = {
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

        let device = ExclusiveDevice::new(spi, cs, embassy_time::Delay).unwrap();
        device
    };

    // i2s DAC
    let i2s = {
        const SAMPLE_RATE: u32 = 8_000;
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
        PioI2sOut::new(
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
        )
    };
    unwrap!(spawner.spawn(reader(sdcard_block_device, i2s)))
}

const BUFFER_SIZE: usize = 1024;

#[embassy_executor::task]
async fn reader(
    mut block_device: ExclusiveDevice<
        Spi<'static, SPI0, spi::Async>,
        Output<'static>,
        embassy_time::Delay,
    >,
    mut _i2s: PioI2sOut<'static, PIO0, 0>,
) {
    let sdcard = SdCard::new(block_device, embassy_time::Delay);
    let volume_mgr = VolumeManager::new(sdcard, DummyTimeSource {});

    // Wait for sdcard
    let volume = {
        loop {
            if let Ok(vol) = volume_mgr.open_volume(VolumeIdx(0)).await {
                break vol;
            }
            warn!("Could not init Sd card");
            Timer::after_millis(500).await;
        }
    };

    let mut library: Library = Library::new(volume);
    library.discover_music().await;
    info!("music: {:?}", library.artists());

    // create two audio buffers (back and front) which will take turns being
    // filled with new audio data and being sent to the pio fifo using dma
    // *2 is buffer swapping not stereo
    static DMA_BUFFER: StaticCell<[u32; BUFFER_SIZE * 2]> = StaticCell::new();
    let dma_buffer = DMA_BUFFER.init_with(|| [0u32; BUFFER_SIZE * 2]);
    let (mut back_buffer, mut front_buffer) = dma_buffer.split_at_mut(BUFFER_SIZE);

    loop {
        let sample_rate = library.sample_rate();
        let bit_depth = library.bit_depth();

        // Calculate the timeout as the time to fill the buffer (in seconds)
        let timeout_secs_f64 = BUFFER_SIZE as f64 / sample_rate as f64;
        let timeout_millis = (timeout_secs_f64 * 1000.0) as u64; // Convert seconds to milliseconds
        let timeout = Duration::from_millis(timeout_millis);

        fill_back(&mut file_reader, &mut front_buffer).await;
        loop {
            let start = Instant::now();
            if file_reader.read() >= file_reader.end() {
                info!("Reached end of audio file");
                break;
            }

            // Read the next chunk of data into the back buffer asynchronously while sending front buffer.
            let back_buffer_fut = async {
                if let Err(_) =
                    with_timeout(timeout, fill_back(&mut file_reader, &mut back_buffer)).await
                {
                    info!("Filling with silence due to timeout.");
                    // Fill with silence bc reading took too long
                    back_buffer.fill(0);
                }
            };

            // Write the front buffer data to the i2s DMA while the back buffer is being filled.
            let dma_future = i2s.write(front_buffer);

            // Execute the two tasks concurrently.
            join(back_buffer_fut, dma_future).await;

            // Synchronize the timing with the sample rate (e.g., 48kHz, 44.1kHz)
            // Calculate the time elapsed since starting this loop
            let elapsed = Instant::now().duration_since(start);

            // Calculate the time needed to fill the buffer based on sample rate and buffer size
            let expected_fill_time =
                Duration::from_millis((BUFFER_SIZE * 1000) as u64 / sample_rate as u64);

            // Adjust timing for any delays that have already occurred
            let delay_duration = if elapsed < expected_fill_time {
                expected_fill_time - elapsed
            } else {
                Duration::from_millis(0) // If we're behind, don't delay further
            };

            // Wait for the next buffer to be ready
            Timer::after(delay_duration).await;

            mem::swap(&mut back_buffer, &mut front_buffer);
        }

        // Close Audio File and get sd controller back
        sd_controller = file_reader.close();
    }
}

pub async fn fill_back(
    file_reader: &mut FileReader<'_, Spi<'_, SPI0, spi::Async>, Output<'_>, BUFFER_SIZE>,
    back_buffer: &mut [u32],
) {
    let mut read_buf = [0u8; BUFFER_SIZE];

    // read a frame of audio data from the sd card
    file_reader.read_exact(&mut read_buf).await;

    // decode if necisary
    // ...

    // convert 8bit to 24bit and make it stereo
    back_buffer
        .iter_mut()
        .zip(read_buf)
        .for_each(|(dma, read)| {
            *dma = (read as u32) << 16 | read as u32;
        });
}
