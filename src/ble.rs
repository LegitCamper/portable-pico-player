use core::any::Any;

use bt_hci::{controller::ExternalController, uuid::service};
use cyw43::bluetooth::BtDriver;
use defmt::*;
use embassy_futures::select::select;
use embassy_sync::blocking_mutex::raw::NoopRawMutex;
use embassy_time::Timer;
use heapless::Vec;
use trouble_audio::{MAX_SERVICES, pacs};
use trouble_host::{
    gap::{GapConfig, PeripheralConfig},
    prelude::{
        AdStructure, Advertisement, AttributeServer, AttributeTable, BR_EDR_NOT_SUPPORTED,
        BleHostError, Central, Connection, ConnectionEvent, HostResources, LE_GENERAL_DISCOVERABLE,
        Peripheral, Runner, Service, Uuid, appearance, gatt_server,
    },
};

/// Max number of connections
const CONNECTIONS_MAX: usize = 1;

/// Size of L2CAP packets
pub const L2CAP_MTU: usize = 128;

/// Max number of L2CAP channels.
const L2CAP_CHANNELS_MAX: usize = 3; // Signal + att + CoC

/// Max size of a gatt packet - minus headers
const ATT_MTU: usize = L2CAP_MTU - 4 - 3;

/// The size needed to store all le audio server data
const STORAGE_SIZE: usize = ATT_MTU * MAX_SERVICES;

const CONTROLLER_SLOTS: usize = 10;

pub type ControllerT = ExternalController<BtDriver<'static>, CONTROLLER_SLOTS>;
pub type Resources = HostResources<CONNECTIONS_MAX, L2CAP_CHANNELS_MAX, L2CAP_MTU>;

pub async fn ble_task(runner: &mut Runner<'_, ControllerT>) {
    loop {
        if let Err(e) = runner.run().await {
            let e = defmt::Debug2Format(&e);
            defmt::error!("[ble_task] error: {:?}", e);
        }
    }
}

pub async fn run(
    mut runner: &mut Runner<'_, ControllerT>,
    mut _central: Central<'_, ControllerT>,
    mut peripheral: Peripheral<'_, ControllerT>,
) -> ! {
    let mut gatt_storage: [u8; STORAGE_SIZE] = [0; STORAGE_SIZE];

    loop {
        select(ble_task(&mut runner), async {
            loop {
                match advertise::<ControllerT>("Pico Speaker Test", &mut peripheral).await {
                    Ok(conn) => {
                        info!("[adv] connection established");
                        let mut server_builder =
                            trouble_audio::ServerBuilder::<ATT_MTU, NoopRawMutex>::new(
                                b"Pico Speaker Test",
                                &appearance::audio_sink::GENERIC_AUDIO_SINK,
                                gatt_storage.as_mut_slice(),
                            );
                        // server_builder.add_pacs();
                        // let server = server_builder.build();
                        // loop {
                        //     match conn.next().await {
                        //         ConnectionEvent::Disconnected { reason } => {
                        //             info!("[gatt] disconnected: {:?}", reason);
                        //             break;
                        //         }
                        //         ConnectionEvent::Gatt { data } => server.process(data).await,
                        //     }
                        // }
                    }
                    Err(e) => {
                        let e = defmt::Debug2Format(&e);
                        defmt::error!("[adv] error: {:?}", e);
                    }
                }
            }
        })
        .await;
        info!("Exiting Bluetooth");
    }
}

/// Create an advertiser to use to connect to a BLE Central, and wait for it to connect.
pub async fn advertise<'a, C>(
    name: &'a str,
    peripheral: &mut Peripheral<'a, C>,
) -> Result<Connection<'a>, BleHostError<C::Error>>
where
    C: trouble_host::prelude::Controller,
{
    let mut advertiser_data = [0; 45];
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
    Ok(conn)
}
