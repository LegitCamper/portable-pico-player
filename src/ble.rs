use bt_hci::controller::ExternalController;
use cyw43::bluetooth::BtDriver;
use defmt::*;
use embassy_futures::select::select;
use embassy_sync::blocking_mutex::raw::NoopRawMutex;
use trouble_audio::run_server;
use trouble_host::prelude::{
    AdStructure, Advertisement, BR_EDR_NOT_SUPPORTED, BleHostError, Central, Connection,
    ConnectionEvent, GapConfig, GattClient, HostResources, LE_GENERAL_DISCOVERABLE, Peripheral,
    PeripheralConfig, Runner, Stack, Uuid, appearance, gatt_server,
};

/// Size of L2CAP packets
pub const L2CAP_MTU: usize = 128;

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
            defmt::error!("[ble_task] error: {:?}", e);
        }
    }
}

pub async fn run(
    mut runner: &mut Runner<'_, ControllerT>,
    mut _central: Central<'_, ControllerT>,
    mut peripheral: Peripheral<'_, ControllerT>,
) {
    let mut gatt_storage: [u8; L2CAP_MTU * 3] = [0; 384];
    loop {
        select(ble_task(&mut runner), async {
            loop {
                match advertise::<ControllerT>("Pico Speaker Test", &mut peripheral).await {
                    Ok(conn) => {
                        info!("[adv] connection established");
                        run_server::<{ L2CAP_MTU }, NoopRawMutex>(
                            &conn,
                            "Pico Speaker Test",
                            &appearance::audio_sink::GENERIC_AUDIO_SINK.to_le_bytes(),
                            &mut gatt_storage,
                        )
                        .await;
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
