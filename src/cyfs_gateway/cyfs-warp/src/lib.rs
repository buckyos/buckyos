
mod router;
mod http_server;


pub use router::*;
pub use http_server::*;

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