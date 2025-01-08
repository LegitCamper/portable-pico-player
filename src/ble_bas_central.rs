use bt_hci::cmd::Cmd;
use bt_hci::cmd::le::{LeSetScanEnable, LeSetScanParams};
use bt_hci::controller::ControllerCmdSync;
use bt_hci::param::{AddrKind, LeScanKind, ScanningFilterPolicy};
use defmt::*;
use embassy_time::Duration;
use trouble_host::prelude::*;

pub struct Ble<'a, C>
where
    C: Controller + ControllerCmdSync<LeSetScanEnable> + ControllerCmdSync<LeSetScanParams>,
{
    stack: Stack<'a, C>,
    central: Central<'a, C>,
    conn: Option<Connection<'a>>,
}

impl<'a, C> Ble<'a, C>
where
    C: Controller + ControllerCmdSync<LeSetScanEnable> + ControllerCmdSync<LeSetScanParams>,
{
    pub fn new(stack: Stack<'a, C>, central: Central<'a, C>) -> Self {
        Self {
            stack,
            central,
            conn: None,
        }
    }

    pub async fn scan(&mut self) {
        info!("Sending Scanning params");
        self.stack
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
        let devices = self
            .stack
            .command(LeSetScanEnable::new(true, true))
            .await
            .unwrap();

        info!("avalibe devices: {:?}", devices);
    }

    pub async fn connect(&mut self, target: Address) {
        info!("Connecting");
        let config = ConnectConfig {
            connect_params: Default::default(),
            scan_config: ScanConfig {
                filter_accept_list: &[(target.kind, &target.addr)],
                ..Default::default()
            },
        };

        self.conn = Some(self.central.connect(&config).await.unwrap());
        info!("Connected, creating gatt client");
    }

    pub async fn run(&mut self) {
        loop {
            info!("Scanning for peripheral...");
            'scan: loop {
                self.connect(Address::random([0xff, 0x8f, 0x1a, 0x05, 0xe4, 0xff]))
                    .await;

                if let Some(conn) = &self.conn {
                    let client = GattClient::<C, 10, 24>::new(self.stack, &conn)
                        .await
                        .unwrap();

                    loop {
                        if !conn.is_connected() {
                            break 'scan;
                        }
                    }
                }
            }
        }
    }
}
