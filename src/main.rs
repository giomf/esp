use anyhow::Result;
use core::convert::TryInto;
use embedded_svc::{
    http::{Headers, Method},
    wifi::{AuthMethod, ClientConfiguration, self},
};
use esp_idf_svc::{
    eventloop::{EspAsyncSubscription, EspSystemEventLoop, System},
    hal::{prelude::Peripherals, task::block_on},
    http::server::{self, EspHttpServer},
    nvs::EspDefaultNvsPartition,
    ota::EspOta,
    timer::EspTaskTimerService,
    wifi::{AsyncWifi, EspWifi, WifiEvent},
};

const SSID: &str = env!("WIFI_SSID");
const PASSWORD: &str = env!("WIFI_PASS");

const OTA_PARTITION_SIZE: usize = 0x1f0000;
const OTA_CHNUNK_SIZE: usize = 1024 * 20;

fn main() -> Result<()> {
    esp_idf_svc::sys::link_patches();
    esp_idf_svc::log::EspLogger::initialize_default();

    let peripherals = Peripherals::take()?;
    let event_loop = EspSystemEventLoop::take()?;
    let nvs = EspDefaultNvsPartition::take()?;
    let wifi_driver = EspWifi::new(peripherals.modem, event_loop.clone(), Some(nvs)).unwrap();

    block_on(async move {
        let mut server = EspHttpServer::new(&server::Configuration::default()).unwrap();
        let (mut wifi, mut subscription) = wifi_init(wifi_driver, event_loop).await.unwrap();
        server_init(&mut server).unwrap();

        loop {
            match subscription.recv().await.unwrap() {
                WifiEvent::StaDisconnected => {
                    wifi.connect().await.unwrap();
                }
                _ => (),
            }
        }
    });

    Ok(())
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
    wifi.connect().await?;
    wifi.wait_netif_up().await?;
    let subscription = event_loop.subscribe_async::<WifiEvent>().unwrap();
    Ok((wifi, subscription))
}

fn server_init(server: &mut EspHttpServer) -> Result<()> {
    server.fn_handler::<anyhow::Error, _>("/update", Method::Post, |mut request| {
        log::info!("Starting updater");
        let firmware_size = request.content_len().unwrap_or(0) as usize;

        if firmware_size > OTA_PARTITION_SIZE {
            request.into_status_response(413)?.write("".as_bytes())?;
            return Ok(());
        }

        let mut ota = EspOta::new()?;
        let mut ota = ota.initiate_update()?;
        let mut buffer = vec![0; OTA_CHNUNK_SIZE];
        let mut total_bytes_read: usize = 0;

        log::info!("Start uploading. Expected {firmware_size} bytes");
        loop {
            let bytes_read = request.read(&mut buffer).unwrap_or_default();
            total_bytes_read += bytes_read;
            log::debug!("Read {total_bytes_read}/{firmware_size} bytes");

            if bytes_read > 0 {
                if let Err(err) = ota.write(&buffer[..bytes_read]) {
                    log::error!("Error: {err}");
                    ota.abort()?;
                    break;
                }
            }

            if total_bytes_read >= firmware_size {
                log::info!("Update finished");
                ota.complete()?;
                break;
            }
        }

        if total_bytes_read < firmware_size {
            log::error!(
                "Only {total_bytes_read}/{firmware_size} bytes downloaded. May be network error?"
            );
        }

        request.into_ok_response()?.write("".as_bytes())?;
        Ok(())
    })?;

    Ok(())
}
