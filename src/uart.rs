use anyhow::{Context, Result};
use esp_idf_svc::hal::{
    gpio::{AnyIOPin, InputPin, OutputPin},
    prelude::*,
    uart,
};

pub fn init(
    uart1: uart::UART1,
    tx: impl OutputPin,
    rx: impl InputPin,
) -> Result<uart::UartDriver<'static>> {
    let config = uart::config::Config::default().baudrate(Hertz(115_200));
    let uart: uart::UartDriver = uart::UartDriver::new(
        uart1,
        tx,
        rx,
        Option::<AnyIOPin>::None,
        Option::<AnyIOPin>::None,
        &config,
    )
    .context("Failed to create uart driver")?;
    Ok(uart)
}
