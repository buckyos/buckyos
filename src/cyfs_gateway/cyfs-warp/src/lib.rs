#![allow(dead_code)]

mod router;
mod http_server;
mod ndn_router;


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
  "hosts": {
    "another.com": {
      "routes": {
        "/": {
          "upstream": "http://localhost:9090"
        }
      }
    },
    "example.com": {
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
        let start_result = start_cyfs_warp_server(warp_config).await;
        println!("result: {:?}", start_result);
        assert!(start_result.is_ok());
    }
}