// how to test
// run cyfs-gateway1 @ vps with public ip
// run cyfs-gateway2 @ local with lan ip
// let cyfs-gateway2 connect to cyfs-gateway1
// config tcp://cyfs-gateway1:9000 to rtcp://cyfs-gateway2:8000
// then can acess http://cyfs-gateway1.w3.buckyos.io:9000 like access http://cyfs-gateway2:8000

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{rtcp::*, TunnelBuilder};
    use std::time::Duration;
    use buckyos_kit::*;
    use name_client::*;
    use name_lib::*;
    use url::Url;
    use std::net::IpAddr;
    #[tokio::test]
    async fn test_rtcp_url() {
        init_logging("test_rtcp_tunnel",false);

        let url1 = "dev02";
        let target1 = parse_rtcp_stack_id(url1);
        info!("target1: {:?}", target1);
        let url2 = "dev02.devices.web3.buckyos.io:3080";
        let target2 = parse_rtcp_stack_id(url2);
        info!("target2: {:?}", target2);
        let url3 = "LBgzvFCD4VqQxTsO2LCZjs9FPVaQV2Dt0Q5W_lr4mr0.dev.did@LBgzvFCD4VqQxTsO2LCZjs9FPVaQV2Dt0Q5W_lr4mr0.dev.did";
        let target3 = parse_rtcp_stack_id(url3);
        info!("target3: {:?}", target3);
        let url5 = "dev02.devices.web3.buckyos.io:9000";
        let target5 = parse_rtcp_stack_id(url5);
        info!("target5: {:?}", target5);
    }

    //#[tokio::test]
    async fn test_rtcp_tunnel() {
        //rtcp tunnel setup quick start:
        //1. create client rtcp stack(device default rtcp stack)
        //2. get remote rtcp stack id ,like hostname or did (did include public key)
        //    if use hostname , must have did_document
        //3. create tunnel with remote rtcp stack id
        //4. use stream_url like rtcp://$stack_id/:$port to connect to remote tcp server at remote device
        //5  use stream_url like rtcp://$stack_id/google.com:443 to use remote device as a tcp proxy
        std::env::set_var("BUCKY_LOG", "debug");
        init_logging("test_rtcp_tunnel",false);
        let web3_bridge_config = get_default_web3_bridge_config();
        init_name_lib(&web3_bridge_config).await.unwrap();
        //1. create client rtcp stack(device default rtcp stack)
        let (sk, sk_pkcs) = generate_ed25519_key();
        let pk = encode_ed25519_sk_to_pk_jwk(&sk);
        let pk_str = serde_json::to_string(&pk).unwrap();
        let mut name_info =
            NameInfo::from_address("dev02", IpAddr::V4("127.0.0.1".parse().unwrap()));
        name_info.did_document = Some(EncodedDocument::Jwt(pk_str.clone()));
        //add_nameinfo_cache("dev02", name_info).await.unwrap();


        //add_did_cache("dev01", EncodedDocument::Jwt(pk_str.clone())).await.unwrap();
        //add_did_cache("dev02", EncodedDocument::Jwt(pk_str.clone())).await.unwrap();
        let did1 = DID::new("dev","dev01");
        let mut local_stack = RTcpStack::new(did1, 8000, Some(sk_pkcs.clone()));
        local_stack.start().await.unwrap();

        let did2 = DID::new("dev","dev02");
        let dev02_hostname = did2.to_string();
        add_nameinfo_cache(&dev02_hostname, name_info).await.unwrap();
        let mut remote_stack = RTcpStack::new(did2, 9000, Some(sk_pkcs.clone()));
        remote_stack.start().await.unwrap();

        let remote_stack_id = format!("{}:9000", dev02_hostname);
        let tunnel = local_stack.create_tunnel(Some(remote_stack_id.as_str())).await.unwrap();
        tokio::time::sleep(Duration::from_secs(1)).await;
        let stream = tunnel.open_stream(":8888").await.unwrap();
        info!("stream1 ok ");
        tokio::time::sleep(Duration::from_secs(5)).await;

        return;
        // let tunnel_url = Url::parse("rtcp://8000@dev01").unwrap();
        // let tunnel2 = tunnel_builder2.create_tunnel(&tunnel_url).await.unwrap();
        // let stream2 = tunnel2.open_stream(7890).await.unwrap();
        // info!("stream2 ok ");
        // tokio::time::sleep(Duration::from_secs(20)).await;

        // test rudp with dev01 and dev02
        //let tunnel_url = Url::parse("rudp://dev02").unwrap();

        //let data_stream = tunnel.create_datagram_client(1000).await.unwrap();

        //let data_stream2 = tunnel2.create_datagram_server(1000).await.unwrap();
    }
}
