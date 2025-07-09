#![allow(unused)]

mod dns_server;

pub use dns_server::*;


#[cfg(test)]
mod test {
    use super::*;
    use cyfs_gateway_lib::*;
    use serde_json::*;

    #[tokio::test]
    async fn test_cyfs_dns_main() {
       
        let config_str = r#"
{
  "port":2053,
  "resolver_chain": [
    {
      "type": "dns",
      "cache": true
    }
  ],
  "fallback": []
}
    "#;
        let dns_config:DNSServerConfig = serde_json::from_str(config_str).unwrap();
        
        let start_result = start_cyfs_dns_server(dns_config).await;
        //println!("result: {:?}", start_result);
        assert!(start_result.is_ok());
    }
}