use bt_hci::controller::ExternalController;
use cyw43::bluetooth::BtDriver;
use defmt::*;
use embassy_futures::select::select;
use trouble_host::prelude::{
    AdStructure, Advertisement, BR_EDR_NOT_SUPPORTED, BleHostError, Connection, ConnectionEvent,
    GapConfig, GattClient, HostResources, LE_GENERAL_DISCOVERABLE, Peripheral, PeripheralConfig,
    Runner, Stack, Uuid, appearance, gatt_server,
};

/// Size of L2CAP packets
const L2CAP_MTU: usize = 128;

/// Max number of connections
const CONNECTIONS_MAX: usize = 1;

/// Max number of L2CAP channels.
const L2CAP_CHANNELS_MAX: usize = 3; // Signal + att + CoC

const CONTROLLER_SLOTS: usize = 10;

pub type ControllerT = ExternalController<BtDriver<'static>, CONTROLLER_SLOTS>;
pub type Resources = HostResources<CONNECTIONS_MAX, L2CAP_CHANNELS_MAX, L2CAP_MTU>;

pub async fn ble_task(runner: &mut Runner<'_, ControllerT>) {
    loop {
        if let Err(e) = runner.run().await {
            let e = defmt::Debug2Format(&e);
            defmt::panic!("[ble_task] error: {:?}", e);
        }
    }
}

// GATT Server definition
#[gatt_server]
struct Server {
    pacs: trouble_audio::pacs::PacsSource,
}

pub async fn le_audio_periphery_test<'a>(
    peripheral: &mut Peripheral<'a, ControllerT>,
    stack: &'a Stack<'a, ControllerT>,
) {
    info!("Starting advertising and GATT service");
    let server = Server::new_with_config(GapConfig::Peripheral(PeripheralConfig {
        name: "Pico Speaker Test",
        appearance: &appearance::audio_sink::GENERIC_AUDIO_SINK,
    }))
    .unwrap();

    loop {
        match advertise::<ControllerT>("Pico Speaker Test", peripheral).await {
            Ok(conn) => {
                let client = GattClient::<ControllerT, 10, 24>::new(&stack, &conn)
                    .await
                    .unwrap();

                select(
                    client.task(),
                    select(
                        run_server(&server, &conn),
                        trouble_audio::pacs::sink_client(&client),
                    ),
                )
                .await;
            }
            Err(e) => {
                // let e = defmt::Debug2Format(&e);
                // defmt::panic!("[adv] error: {:?}", e);
            }
        }
    }
}

async fn run_server<'a>(server: &Server<'a>, conn: &Connection<'a>) {
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
                            // trouble_audio::pacs::sink_server::<
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
}

/// Create an advertiser to use to connect to a BLE Central, and wait for it to connect.
async fn advertise<'a, C>(
    name: &'a str,
    peripheral: &mut Peripheral<'a, C>,
) -> Result<Connection<'a>, BleHostError<C::Error>>
where
    C: trouble_host::prelude::Controller,
{
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
