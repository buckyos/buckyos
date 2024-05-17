

use tokio::io::{AsyncRead, AsyncWrite};


#[async_trait::async_trait]
pub trait Tunnel: Send + Unpin + AsyncRead + AsyncWrite {
    fn split(self: Box<Self>) -> (Box<dyn TunnelReader>, Box<dyn TunnelWriter>);

    /*
    async fn run_forward(&mut self, forward: String) -> GatewayResult<()> {
        let mut stream = TcpStream::connect(&forward).await.map_err(|e| {
            error!("Error connecting to forward address {}: {}", forward, e);
            e
        })?;

        let (mut tunnel_reader, mut tunnel_writer) = self.split();
        let (mut stream_reader, mut stream_writer) = stream.split();

        let tunnel_to_stream = tokio::io::copy(&mut tunnel_reader, &mut stream_writer);
        let stream_to_tunnel = tokio::io::copy(&mut stream_reader, &mut tunnel_writer);

        tokio::try_join!(tunnel_to_stream, stream_to_tunnel).unwrap();

        Ok(())
    }
    */
}

pub trait TunnelReader: Send + Unpin + AsyncRead {}

pub trait TunnelWriter: Send + Unpin + AsyncWrite {}

impl<T: AsyncRead + Unpin + Send> TunnelReader for T {}
impl<T: AsyncWrite + Unpin + Send> TunnelWriter for T {}

// impl<T: AsyncRead + AsyncWrite + Unpin + Send> Tunnel for T {}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TunnelType {
    Tcp,
    Udp,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TunnelSide {
    Active,
    Passive,
}


/* 
pub struct Tunnel {
    id: String,

    // tunnel info
    tunnel_type: TunnelType,
    server: String,
    // tunnel: Arc<Mutex<Box<dyn Tunnel>>>,

    // local forward address
    forward: String,
}

impl Tunnel {
    pub fn new(id: String, tunnel_type: TunnelType, server: String, forward: String) -> Self {
        Self {
            id,
            tunnel_type,
            server,
            forward,
        }
    }

    pub fn id(&self) -> &str {
        &self.id
    }

    pub fn tunnel_type(&self) -> TunnelType {
        self.tunnel_type
    }

    /*
    load from json array as below
    {
        block: "tunnel",
        type: "tcp",
        id: "local-service-1",
        server: "device-id:port"，
        forward: "127.0.0.1:9000"
    }
    */
    pub fn load(&self, json: &serde_json::Value) -> GatewayResult<Self> {
        let id = json["id"]
            .as_str()
            .ok_or(GatewayError::InvalidConfig("id"))?;
        let tunnel_type = match json["type"].as_str() {
            Some("tcp") => TunnelType::Tcp,
            Some("udp") => TunnelType::Udp,
            _ => return Err(GatewayError::InvalidConfig("type")),
        };
        let server = json["server"]
            .as_str()
            .ok_or(GatewayError::InvalidConfig("server"))?;
        let forward = json["forward"]
            .as_str()
            .ok_or(GatewayError::InvalidConfig("forward"))?;

        Ok(Self::new(
            id.to_string(),
            tunnel_type,
            server.to_string(),
            forward.to_string(),
        ))
    }

    pub async fn run(&self) -> GatewayResult<()> {
        loop {
            match self.build().await {
                Ok(_) => {
                    info!("Tunnel {} closed", self.id);
                    break;
                }
                Err(e) => {
                    error!("Tunnel error: {} {}", self.id, e);

                    // slelp 5 seconds and try again
                    tokio::time::sleep(tokio::time::Duration::from_secs(5)).await;
                }
            }
        }

        Ok(())
    }

    async fn build(&self) -> GatewayResult<()> {
        info!(
            "now will build tunnel connection {} {} <---> {}",
            self.id, self.forward, self.server
        );

        // try build tunnel connection
        let (mut tunnel_reader, mut tunnel_writer) = match self.tunnel_type {
            TunnelType::Tcp => {
                let tunnel = TcpTunnel::build(self.server.clone()).await?;
                tunnel.split()
            }
            TunnelType::Udp => {
                unimplemented!()
            }
        };

        // try to connect to forward address
        let mut stream: TcpStream = TcpStream::connect(&self.forward).await.map_err(|e| {
            error!(
                "Error connecting to forward address {}: {}",
                self.forward, e
            );
            e
        })?;

        // split tunnel and stream
        let (mut stream_reader, mut stream_writer) = stream.split();

        let tunnel_to_stream = tokio::io::copy(&mut tunnel_reader, &mut stream_writer);
        let stream_to_tunnel = tokio::io::copy(&mut stream_reader, &mut tunnel_writer);

        tokio::try_join!(tunnel_to_stream, stream_to_tunnel).map_err(|e| {
            error!("Error running tunnel: {} {}", self.id, e);
            e
        })?;

        Ok(())
    }

    async fn build_tunnel(&self) -> Result<Box<dyn Tunnel>, GatewayError> {
        match self.tunnel_type {
            TunnelType::Tcp => {
                let tunnel = TcpTunnel::build(self.server.clone()).await?;
                Ok(Box::new(tunnel))
            }
            TunnelType::Udp => {
                unimplemented!()
            }
        }
    }
}
*/