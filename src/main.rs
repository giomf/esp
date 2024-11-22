mod base36;
mod http_server;
mod mdns;
mod wifi;

use anyhow::{Context, Result};
use esp_idf_svc::{
    eventloop::EspSystemEventLoop,
    hal::{prelude::Peripherals, task::block_on},
    wifi::WifiEvent,
};
use wifi::Wifi;

const SSID: &str = env!("WIFI_SSID");
const PASSWORD: &str = env!("WIFI_PASS");

fn main() -> Result<()> {
    esp_idf_svc::sys::link_patches();
    esp_idf_svc::log::EspLogger::initialize_default();

    let peripherals = Peripherals::take()?;
    let event_loop = EspSystemEventLoop::take()?;

    let mut wifi = Wifi::new(event_loop.clone(), peripherals.modem)?;
    let mut wifi_subscription = event_loop.subscribe_async::<WifiEvent>().unwrap();

    let mac_address = wifi.get_mac_address();
    let _mdns = mdns::init(mac_address).context("Failed to initialize mDNS")?;

    let _http_server = http_server::init().context("Failed to intialize http server")?;

    block_on(async move {
        wifi.connect(SSID, PASSWORD).await.unwrap();

        loop {
            match wifi_subscription.recv().await.unwrap() {
                WifiEvent::StaDisconnected(_) => {
                    log::error!("Wifi disconnected! Retrying.");
                    // Reconnect while ignoring all errors
                    let _ = wifi.connect(SSID, PASSWORD).await;
                }
                _ => (),
            }
        }
    });

    Ok(())
}
