#![no_std]
#![no_main]

use core::mem;

#[cfg(feature = "bluetooth")]
use bt_hci::controller::ExternalController;
#[cfg(feature = "bluetooth")]
use embassy_futures::select::select;
use embassy_rp::pio_programs::i2s::{PioI2sOut, PioI2sOutProgram};
use embedded_hal::delay::DelayNs;
use embedded_sdmmc::{BlockDevice, Mode, VolumeIdx, VolumeManager};
use mipidsi::interface::SpiInterface;
use mipidsi::models::ST7789;
use mipidsi::options::{ColorInversion, Orientation};
#[cfg(feature = "bluetooth")]
use trouble_host::{Address, Host, HostResources};

use cyw43_pio::PioSpi;
use defmt::*;
use display::{Display, MediaUi};
use embassy_executor::{Executor, Spawner};
use embassy_rp::bind_interrupts;
use embassy_rp::gpio::{Level, Output};
use embassy_rp::i2c;
use embassy_rp::multicore::{Stack, spawn_core1};
use embassy_rp::peripherals::{DMA_CH2, I2C0, I2C1, PIN_2, PIN_3, PIN_4, PIN_5, PIO0, PIO1, SPI0};
use embassy_rp::pio::{self, Pio};
use embassy_rp::spi::{self, Spi};
use embassy_sync::blocking_mutex::raw::CriticalSectionRawMutex;
use embassy_sync::channel::Channel;
use embassy_time::{Delay, Duration, Timer};
use embedded_hal::delay::DelayNs;
use embedded_hal_bus::spi::{ExclusiveDevice, NoDelay};
use mipidsi::interface::SpiInterface;
use mipidsi::models::ST7789;
use mipidsi::options::{ColorInversion, Orientation};
use static_cell::StaticCell;
use wavv::{DataBulk, Wav};
use {defmt_rtt as _, panic_probe as _};

#[cfg(feature = "bluetooth")]
mod ble;
mod display;
mod storage;

bind_interrupts!(struct Irqs {
    // i2s
    PIO0_IRQ_0 => pio::InterruptHandler<PIO0>;
    // bluetooth
    PIO1_IRQ_0 => pio::InterruptHandler<PIO1>;
});

