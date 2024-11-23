use anyhow::Result;
use esp_idf_svc::mdns::EspMdns;

const MDNS_SERVICE_NAME: &str = "_efm";
const MDNS_SERVICE_PROTOCOL: &str = "_tcp";
const MDNS_SERVICE_PORT: u16 = 80;

pub fn init(hostname: &str) -> Result<EspMdns> {
    log::info!("Initialize mDNS");
    log::info!("Set mDNS hostname to {hostname}");
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
