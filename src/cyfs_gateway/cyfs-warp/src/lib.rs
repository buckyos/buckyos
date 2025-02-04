#![allow(dead_code)]

mod router;
mod http_server;
mod ndn_router;
mod cert;

pub use router::*;
pub use http_server::*;

use anyhow::Result;

// 辅助函数：解析Range header
pub fn parse_range(range: &str, file_size: u64) -> Result<(u64, u64)> {
  // 解析 "bytes=start-end" 格式
  let range = range.trim_start_matches("bytes=");
  let mut parts = range.split('-');
  
  let start = parts.next()
      .and_then(|s| s.parse::<u64>().ok())
      .unwrap_or(0);

      
  let end = parts.next()
      .and_then(|s| s.parse::<u64>().ok())
      .unwrap_or(file_size - 1);

  // 验证范围有效性
  if start >= file_size || end >= file_size || start > end {
      return Err(anyhow::anyhow!("Invalid range"));
  }

  Ok((start, end))
}

mod test {
    #![allow(unused)]
    use super::*;
    use cyfs_gateway_lib::*;
    use buckyos_kit::*;
    use serde_json::*;

    #[tokio::test]
    async fn test_cyfs_warp_main() {
        let config_str = r#"
{
  "tls_port":3002,
  "http_port":3000,
  "bind":"0.0.0.0",
  "hosts": {
    "another.com": {
      "routes": {
        "1": {
          "upstream": "http://localhost:9090"
        }, 
        "2": {
          "upstream": "http://localhost:9091 redirect"
        },
        "3": {
          "upstream": "http://localhost:9092 redirect permanent"
        }
      }
    },
    "example.com": {
      "tls": {
        "cert_path": "D:\\temp\\cert.pem",
        "key_path": "D:\\temp\\key.pem"
      },
      "routes": {
        "/api": {
          "upstream": "http://localhost:8080"
        },
        "/static": {
          "local_dir": "D:\\temp"
        }
      }
    }
  }
}        
        "#;
        let warp_config:WarpServerConfig = serde_json::from_str(config_str).unwrap();
        //init_logging();
        start_cyfs_warp_server(warp_config).await.unwrap();
        tokio::signal::ctrl_c().await.unwrap();
    }


    #[tokio::test]
    async fn test_cyfs_warp_https() {
        use env_logger;
        env_logger::builder().filter_level(log::LevelFilter::Info).init();
        let config_str = r#"
        {
            "hosts": {
                "dev.photosssa.org": {
                    "redirect_to_https": true,
                    "routes": {
                        "/": {
                            "response": {
                                "status": 200,
                                "headers": {
                                    "Content-Type": "text/html"
                                },
                                "body": "Hello, BuckyOS!"
                            }
                        }
                    }
                }
            }
        }
        "#;
        let warp_config:WarpServerConfig = serde_json::from_str(config_str).unwrap();
        start_cyfs_warp_server(warp_config).await.unwrap();
        tokio::signal::ctrl_c().await.unwrap();
    }


    #[tokio::test]
    async fn test_cyfs_warp_disable_https() {
        use env_logger;
        env_logger::builder().filter_level(log::LevelFilter::Info).init();
        let config_str = r#"
        {
            "hosts": {
                "dev.photosssa.org": {
                    "tls": {
                        "disable_tls": true
                    },
                    "routes": {
                        "/": {
                            "response": {
                                "status": 200,
                                "headers": {
                                    "Content-Type": "text/html"
                                },
                                "body": "Hello, BuckyOS!"
                            }
                        }
                    }
                }
            }
        }
        "#;
        let warp_config:WarpServerConfig = serde_json::from_str(config_str).unwrap();
        start_cyfs_warp_server(warp_config).await.unwrap();
        tokio::signal::ctrl_c().await.unwrap();
    }
}