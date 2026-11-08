use std::io;
use std::path::Path;
use tokio::io::{AsyncRead, AsyncWrite};

pub trait ExecutorIo: AsyncRead + AsyncWrite + Send + Unpin {}
impl<T: AsyncRead + AsyncWrite + Send + Unpin> ExecutorIo for T {}

pub fn local_executor_endpoint(root: &Path) -> String {
    #[cfg(unix)]
    {
        root.join("local/node_daemon/executor/computer.sock")
            .display()
            .to_string()
    }
    #[cfg(windows)]
    {
        use sha2::{Digest, Sha256};
        format!(
            r"\\.\pipe\buckyos-computer-{:x}",
            Sha256::digest(root.to_string_lossy().as_bytes())
        )
    }
}

pub async fn connect_local_executor(endpoint: &str) -> io::Result<Box<dyn ExecutorIo>> {
    #[cfg(unix)]
    {
        Ok(Box::new(tokio::net::UnixStream::connect(endpoint).await?))
    }
    #[cfg(windows)]
    {
        if !endpoint.starts_with(r"\\.\pipe\") {
            return Err(io::Error::new(
                io::ErrorKind::InvalidInput,
                "local named pipe required",
            ));
        }
        Ok(Box::new(
            tokio::net::windows::named_pipe::ClientOptions::new().open(endpoint)?,
        ))
    }
}

pub struct LocalExecutorListener {
    #[cfg(unix)]
    listener: tokio::net::UnixListener,
    #[cfg(windows)]
    listener: tokio::net::windows::named_pipe::NamedPipeServer,
    #[cfg(windows)]
    endpoint: String,
}

impl LocalExecutorListener {
    pub async fn bind(endpoint: &str) -> io::Result<Self> {
        #[cfg(unix)]
        {
            use std::os::unix::fs::{FileTypeExt, MetadataExt, PermissionsExt};
            let path = Path::new(endpoint);
            if let Some(parent) = path.parent() {
                std::fs::create_dir_all(parent)?;
            }
            match std::fs::symlink_metadata(path) {
                Ok(meta) => {
                    if !meta.file_type().is_socket() || meta.uid() != unsafe { libc::geteuid() } {
                        return Err(io::Error::new(
                            io::ErrorKind::PermissionDenied,
                            "IPC path is not our socket",
                        ));
                    }
                    match tokio::net::UnixStream::connect(path).await {
                        Ok(_) => {
                            return Err(io::Error::new(
                                io::ErrorKind::AddrInUse,
                                "IPC listener already running",
                            ))
                        }
                        Err(e) if e.kind() == io::ErrorKind::ConnectionRefused => {
                            std::fs::remove_file(path)?
                        }
                        Err(e) => return Err(e),
                    }
                }
                Err(e) if e.kind() == io::ErrorKind::NotFound => {}
                Err(e) => return Err(e),
            }
            let listener = tokio::net::UnixListener::bind(path)?;
            std::fs::set_permissions(path, std::fs::Permissions::from_mode(0o600))?;
            Ok(Self { listener })
        }
        #[cfg(windows)]
        {
            let listener = tokio::net::windows::named_pipe::ServerOptions::new()
                .first_pipe_instance(true)
                .reject_remote_clients(true)
                .create(endpoint)?;
            Ok(Self {
                listener,
                endpoint: endpoint.into(),
            })
        }
    }

    pub async fn accept(&mut self) -> io::Result<Box<dyn ExecutorIo>> {
        #[cfg(unix)]
        {
            Ok(Box::new(self.listener.accept().await?.0))
        }
        #[cfg(windows)]
        {
            self.listener.connect().await?;
            let next = tokio::net::windows::named_pipe::ServerOptions::new()
                .reject_remote_clients(true)
                .create(&self.endpoint)?;
            Ok(Box::new(std::mem::replace(&mut self.listener, next)))
        }
    }
}
