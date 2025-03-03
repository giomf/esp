mod base36;
mod http_server;
mod mdns;
mod uart;
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
    let hostname = wifi.get_hostname()?;

    let _mdns = mdns::init(&hostname).context("Failed to initialize mDNS")?;
    let uart = uart::Uart::new(
        peripherals.uart1,
        peripherals.pins.gpio2,
        peripherals.pins.gpio3,
    )?;

    uart.init().context("Failed to initialize panel")?;
    let _http_server = http_server::init(hostname, uart).context("Failed to intialize http server")?;

    block_on(async move {
        wifi.connect(SSID, PASSWORD).await.unwrap();
        let mut wifi_subscription = event_loop.subscribe_async::<WifiEvent>().unwrap();

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
