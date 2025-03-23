use crate::uart::Uart;
use am03127::{
    page_content::{
        formatting::{Clock as ClockFormat, ColumnStart, Font},
        Lagging, Leading, PageContent, WaitingModeAndSpeed,
    },
    real_time_clock::RealTimeClock,
};
use anyhow::{bail, Context, Result};
use embedded_svc::http::Headers;
use esp_idf_svc::{
    hal::reset::restart,
    http::{
        server::{Configuration, EspHttpConnection, EspHttpServer, Request},
        Method,
    },
    io::Write,
    ota::EspOta,
    timer::EspTimerService,
};
use heapless::String;
use serde::{de::DeserializeOwned, Deserialize, Serialize};
use std::sync::{Arc, Mutex};
use std::time::Duration;

const STATUS_CODE_LENGTH_REQUIRED: u16 = 411;
const STATUS_CODE_REQUEST_ENTITY_TO_LARGE: u16 = 413;
const STATUS_CODE_UNSUPPORTED_MEDIA_TYPE: u16 = 415;

const HTTP_SERVER_STACK_SIZE: usize = OTA_CHNUNK_SIZE + 1024 * 8;
const OTA_PARTITION_SIZE: usize = 0x1f0000;
const OTA_CHNUNK_SIZE: usize = 1024 * 8;
const CONTENT_TYPE_OCTET_STEAM: &str = "application/octet-stream";
const CONTENT_TYPE_JSON: &str = "application/json";

#[derive(Debug, Clone, Default, Serialize)]
pub struct Status {
    pub hostname: String<30>,
    pub version: String<24>,
}

#[derive(Debug, Clone, Default, Deserialize)]
pub struct Clock {
    pub day: u8,
    pub hour: u8,
    pub minute: u8,
    pub month: u8,
    pub second: u8,
    pub year: u8,
}

#[derive(Deserialize, Debug, Clone)]
#[serde(deny_unknown_fields)]
pub struct FormattedText {
    pub text: String<24>,
    #[serde(default)]
    pub leading: Leading,
    #[serde(default)]
    pub lagging: Lagging,
    #[serde(default)]
    pub waiting_mode_and_speed: WaitingModeAndSpeed,
}

pub fn init(hostname: String<30>, uart: Uart) -> Result<EspHttpServer<'static>> {
    log::info!("Initialize http server");
    let configuration = Configuration {
        stack_size: HTTP_SERVER_STACK_SIZE,
        ..Default::default()
    };

    // Wrap the Uart in Arc<Mutex<>> for shared ownership
    let uart = Arc::new(Mutex::new(uart));

    let mut server = EspHttpServer::new(&configuration)?;
    add_update_handler(&mut server)?;

    // Pass clones of the Arc to each handler
    add_text_handler(&mut server, Arc::clone(&uart))?;
    add_clock_handler(&mut server, Arc::clone(&uart))?;
    add_status_handler(&mut server, hostname)?;

    Ok(server)
}

fn add_clock_handler(server: &mut EspHttpServer<'static>, uart: Arc<Mutex<Uart>>) -> Result<()> {
    let uart_get = uart.clone();
    server.fn_handler::<anyhow::Error, _>("/clock", Method::Get, move |request| {
        log::info!("Displaying clock");
        let message = format!(
            "{}{}{}{}",
            ClockFormat::Time,
            Font::Narrow,
            ColumnStart(41),
            ClockFormat::Date
        );
        let command = PageContent::default().message(&message).command();

        // Lock the UART to get exclusive access
        // The lock is automatically released when uart_guard goes out of scope
        let uart = uart_get
            .lock()
            .map_err(|e| anyhow::anyhow!("Failed to lock UART: {:?}", e))?;

        // Write to the UART
        uart.write(&command)?;

        // Respond to the HTTP request
        request.into_ok_response()?.write(&[])?;
        Ok(())
    })?;

    let uart_post = uart.clone();
    server.fn_handler::<anyhow::Error, _>("/clock", Method::Post, move |mut request| {
        log::info!("Displaying clock");

        // Check content type
        if !request
            .content_type()
            .is_some_and(|content_type| content_type == CONTENT_TYPE_JSON)
        {
            log::warn!("Wrong content type");
            request
                .into_status_response(STATUS_CODE_UNSUPPORTED_MEDIA_TYPE)?
                .write(&[])?;
            return Ok(());
        }

        let clock = read_json_body::<Clock>(&mut request)?;
        let command = RealTimeClock::default()
            .year(clock.year)
            .month(clock.month)
            .day(clock.day)
            .hour(clock.hour)
            .minute(clock.minute)
            .second(clock.second)
            .command();

        // Lock the UART to get exclusive access
        // The lock is automatically released when uart_guard goes out of scope
        let uart = uart_post
            .lock()
            .map_err(|e| anyhow::anyhow!("Failed to lock UART: {:?}", e))?;

        // Write to the UART
        uart.write(&command)?;

        // Respond to the HTTP request
        request.into_ok_response()?.write(&[])?;
        Ok(())
    })?;
    Ok(())
}

