mod base36;

use anyhow::{Context, Result};
use core::convert::TryInto;
use std::time::Duration;
use embedded_svc::{
    http::{Headers, Method},
    wifi::{self, AuthMethod, ClientConfiguration},
};
use esp_idf_svc::{
    eventloop::{EspAsyncSubscription, EspSystemEventLoop, System}, hal::{prelude::Peripherals, reset::restart, task::block_on}, http::server::{self, EspHttpServer}, mdns::EspMdns, nvs::EspDefaultNvsPartition, ota::EspOta, timer::{EspTaskTimerService, EspTimerService}, wifi::{AsyncWifi, EspWifi, WifiDeviceId, WifiEvent}
};

const SSID: &str = env!("WIFI_SSID");
const PASSWORD: &str = env!("WIFI_PASS");

const OTA_PARTITION_SIZE: usize = 0x1f0000;
const OTA_CHNUNK_SIZE: usize = 1024 * 8;

const MDNS_SERVICE_NAME: &str = "_efm";
const MDNS_SERVICE_PROTOCOL: &str = "_tcp";
const MDNS_SERVICE_PORT: u16 = 80;

const STATUS_CODE_REQUEST_ENTITY_TO_LARGE: u16 = 413;
const HTTP_SERVER_STACK_SIZE: usize = OTA_CHNUNK_SIZE + 1024 * 8;

fn main() -> Result<()> {
    esp_idf_svc::sys::link_patches();
    esp_idf_svc::log::EspLogger::initialize_default();

    let peripherals = Peripherals::take()?;
    let event_loop = EspSystemEventLoop::take()?;
    let nvs = EspDefaultNvsPartition::take()?;
    let wifi_driver = EspWifi::new(peripherals.modem, event_loop.clone(), Some(nvs))
        .context("Failed to create wifi driver")?;
    let mac_address = wifi_driver.get_mac(WifiDeviceId::Sta)?;
    let _mdns = mdns_init(mac_address).context("Failed to initialize mDNS")?;
    let _http_server = http_server_init().context("Failed to intialize http server")?;

    block_on(async move {
        let (mut wifi, mut wifi_subscription) = wifi_init(wifi_driver, event_loop)
            .await
            .context("Failed to initialize wifi")
            .unwrap();

        loop {
            match wifi_subscription.recv().await.unwrap() {
                WifiEvent::StaDisconnected(_) => {
                    log::error!("Wifi disconnected! Retrying.");
                    // Reconnect while ignoring all errors
                    let _ = wifi.connect().await;
                }
                _ => (),
            }
        }
    });

    Ok(())
}

fn mdns_init(mac_address: [u8; 6]) -> Result<EspMdns> {
    let hostname = base36::encode(mac_address);
    log::info!("Set {hostname} as mDNS hostname");
    let mut mdns = EspMdns::take()?;
    mdns.set_hostname(hostname)?;
    mdns.add_service(
        None,
        MDNS_SERVICE_NAME,
        MDNS_SERVICE_PROTOCOL,
        MDNS_SERVICE_PORT,
        Default::default(),
    )?;
    Ok(mdns)
}

async fn wifi_init<'a>(
    wifi_driver: EspWifi<'a>,
    event_loop: EspSystemEventLoop,
) -> Result<(
    AsyncWifi<EspWifi<'a>>,
    EspAsyncSubscription<WifiEvent<'a>, System>,
)> {
    let timer_service = EspTaskTimerService::new()?;
    let mut wifi = AsyncWifi::wrap(wifi_driver, event_loop.clone(), timer_service).unwrap();
    let wifi_configuration = wifi::Configuration::Client(ClientConfiguration {
        ssid: SSID.try_into().unwrap(),
        auth_method: AuthMethod::WPA2Personal,
        password: PASSWORD.try_into().unwrap(),
        channel: None,
        ..Default::default()
    });

    wifi.set_configuration(&wifi_configuration)?;
    wifi.start().await?;

    while let Err(err) = wifi.connect().await {
        log::error!("Error connecting to wifi {err}! Retrying.");
    }

    wifi.wait_netif_up().await?;
    let subscription = event_loop.subscribe_async::<WifiEvent>().unwrap();
    Ok((wifi, subscription))
}

fn http_server_init() -> Result<EspHttpServer<'static>> {
    let configuration = server::Configuration {
        stack_size: HTTP_SERVER_STACK_SIZE,
        ..Default::default()
    };
    let mut server = EspHttpServer::new(&configuration)?;
    server.fn_handler::<anyhow::Error, _>("/update", Method::Post, |mut request| {
        log::info!("Starting updater");
        let firmware_size = request.content_len().unwrap_or(0) as usize;

        if firmware_size > OTA_PARTITION_SIZE {
            request
                .into_status_response(STATUS_CODE_REQUEST_ENTITY_TO_LARGE)?;
            return Ok(());
        }

        let mut ota = EspOta::new()?;
        let running_slot = ota.get_running_slot()?;
        let update_slot = ota.get_update_slot()?;
        log::info!(
            "Current slot: {} - {}",
            running_slot.label,
            running_slot.firmware.unwrap().version
        );
        log::info!("Update slot: {}", update_slot.label);

        let mut ota_updater = ota.initiate_update()?;
        let mut buffer = [0; OTA_CHNUNK_SIZE];
        let mut total_bytes_read: usize = 0;

        log::info!("Start uploading. Expected {firmware_size} bytes");
        loop {
            let bytes_read = request.read(&mut buffer).unwrap_or_default();
            total_bytes_read += bytes_read;
            log::info!("Read {total_bytes_read}/{firmware_size} bytes from firmware");

            if bytes_read > 0 {
                if let Err(err) = ota_updater.write(&buffer[..bytes_read]) {
                    log::error!("Error: {err}");
                    ota_updater.abort()?;
                    break;
                }
            }

            if total_bytes_read >= firmware_size {
                log::info!("Update finished");
                ota_updater.complete()?;
                break;
            }
        }

        if total_bytes_read < firmware_size {
            log::error!(
                "Only {total_bytes_read}/{firmware_size} bytes downloaded. May be network error?"
            );
        }


        let reboot_timer = EspTimerService::new()?;
        let reboot_timer = reboot_timer.timer(move || {
            log::info!("Rebooting");
            restart();
        })?;
        log::info!("Schedule reboot in 5 seconds...");
        request.into_ok_response()?;
        reboot_timer.after(Duration::from_secs(5))?;
        std::mem::forget(reboot_timer);
        Ok(())
    })?;

    Ok(server)
}
