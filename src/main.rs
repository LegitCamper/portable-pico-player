#![no_std]
#![no_main]

#[global_allocator]
static ALLOCATOR: emballoc::Allocator<4096> = emballoc::Allocator::new();

extern crate alloc;

use bt_hci::controller::ExternalController;
use core::f32::consts::PI;
use cyw43_pio::PioSpi;
use defmt::*;
use embassy_executor::Spawner;
use embassy_futures::{join::join, select::select};
use embassy_rp::gpio::{Level, Output};
use embassy_rp::i2c;
use embassy_rp::peripherals::{DMA_CH0, I2C0, I2C1, PIN_14, PIO0, PWM_SLICE2, PWM_SLICE7};
use embassy_rp::pio::{self, Pio};
use embassy_rp::pwm::{Pwm, SetDutyCycle};
use embassy_rp::spi;
use embassy_rp::{bind_interrupts, pwm};
use embassy_sync::blocking_mutex::raw::NoopRawMutex;
use embassy_time::Timer;
use embedded_hal_bus::spi::ExclusiveDevice;
use embedded_sdmmc::sdcard::{DummyCsPin, SdCard};
use heapless::Vec;
use libm::sinf;
use static_cell::StaticCell;
// use trouble_host::{HostResources, prelude::*};
use {defmt_rtt as _, embassy_time as _, panic_probe as _};

mod aux;
// mod ble;
mod display;
use display::display_task;
mod storage;

bind_interrupts!(struct Irqs {
    PIO0_IRQ_0 => pio::InterruptHandler<PIO0>;
    I2C0_IRQ => i2c::InterruptHandler<I2C0>;
});

#[embassy_executor::task]
async fn cyw43_task(
    runner: cyw43::Runner<'static, Output<'static>, PioSpi<'static, PIO0, 0, DMA_CH0>>,
) -> ! {
    runner.run().await
}

#[embassy_executor::main]
async fn main(spawner: Spawner) {
    embassy_rp::pac::SIO.spinlock(31).write_value(1);
    let p = embassy_rp::init(Default::default());

    // Set up I2C0 for the SSD1306 OLED Display
    let i2c0 = i2c::I2c::new_async(p.I2C0, p.PIN_1, p.PIN_0, Irqs, i2c::Config::default());
    unwrap!(spawner.spawn(display_task(i2c0)));

    // Set up SPI0 for the Micro SD reader
    let library = {
        let mut config = spi::Config::default();
        config.frequency = 400_000;
        let spi = spi::Spi::new_blocking(p.SPI0, p.PIN_2, p.PIN_3, p.PIN_4, config);
        // Use a dummy cs pin here, for embedded-hal SpiDevice compatibility reasons
        let spi_dev = ExclusiveDevice::new_no_delay(spi, DummyCsPin);
        // Real cs pin
        let cs = Output::new(p.PIN_5, Level::High);

        let sdcard = SdCard::new(spi_dev, cs, embassy_time::Delay);
        info!("Card size is {} bytes", sdcard.num_bytes().unwrap());

        // Now that the card is initialized, the SPI clock can go faster
        let mut config = spi::Config::default();
        config.frequency = 16_000_000;
        sdcard.spi(|dev| dev.bus_mut().set_config(&config));

        // unwrap!(spawner.spawn(storage_task(sdcard)));
        storage::Library::new(sdcard)
    };

    unwrap!(spawner.spawn(aux::run(p.PWM_SLICE7, p.PIN_14, library)));

    // Sets up Bluetooth and Trouble
    // {
    //     // Release:
    //     #[cfg(not(debug_assertions))]
    //     let fw = include_bytes!("../cyw43-firmware/43439A0.bin");
    //     #[cfg(not(debug_assertions))]
    //     let clm = include_bytes!("../cyw43-firmware/43439A0_clm.bin");
    //     #[cfg(not(debug_assertions))]
    //     let btfw = include_bytes!("../cyw43-firmware/43439A0_btfw.bin");

    //     // Dev
    //     #[cfg(debug_assertions)]
    //     let fw = unsafe { core::slice::from_raw_parts(0x10100000 as *const u8, 224190) };
    //     #[cfg(debug_assertions)]
    //     let clm = unsafe { core::slice::from_raw_parts(0x10140000 as *const u8, 4752) };
    //     #[cfg(debug_assertions)]
    //     let btfw = unsafe { core::slice::from_raw_parts(0x10141400 as *const u8, 6164) };

    //     let pwr = Output::new(p.PIN_23, Level::Low);
    //     let cs = Output::new(p.PIN_25, Level::High);
    //     let mut pio = Pio::new(p.PIO0, Irqs);
    //     let spi = PioSpi::new(
    //         &mut pio.common,
    //         pio.sm0,
    //         cyw43_pio::DEFAULT_CLOCK_DIVIDER,
    //         pio.irq0,
    //         cs,
    //         p.PIN_24,
    //         p.PIN_29,
    //         p.DMA_CH0,
    //     );

    //     static STATE: StaticCell<cyw43::State> = StaticCell::new();
    //     let state = STATE.init(cyw43::State::new());
    //     let (_net_device, bt_device, mut control, runner) =
    //         cyw43::new_with_bluetooth(state, pwr, spi, fw, btfw).await;
    //     unwrap!(spawner.spawn(cyw43_task(runner)));
    //     control.init(clm).await;
    //     let controller: ble::ControllerT = ExternalController::new(bt_device);

    //     let address: Address = Address::random([0xff, 0x8f, 0x1b, 0x05, 0xe4, 0xff]);
    //     static RESOURCES: StaticCell<ble::Resources> = StaticCell::new();
    //     let stack = trouble_host::new(controller, RESOURCES.init(HostResources::new()))
    //         .set_random_address(address);
    //     let Host {
    //         central,
    //         mut runner,
    //         peripheral,
    //         ..
    //     } = stack.build();

    //     ble::run(&mut runner, central, peripheral).await;
    // }
}
