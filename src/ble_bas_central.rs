use bt_hci::cmd::Cmd;
use bt_hci::cmd::le::{LeSetScanEnable, LeSetScanParams};
use bt_hci::controller::ControllerCmdSync;
use bt_hci::param::{AddrKind, LeScanKind, ScanningFilterPolicy};
use defmt::*;
use embassy_time::Duration;
use trouble_host::prelude::*;

pub async fn run<'a, C>(stack: Stack<'a, C>, mut central: Central<'a, C>)
where
    C: Controller + ControllerCmdSync<LeSetScanEnable> + ControllerCmdSync<LeSetScanParams>,
{
    info!("Sending Scanning params");
    stack
        .command(LeSetScanParams::new(
            LeScanKind::Passive,
            Duration::from_millis(10).into(),
            Duration::from_millis(10).into(),
            AddrKind::PUBLIC,
            ScanningFilterPolicy::BasicUnfiltered,
        ))
        .await
        .unwrap();

    info!("Searching for devices");
    let devices = stack
        .command(LeSetScanEnable::new(true, true))
        .await
        .unwrap();

    info!("avalibe devices: {:?}", devices);

    loop {
        info!("Scanning for peripheral...");
        'scan: loop {
            info!("Connecting");

            let target: Address = Address::random([0xff, 0x8f, 0x1a, 0x05, 0xe4, 0xff]);
            let config = ConnectConfig {
                connect_params: Default::default(),
                scan_config: ScanConfig {
                    filter_accept_list: &[(target.kind, &target.addr)],
                    ..Default::default()
                },
            };

            let conn = central.connect(&config).await.unwrap();
            info!("Connected, creating gatt client");

            // let client = GattClient::<C, 10, 24>::new(stack, &conn).await.unwrap();

            loop {
                if !conn.is_connected() {
                    break 'scan;
                }
            }
        }
    }
}
