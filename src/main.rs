#![no_std]
#![no_main]

use bt_hci::controller::ExternalController;
use cyw43::bluetooth::BtDriver;
use cyw43_pio::PioSpi;
use defmt::*;
use embassy_executor::Spawner;
use embassy_rp::bind_interrupts;
use embassy_rp::gpio::{Level, Output};
use embassy_rp::peripherals::{DMA_CH0, PIO0};
use embassy_rp::pio::{InterruptHandler, Pio};
use static_cell::StaticCell;
use trouble_host::{HostResources, PacketQos};
use {defmt_rtt as _, embassy_time as _, panic_probe as _};

mod ble_bas_central;

bind_interrupts!(struct Irqs {
    PIO0_IRQ_0 => InterruptHandler<PIO0>;
});

/// Size of L2CAP packets
const L2CAP_MTU: usize = 128;

/// Max number of connections
const CONNECTIONS_MAX: usize = 1;

/// Max number of L2CAP channels.
const L2CAP_CHANNELS_MAX: usize = 3; // Signal + att + CoC

type Controller = ExternalController<BtDriver<'static>, 10>;
type Resources<C> = HostResources<C, CONNECTIONS_MAX, L2CAP_CHANNELS_MAX, L2CAP_MTU>;

#[embassy_executor::task]
async fn bt_task(mut runner: trouble_host::prelude::Runner<'static, Controller>) {
    runner.run().await.unwrap();
}

#[embassy_executor::task]
async fn cyw43_task(
    runner: cyw43::Runner<'static, Output<'static>, PioSpi<'static, PIO0, 0, DMA_CH0>>,
) -> ! {
    runner.run().await
}

#[embassy_executor::main]
async fn main(spawner: Spawner) {
    let p = embassy_rp::init(Default::default());

    // Release:
    // let fw = include_bytes!("../cyw43-firmware/43439A0.bin");
    // let clm = include_bytes!("../cyw43-firmware/43439A0_clm.bin");
    // let btfw = include_bytes!("../cyw43-firmware/43439A0_btfw.bin");

    // Dev
    let fw = unsafe { core::slice::from_raw_parts(0x10100000 as *const u8, 224190) };
    let clm = unsafe { core::slice::from_raw_parts(0x10140000 as *const u8, 4752) };
    let btfw = unsafe { core::slice::from_raw_parts(0x10141400 as *const u8, 6164) };

    let pwr = Output::new(p.PIN_23, Level::Low);
    let cs = Output::new(p.PIN_25, Level::High);
    let mut pio = Pio::new(p.PIO0, Irqs);
    let spi = PioSpi::new(
        &mut pio.common,
        pio.sm0,
        pio.irq0,
        cs,
        p.PIN_24,
        p.PIN_29,
        p.DMA_CH0,
    );

    static STATE: StaticCell<cyw43::State> = StaticCell::new();
    let state = STATE.init(cyw43::State::new());
    let (_net_device, bt_device, mut control, runner) =
        cyw43::new_with_bluetooth(state, pwr, spi, fw, btfw).await;
    unwrap!(spawner.spawn(cyw43_task(runner)));
    control.init(clm).await;

    let controller: Controller = ExternalController::new(bt_device);
    static RESOURCES: StaticCell<Resources<Controller>> = StaticCell::new();
    let resources = RESOURCES.init(Resources::new(PacketQos::None));
    let (stack, _, central, runner) = trouble_host::new(controller, resources).build();
    unwrap!(spawner.spawn(bt_task(runner)));

    ble_bas_central::run(stack, central).await;
}
