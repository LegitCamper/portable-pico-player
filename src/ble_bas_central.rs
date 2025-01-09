use bt_hci::WriteHci;
use bt_hci::cmd::controller_baseband::Reset;
use bt_hci::cmd::le::{
    LeReadBufferSize, LeReadChannelMap, LeSetEventMask, LeSetScanEnable, LeSetScanParams,
};
use bt_hci::cmd::{Cmd, SyncCmd};
use bt_hci::controller::ControllerCmdSync;
use bt_hci::event::CommandComplete;
use bt_hci::param::{AddrKind, ConnHandle, LeEventMask, LeScanKind, ScanningFilterPolicy};
use defmt::*;
use embassy_time::Duration;
use trouble_host::{BdAddr, prelude::*};

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

    pub async fn reset(&mut self) {
        self.stack.command(Reset::new()).await.unwrap();
    }

    pub async fn scan(&mut self, timeout: Duration) -> Option<ScanReport> {
        info!("Scanning enabling");
        self.central
            .scan(&ScanConfig {
                active: true,
                filter_accept_list: &[],
                phys: PhySet::M2,
                interval: Duration::from_millis(100),
                window: Duration::from_millis(50),
                timeout,
            })
            .await
            .ok()
    }

    pub async fn connect(&mut self, addr: BdAddr) {
        info!("Connecting");
        let config = ConnectConfig {
            connect_params: Default::default(),
            scan_config: ScanConfig {
                filter_accept_list: &[(AddrKind::RANDOM, &addr)],
                timeout: Duration::from_millis(500),
                active: true,
                ..Default::default()
            },
        };

        self.conn = Some(self.central.connect(&config).await.unwrap());
        info!("Connected, creating gatt client");
    }

    pub async fn conn_met(&self) -> HostMetrics {
        self.stack.metrics()
    }
}
