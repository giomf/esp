use crate::base36;
use anyhow::Result;
use core::convert::TryInto;
use embedded_svc::wifi::{self};
use esp_idf_svc::{
    eventloop::EspSystemEventLoop,
    hal::modem::Modem,
    ipv4::{self, DHCPClientSettings},
    netif::{EspNetif, NetifConfiguration},
    nvs::EspDefaultNvsPartition,
    timer::EspTaskTimerService,
    wifi::{AsyncWifi, AuthMethod, EspWifi, WifiDeviceId, WifiDriver},
};
use heapless::String;

pub struct Wifi<'a> {
    wifi: AsyncWifi<EspWifi<'a>>,
}

impl<'a> Wifi<'a> {
    pub fn new(event_loop: EspSystemEventLoop, modem: Modem) -> Result<Self> {
        log::info!("Initialize wifi");
        let driver = WifiDriver::new(
            modem,
            event_loop.clone(),
            Some(EspDefaultNvsPartition::take()?),
        )?;
        let mac_address = driver.get_mac(WifiDeviceId::Sta)?;
        let hostname = base36::encode(mac_address);
        log::info!("Set wifi hostname to {hostname}");

        let network_configuration = Wifi::create_network_configuration_with_hostname(&hostname);
        let network_configuration = EspNetif::new_with_conf(&network_configuration)?;

        let wifi = EspWifi::wrap_all(
            driver,
            network_configuration,
            EspNetif::new(esp_idf_svc::netif::NetifStack::Ap)?,
        )?;

        let timer_service = EspTaskTimerService::new()?;
        let wifi = AsyncWifi::wrap(wifi, event_loop, timer_service)?;

        Ok(Self { wifi })
    }

    fn create_network_configuration_with_hostname(hostname: &str) -> NetifConfiguration {
        let hostname: String<30> = String::try_from(hostname).unwrap();
        let mut network_configuration = NetifConfiguration::wifi_default_client();
        let ip_configuration =
            ipv4::Configuration::Client(ipv4::ClientConfiguration::DHCP(DHCPClientSettings {
                hostname: Some(hostname),
            }));
        network_configuration.ip_configuration = Some(ip_configuration);
        network_configuration
    }

    pub async fn connect(&mut self, ssid: &str, password: &str) -> Result<()> {
        log::info!("Connect to wifi {}", ssid);
        let configuration = wifi::Configuration::Client(wifi::ClientConfiguration {
            ssid: ssid.try_into().unwrap(),
            auth_method: AuthMethod::WPA2Personal,
            password: password.try_into().unwrap(),
            channel: None,
            ..Default::default()
        });

        self.wifi.set_configuration(&configuration)?;
        log::info!("Start");
        self.wifi.start().await?;

        while let Err(err) = self.wifi.connect().await {
            log::error!("Failed connecting to wifi {err}! Retrying.");
        }

        self.wifi.wait_netif_up().await?;
        Ok(())
    }

    pub fn get_hostname(&self) -> Result<String<30>> {
        Ok(self.wifi.wifi().sta_netif().get_hostname()?)
    }
}