// #[cortex_m_rt::entry]
#[embassy_executor::main]
async fn main(_spawner: Spawner) {
    let mut p = embassy_rp::init(Default::default());

    // // Sets up Bluetooth and Trouble
    // let (spi, pwr) = {
    //     let pwr = Output::new(p.PIN_23, Level::Low);
    //     let cs = Output::new(p.PIN_25, Level::High);
    //     let mut pio = Pio::new(p.PIO1, Irqs);
    //     let spi = PioSpi::new(
    //         &mut pio.common,
    //         pio.sm0,
    //         cyw43_pio::DEFAULT_CLOCK_DIVIDER,
    //         pio.irq0,
    //         cs,
    //         p.PIN_24,
    //         p.PIN_29,
    //         p.DMA_CH2,
    //     );
    //     (spi, pwr)
    // };

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

    // loop {
    //     let file = music_dir
    //         .open_file_in_dir("test.wav", Mode::ReadOnly)
    //         .unwrap();
    //     let mut wav = Wav::new(file).unwrap();

    // // spawning ui and io task
    // spawn_core1(
    //     p.CORE1,
    //     unsafe { &mut *core::ptr::addr_of_mut!(CORE1_STACK) },
    //     move || {
    //         let executor1 = EXECUTOR1.init(Executor::new());
    //         executor1
    //             .run(|spawner| unwrap!(spawner.spawn(core1_task(display, backlight, pwr, spi))));
    //     },
    // );

    // Set up SPI0 for the Micro SD reader
    let sdcard = {
        let mut config = spi::Config::default();
        config.frequency = 400_000;
        let spi = spi::Spi::new_blocking(p.SPI0, p.PIN_2, p.PIN_3, p.PIN_4, config);
        let cs = Output::new(p.PIN_5, Level::High);

        let sdcard = SdCard::new(spi, cs, embassy_time::Delay);
        // Now that the card is initialized, the SPI clock can go faster
        let mut config = spi::Config::default();
        config.frequency = 16_000_000;
        sdcard.spi(|dev| dev.bus_mut().set_config(&config));
        sdcard
    };

    // while let Err(_) = sdcard.num_bytes() {
    //     info!("Sdcard not found, looking again in 1 second");
    //     Timer::after_secs(1).await;
    // }
    // info!("sd size:{}", sdcard.num_bytes().unwrap());

    // let mut volume_mgr = VolumeManager::new(sdcard, storage::DummyTimesource());
    // let mut volume0 = volume_mgr.open_volume(VolumeIdx(0)).unwrap();
    // let mut root_dir = volume0.open_root_dir().unwrap();
    // let mut music_dir = root_dir.open_dir("music").unwrap();
    // info!("reading test file");

    // storage::read_wav(&mut music_dir, "test.wav", async |mut wav| {
    //     let samples = wav.next_n::<10>().unwrap();
    //     // write samples to bt and/or dac
    // })
    // .await;

    // // spawning compute task
    // let executor0 = EXECUTOR0.init(Executor::new());
    // executor0.run(|spawner| unwrap!(spawner.spawn(core0_task(sdcard))));

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

        // start pio state machine
        let mut fade_value: i32 = 0;
        let mut phase: i32 = 0;

        loop {
            // trigger transfer of front buffer data to the pio fifo
            // but don't await the returned future, yet
            let dma_future = i2s.write(front_buffer);

            // fade in audio when bootsel is pressed
            let fade_target = if p.BOOTSEL.is_pressed() { i32::MAX } else { 0 };

            // fill back buffer with fresh audio samples before awaiting the dma future
            for s in back_buffer.iter_mut() {
                // exponential approach of fade_value => fade_target
                fade_value += (fade_target - fade_value) >> 14;
                // generate triangle wave with amplitude and frequency based on fade value
                phase = (phase + (fade_value >> 22)) & 0xffff;
                let triangle_sample = (phase as i16 as i32).abs() - 16384;
                let sample = (triangle_sample * (fade_value >> 15)) >> 16;
                // duplicate mono sample into lower and upper half of dma word
                *s = (sample as u16 as u32) * 0x10001;
            }

            // now await the dma future. once the dma finishes, the next buffer needs to be queued
            // within DMA_DEPTH / SAMPLE_RATE = 8 / 48000 seconds = 166us
            dma_future.await;
            mem::swap(&mut back_buffer, &mut front_buffer);
        }
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

//     // Configure bluetooth
//     #[cfg(feature = "bluetooth")]
//     {
//         // Release:
//         #[cfg(not(debug_assertions))]
//         let (fw, clm, btfw) = (
//             include_bytes!("../cyw43-firmware/43439A0.bin"),
//             include_bytes!("../cyw43-firmware/43439A0_clm.bin"),
//             include_bytes!("../cyw43-firmware/43439A0_btfw.bin"),
//         );

//         // Dev
//         #[cfg(debug_assertions)]
//         let (fw, clm, btfw) = (
//             unsafe { core::slice::from_raw_parts(0x10100000 as *const u8, 224190) },
//             unsafe { core::slice::from_raw_parts(0x10140000 as *const u8, 4752) },
//             unsafe { core::slice::from_raw_parts(0x10141400 as *const u8, 6164) },
//         );

//         static STATE: StaticCell<cyw43::State> = StaticCell::new();
//         let state = STATE.init(cyw43::State::new());
//         let (_net_device, bt_device, mut control, runner) =
//             cyw43::new_with_bluetooth(state, pwr, spi, fw, btfw).await;
//         control.init(clm).await;
//         let controller: ble::ControllerT = ExternalController::new(bt_device);

//         let address: Address = Address::random([0xff, 0x8f, 0x1b, 0x05, 0xe4, 0xff]);
//         static RESOURCES: StaticCell<ble::Resources> = StaticCell::new();
//         let stack = trouble_host::new(controller, RESOURCES.init(HostResources::new()))
//             .set_random_address(address);
//         let Host {
//             central,
//             mut runner,
//             peripheral,
//             ..
//         } = stack.build();

//         select(
//             runner.run(),
//             ble::run(&mut runner, central, peripheral).await,
//         )
//         .await;
//     }

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
