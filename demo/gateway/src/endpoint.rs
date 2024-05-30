#[derive(Debug)]
pub enum Protocol {
    TCP,
}

#[derive(Debug)]
pub struct DeviceEndPoint {
    pub device_name: String,
    pub port: Option<u16>,
    pub protocol: Option<Protocol>,
    pub nat_id:Option<String>
}

impl DeviceEndPoint {
    pub fn from_str(s: &str) -> Result<Self, String> {
        //str is like "[tcp://]device_name[:port][@nat_id]"
        let mut parts = s.split('@');
        let device_name = parts.next().unwrap().to_string();
        let mut port = None;
        let mut protocol = None;
        let mut nat_id = None;
        if let Some(device_name) = device_name.strip_prefix("tcp://") {
            protocol = Some(Protocol::TCP);
        } else {
            protocol = Some(Protocol::TCP);
        }
        let mut parts = device_name.split(':');
        let device_name = parts.next().unwrap().to_string();
        if let Some(port_str) = parts.next() {
            port = Some(port_str.parse().map_err(|_| "Invalid port")?);
        }
        if let Some(nat_id_str) = parts.next() {
            nat_id = Some(nat_id_str.to_string());
        }

        Ok(Self {
            device_name,
            port,
            protocol,
            nat_id
        })       
    }
}