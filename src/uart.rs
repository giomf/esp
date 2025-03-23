use am03127::{self};
use anyhow::{bail, Context, Result};
use esp_idf_svc::hal::{
    gpio::{AnyIOPin, InputPin, OutputPin},
    prelude::*,
    uart::{
        self,
        config::{DataBits::DataBits8, StopBits},
    },
};

const ID: u8 = 1;
const READ_TIMEOUT: u32 = 64;
const READ_BUFFER_SIZE: usize = 32;

pub struct Uart {
    uart: uart::UartDriver<'static>,
}
impl Uart {
    pub fn new(uart1: uart::UART1, tx: impl OutputPin, rx: impl InputPin) -> Result<Self> {
        let config = uart::config::Config::default()
            .baudrate(Hertz(9600))
            .stop_bits(StopBits::STOP1)
            .data_bits(DataBits8)
            .parity_none();

        let uart: uart::UartDriver = uart::UartDriver::new(
            uart1,
            tx,
            rx,
            Option::<AnyIOPin>::None,
            Option::<AnyIOPin>::None,
            &config,
        )
        .context("Failed to create uart driver")?;
        Ok(Self { uart })
    }

    pub fn init(&self) -> Result<()> {
        log::info!("Initialize panel with ID: {ID}");
        let id_command = am03127::set_id(ID);
        self.write(&id_command)?;
        Ok(())
    }

    pub fn write(&self, command: &str) -> Result<()> {
        let mut buffer = [0; READ_BUFFER_SIZE];
        let _ = self.uart.write(command.as_bytes())?;
        let _ = self.uart.read(&mut buffer, READ_TIMEOUT)?;
        let result = String::from_utf8_lossy(&buffer);

        log::info!("Receiving: {}", &result);
        if result.starts_with("ACK") {
            return Ok(());
        } else if result.starts_with("NACK") {
            bail!("NACK");
        }

        Ok(())
    }
}
