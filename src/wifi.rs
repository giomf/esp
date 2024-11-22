use anyhow::Result;
use embedded_svc::wifi;
use esp_idf_svc::{
    eventloop::EspSystemEventLoop,
    hal::modem::Modem,
    nvs::EspDefaultNvsPartition,
    timer::EspTaskTimerService,
    wifi::{AsyncWifi, AuthMethod, ClientConfiguration, EspWifi, WifiDeviceId},
};

pub struct Wifi<'a> {
    mac_address: [u8; 6],
    wifi: AsyncWifi<EspWifi<'a>>,
}

impl<'a> Wifi<'a> {
    pub fn new(event_loop: EspSystemEventLoop, modem: Modem) -> Result<Self> {
        let driver = EspWifi::new(
            modem,
            event_loop.clone(),
            Some(EspDefaultNvsPartition::take()?),
        )?;
        let mac_address = driver.get_mac(WifiDeviceId::Sta)?;
        let timer_service = EspTaskTimerService::new()?;
        let wifi = AsyncWifi::wrap(driver, event_loop, timer_service)?;

        let result = Self { wifi, mac_address };

        Ok(result)
    }

    pub async fn connect(&mut self, ssid: &str, password: &str) -> Result<()> {
        let configuration = wifi::Configuration::Client(ClientConfiguration {
            ssid: ssid.try_into().unwrap(),
            auth_method: AuthMethod::WPA2Personal,
            password: password.try_into().unwrap(),
            channel: None,
            ..Default::default()
        });

        self.wifi.set_configuration(&configuration)?;

        self.wifi.start().await?;

        while let Err(err) = self.wifi.connect().await {
            log::error!("Error connecting to wifi {err}! Retrying.");
        }

        self.wifi.wait_netif_up().await?;
        Ok(())
    }

    pub fn get_mac_address(&self) -> [u8; 6] {
        self.mac_address
    }
}