fn add_text_handler(server: &mut EspHttpServer<'static>, uart: Arc<Mutex<Uart>>) -> Result<()> {
    server.fn_handler::<anyhow::Error, _>("/text", Method::Post, move |mut request| {
        log::info!("Setting Panel text");

        // Check content type
        if !request
            .content_type()
            .is_some_and(|content_type| content_type == CONTENT_TYPE_JSON)
        {
            log::warn!("Wrong content type");
            request
                .into_status_response(STATUS_CODE_UNSUPPORTED_MEDIA_TYPE)?
                .write(&[])?;
            return Ok(());
        }

        let formatted_text = read_json_body::<FormattedText>(&mut request)?;
        let command = PageContent::default()
            .leading(formatted_text.leading)
            .lagging(formatted_text.lagging)
            .waiting_mode_and_speed(formatted_text.waiting_mode_and_speed)
            .message(&formatted_text.text)
            .command();

        // Lock the UART to get exclusive access
        let uart = uart
            .lock()
            .map_err(|e| anyhow::anyhow!("Failed to lock UART: {:?}", e))?;

        // Write to the UART and handle errors
        uart.write(&command).context("Failed to write to uart")?;

        // Respond to the HTTP request
        request.into_ok_response()?.write(&[])?;
        Ok(())
    })?;
    Ok(())
}

fn add_update_handler(server: &mut EspHttpServer<'static>) -> Result<()> {
    server.fn_handler::<anyhow::Error, _>("/update", Method::Post, |mut request| {
        log::info!("Starting updater");

        if !request
            .content_type()
            .is_some_and(|content_type| content_type == CONTENT_TYPE_OCTET_STEAM)
        {
            request.into_status_response(STATUS_CODE_UNSUPPORTED_MEDIA_TYPE)?;
            return Ok(());
        }

        let firmware_size = match request.content_len() {
            None => {
                request.into_status_response(STATUS_CODE_LENGTH_REQUIRED)?;
                return Ok(());
            }
            Some(size) => size as usize,
        };

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
            let bytes_read = request.read(&mut buffer)?;
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

fn add_status_handler(server: &mut EspHttpServer<'static>, hostname: String<30>) -> Result<()> {
    server.fn_handler::<anyhow::Error, _>("/status", Method::Get, move |request| {
        log::info!("Sending Status information");
        let ota = EspOta::new()?;
        let running_slot = ota.get_running_slot()?;

        let status = Status {
            hostname: hostname.clone(),
            version: running_slot.firmware.unwrap().version,
        };

        let status = serde_json::to_string(&status)?;
        request.into_ok_response()?.write_all(&status.as_bytes())?;
        Ok(())
    })?;
    Ok(())
}

fn read_json_body<T: DeserializeOwned>(
    request: &mut Request<&mut EspHttpConnection<'_>>,
) -> Result<T> {
    // Read request body
    let mut body = Vec::new();
    let mut buffer = [0u8; 128];
    loop {
        match request.read(&mut buffer) {
            Ok(0) => break, // No more data to read
            Ok(n) => body.extend_from_slice(&buffer[..n]),
            Err(e) => {
                bail!("Error reading request body: {}", e);
            }
        }
    }

    // Parse the JSON body
    let json_body = serde_json::from_slice::<T>(&body).context("Failed to parse json body")?;
    Ok(json_body)
}
