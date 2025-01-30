#![no_std]
#![no_main]

use bt_hci::controller::ExternalController;
use cyw43_pio::PioSpi;
use defmt::*;
use embassy_executor::Spawner;
use embassy_futures::select::select;
use embassy_rp::bind_interrupts;
use embassy_rp::gpio::{Level, Output};
use embassy_rp::peripherals::{DMA_CH0, PIO0};
use embassy_rp::pio::{InterruptHandler, Pio};
use static_cell::StaticCell;
use trouble_host::prelude::Controller;
use trouble_host::{HostResources, prelude::*};
use {defmt_rtt as _, embassy_time as _, panic_probe as _};

mod ble;

bind_interrupts!(struct Irqs {
    PIO0_IRQ_0 => InterruptHandler<PIO0>;
});

#[embassy_executor::task]
async fn cyw43_task(
    runner: cyw43::Runner<'static, Output<'static>, PioSpi<'static, PIO0, 0, DMA_CH0>>,
) -> ! {
    runner.run().await
}

/// Max number of connections
const CONNECTIONS_MAX: usize = 1;

/// Max number of L2CAP channels.
const L2CAP_CHANNELS_MAX: usize = 2; // Signal + att

const L2CAP_MTU: usize = 128;

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
        cyw43_pio::DEFAULT_CLOCK_DIVIDER,
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
    let controller: ble::Controller = ExternalController::new(bt_device);

    // Using a fixed "random" address can be useful for testing. In real scenarios, one would
    // use e.g. the MAC 6 byte array as the address (how to get that varies by the platform).
    let address: Address = Address::random([0xff, 0x8f, 0x1b, 0x05, 0xe4, 0xff]);
    info!("Our address = {:?}", address);

    static RESOURCES: StaticCell<ble::Resources> = StaticCell::new();
    static STACK: StaticCell<Stack<ble::Controller>> = StaticCell::new();
    let stack = STACK.init(
        trouble_host::new(controller, RESOURCES.init(HostResources::new()))
            .set_random_address(address),
    );
    let Host {
        central: _,
        runner,
        mut peripheral,
        ..
    } = stack.build();
    info!("starting bt task");
    unwrap!(spawner.spawn(ble::bt_task(runner)));

    // Sample BT LE Audio Peripheral
    {
        info!("Starting advertising and GATT service");
        let server = Server::new_with_config(GapConfig::Peripheral(PeripheralConfig {
            name: "Pico Speaker Test",
            appearance: &appearance::audio_sink::GENERIC_AUDIO_SINK,
        }))
        .unwrap();

        loop {
            match advertise("Pico Speaker Test", &mut peripheral).await {
                Ok(conn) => {
                    let client = GattClient::<ble::Controller, 10, 24>::new(stack, &conn)
                        .await
                        .unwrap();

                    select(
                        select(client.task(), trouble_audio::pacs::source_client(&client)),
                        async {
                            loop {
                                match conn.next().await {
                                    ConnectionEvent::Disconnected { reason } => {
                                        info!("[gatt] disconnected: {:?}", reason);
                                        break;
                                    }
                                    ConnectionEvent::Gatt { data } => {
                                        let event = data.process(&server).await;
                                        match event {
                                            Ok(event) => {
                                                if let Some(data) = event {
                                                    // trouble_audio::pacs::source_server::<
                                                    //     ble::Controller,
                                                    //     10,
                                                    // >(
                                                    //     &server.pacs, &data
                                                    // );
                                                }
                                            }
                                            Err(e) => {
                                                warn!("[gatt] error processing event: {:?}", e);
                                            }
                                        }
                                    }
                                }
                            }
                        },
                    )
                    .await;
                }
                Err(e) => {
                    let e = defmt::Debug2Format(&e);
                    defmt::panic!("[adv] error: {:?}", e);
                }
            }
        }
    }
}

// GATT Server definition
#[gatt_server]
struct Server {
    pacs: trouble_audio::pacs::PacsSource,
}

/// Create an advertiser to use to connect to a BLE Central, and wait for it to connect.
async fn advertise<'a, C: Controller>(
    name: &'a str,
    peripheral: &mut Peripheral<'a, C>,
) -> Result<Connection<'a>, BleHostError<C::Error>> {
    let mut advertiser_data = [0; 31];
    AdStructure::encode_slice(
        &[
            AdStructure::Flags(LE_GENERAL_DISCOVERABLE | BR_EDR_NOT_SUPPORTED),
            // This should match `Server`
            AdStructure::ServiceUuids16(&[Uuid::from(
                bt_hci::uuid::service::PUBLISHED_AUDIO_CAPABILITIES,
            )]),
            AdStructure::CompleteLocalName(name.as_bytes()),
        ],
        &mut advertiser_data[..],
    )?;
    let advertiser = peripheral
        .advertise(
            &Default::default(),
            Advertisement::ConnectableScannableUndirected {
                adv_data: &advertiser_data[..],
                scan_data: &[],
            },
        )
        .await?;
    info!("[adv] advertising");
    let conn = advertiser.accept().await?;
    info!("[adv] connection established");
    Ok(conn)
}
