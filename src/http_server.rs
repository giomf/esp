use crate::uart::Uart;
use am03127::{
    page_content::{
        formatting::{Clock as ClockFormat, ColumnStart, Font},
        Lagging, Leading, PageContent, WaitingModeAndSpeed,
    },
    real_time_clock::RealTimeClock,
};
use anyhow::Result;
use core::fmt::Debug;
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
use heapless::{String, Vec};
use serde::{de::DeserializeOwned, Deserialize, Serialize};
use std::time::Duration;
use std::{
    str::FromStr,
    sync::{Arc, Mutex},
};
use thiserror::Error;

static HTML: &str = include_str!("index.html");

const STATUS_CODE_BAD_REQUEST: u16 = 400;
const STATUS_CODE_LENGTH_REQUIRED: u16 = 411;
const STATUS_CODE_REQUEST_ENTITY_TO_LARGE: u16 = 413;
const STATUS_CODE_UNSUPPORTED_MEDIA_TYPE: u16 = 415;

const HTTP_SERVER_STACK_SIZE: usize = OTA_CHNUNK_SIZE + 1024 * 8;
const HTTP_SERVER_MAX_RESPONSE_BODY_SIZE: usize = 1024;
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
    pub text: String<32>,
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
    add_web_page_handler(&mut server)?;

    Ok(server)
}

fn error_handling_wrapper<E, F>(
    handler: F,
) -> impl for<'r> Fn(Request<&mut EspHttpConnection<'r>>) -> Result<(), E> + Send + 'static
where
    F: for<'r> Fn(
            &mut Request<&mut EspHttpConnection<'r>>,
        ) -> Result<Vec<u8, HTTP_SERVER_MAX_RESPONSE_BODY_SIZE>, CustomError>
        + Send
        + 'static,
    E: Debug,
{
    move |request| match handler(&mut request) {
        Ok(body) => {
            request
                .into_ok_response()
                .unwrap()
                .write_all(&body)
                .unwrap();
            Ok(())
        }
        Err(err) => {
            log::error!("Handler error: {:?}", err);
            let status = match err {
                CustomError::Unknown => 500,
                CustomError::InvalidContentType() => 415,
                CustomError::Parsing(_) => 400,
            };

            request
                .into_status_response(status)
                .unwrap()
                .write_all(b"")
                .unwrap();
            Ok(())
        }
    }
}

fn add_web_page_handler(server: &mut EspHttpServer<'static>) -> Result<()> {
    // Do not use the error wrapper here since we want not to be limited by the max body size.
    server.fn_handler::<anyhow::Error, _>("/", Method::Get, |request| {
        request.into_ok_response()?.write_all(HTML.as_bytes())?;
        Ok(())
    })?;
    Ok(())
}

fn add_clock_handler(server: &mut EspHttpServer<'static>, uart: Arc<Mutex<Uart>>) -> Result<()> {
    let uart_get = uart.clone();
    server.fn_handler::<anyhow::Error, _>("/clock", Method::Get, move |request| {
        log::info!("Display clock");
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
        log::info!("Setting clock");

        // Check content type
        if !request
            .content_type()
            .is_some_and(|content_type| content_type == CONTENT_TYPE_JSON)
        {
            log::warn!("Content type not supported");
            request
                .into_status_response(STATUS_CODE_UNSUPPORTED_MEDIA_TYPE)?
                .write(b"Content type not supported")?;
            return Ok(());
        }

        let clock = match read_json_body::<Clock>(&mut request) {
            Ok(clock) => clock,
            Err(err) => {
                log::error!(
                    "Bad request: {}\n{}",
                    err.to_string(),
                    err.root_cause().to_string()
                );
                request
                    .into_status_response(STATUS_CODE_BAD_REQUEST)?
                    .write_all(err.to_string().as_bytes())?;
                return Ok(());
            }
        };

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

#[derive(Error, Debug)]
pub enum CustomError {
    #[error("Content type \"{received}\" not accepted. Try \"{expected}\" instead.")]
    InvalidContentType {
        expected: String<32>,
        received: String<32>,
    },
    #[error("Parsing Error")]
    Parsing(#[from] serde_json::Error),

    #[error("Unknown Error")]
    Unknown,
}

fn add_text_handler(server: &mut EspHttpServer<'static>, uart: Arc<Mutex<Uart>>) -> Result<()> {
    server.fn_handler::<CustomError, _>(
        "/text",
        Method::Post,
        error_handling_wrapper(|mut request| {
            log::info!("Setting Panel text");

            // Check content type
            let content_type = request.content_type().unwrap_or_default();
            if content_type == CONTENT_TYPE_JSON {
                return Err(CustomError::InvalidContentType {
                    expected: String::from_str(CONTENT_TYPE_JSON)
                        .map_err(|_| CustomError::Unknown)?,
                    received: String::from_str("").map_err(|_| CustomError::Unknown)?,
                });
            }
            let formatted_text = read_json_body::<FormattedText>(&mut request)?;

            let command = PageContent::default()
                .leading(formatted_text.leading)
                .lagging(formatted_text.lagging)
                .waiting_mode_and_speed(formatted_text.waiting_mode_and_speed)
                .message(&formatted_text.text)
                .command();

            // Lock the UART to get exclusive access
            let uart = uart.lock().map_err(|err| CustomError::Unknown)?;

            // Write to the UART and handle errors
            uart.write(&command).context("Failed to write to uart")?;

            // Respond to the HTTP request
            request.into_ok_response()?.write(&[])?;
            Ok(Vec::new())
        }),
    )?;
    Ok(())
}

fn add_update_handler(server: &mut EspHttpServer<'static>) -> Result<()> {
    server.fn_handler::<anyhow::Error, _>("/update", Method::Post, |mut request| {
        log::info!("Starting updater");

        if !request
            .content_type()
            .is_some_and(|content_type| content_type == CONTENT_TYPE_OCTET_STEAM)
        {
            log::warn!("Content type not supported");
            request
                .into_status_response(STATUS_CODE_UNSUPPORTED_MEDIA_TYPE)?
                .write(b"Content type not supported")?;
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
) -> Result<T, CustomError> {
    // Read request body
    let mut body = Vec::<u8, 256>::new();
    let mut buffer = [0u8; 128];
    loop {
        match request.read(&mut buffer) {
            Ok(0) => break, // No more data to read
            Ok(n) => body.extend_from_slice(&buffer[..n]).unwrap(),
            Err(e) => {
                return Err(CustomError::Unknown);
            }
        }
    }

    // Parse the JSON body
    let json_body = serde_json::from_slice::<T>(&body)?;
    Ok(json_body)
}
