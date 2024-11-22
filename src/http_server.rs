use anyhow::Result;
use embedded_svc::http::Headers;
use esp_idf_svc::{
    hal::reset::restart,
    http::{
        server::{Configuration, EspHttpServer},
        Method,
    },
    ota::EspOta,
    timer::EspTimerService,
};
use std::time::Duration;

const STATUS_CODE_REQUEST_ENTITY_TO_LARGE: u16 = 413;
const HTTP_SERVER_STACK_SIZE: usize = OTA_CHNUNK_SIZE + 1024 * 8;
const OTA_PARTITION_SIZE: usize = 0x1f0000;
const OTA_CHNUNK_SIZE: usize = 1024 * 8;

pub fn init() -> Result<EspHttpServer<'static>> {
    let configuration = Configuration {
        stack_size: HTTP_SERVER_STACK_SIZE,
        ..Default::default()
    };
    let mut server = EspHttpServer::new(&configuration)?;
    add_update_handler(&mut server)?;
    Ok(server)
}

fn add_update_handler(server: &mut EspHttpServer<'static>) -> Result<()> {
    server.fn_handler::<anyhow::Error, _>("/update", Method::Post, |mut request| {
        log::info!("Starting updater");
        let firmware_size = request.content_len().unwrap_or(0) as usize;

        if firmware_size > OTA_PARTITION_SIZE {
            request.into_status_response(STATUS_CODE_REQUEST_ENTITY_TO_LARGE)?;
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

    Ok(())
}
