use bt_hci::controller::ExternalController;
use cyw43::bluetooth::BtDriver;
use defmt::*;
use trouble_host::prelude::*;

/// Size of L2CAP packets
const L2CAP_MTU: usize = 128;

/// Max number of connections
const CONNECTIONS_MAX: usize = 1;

/// Max number of L2CAP channels.
const L2CAP_CHANNELS_MAX: usize = 3; // Signal + att + CoC

pub type Controller = ExternalController<BtDriver<'static>, 10>;
pub type Resources = HostResources<CONNECTIONS_MAX, L2CAP_CHANNELS_MAX, L2CAP_MTU>;

#[embassy_executor::task]
pub async fn bt_task(mut runner: trouble_host::prelude::Runner<'static, Controller>) -> ! {
    loop {
        if let Err(error) = runner.run().await {
            match error {
                trouble_host::BleHostError::Controller(err) => {
                    error!("Bt Controller error: {}", err)
                }
                trouble_host::BleHostError::BleHost(err) => {
                    error!("Bt Host error: {}", err)
                }
            }
        }
    }
}
