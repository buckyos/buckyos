// how to test
// run cyfs-gateway1 @ vps with public ip
// run cyfs-gateway2 @ local with lan ip
// let cyfs-gateway2 connect to cyfs-gateway1
// config tcp://cyfs-gateway1:9000 to rtcp://cyfs-gateway2:8000
// then can acess http://cyfs-gateway1.w3.buckyos.io:9000 like access http://cyfs-gateway2:8000

#[cfg(test)]
mod tests {
    use super::*;
    use buckyos_kit::*;
    use name_client::*;
    use name_lib::*;
    use url::Url;
    #[tokio::test]
    async fn test_rtcp_url() {
        init_logging("test_rtcp_tunnel");

        let url1 = "rtcp://dev02";
        let target1 = parse_rtcp_url(url1);
        info!("target1: {:?}", target1);
        let url2 = "rtcp://dev02.devices.web3.buckyos.io:3080/";
        let target2 = parse_rtcp_url(url2);
        info!("target2: {:?}", target2);
        let url3 = "rtcp://9000@LBgzvFCD4VqQxTsO2LCZjs9FPVaQV2Dt0Q5W_lr4mr0.dev.did/";
        let target3 = parse_rtcp_url(url3);
        info!("target3: {:?}", target3);
        let url4 = "rtcp://8000@LBgzvFCD4VqQxTsO2LCZjs9FPVaQV2Dt0Q5W_lr4mr0.dev.did:3000/";
        let target4 = parse_rtcp_url(url4);
        info!("target4: {:?}", target4);
        let url5 = "rtcp://dev02.devices.web3.buckyos.io:9000/snkpi/323";
        let target5 = parse_rtcp_url(url5);
        info!("target5: {:?}", target5);
    }

    #[tokio::test]
    async fn test_rtcp_tunnel() {
        init_logging("test_rtcp_tunnel");
        init_default_name_client().await.unwrap();

        let (sk, sk_pkcs) = generate_ed25519_key();

        let pk = encode_ed25519_sk_to_pk_jwt(&sk);
        let pk_str = serde_json::to_string(&pk).unwrap();

        let mut name_info =
            NameInfo::from_address("dev02", IpAddr::V4("127.0.0.1".parse().unwrap()));
        name_info.did_document = Some(EncodedDocument::Jwt(pk_str.clone()));
        add_nameinfo_cache("dev02", name_info).await.unwrap();

        //add_did_cache("dev01", EncodedDocument::Jwt(pk_str.clone())).await.unwrap();
        //add_did_cache("dev02", EncodedDocument::Jwt(pk_str.clone())).await.unwrap();

        let mut tunnel_builder1 = RTcpStack::new("dev01".to_string(), 8000, Some(sk_pkcs.clone()));
        tunnel_builder1.start().await.unwrap();

        let mut tunnel_builder2 = RTcpStack::new("dev02".to_string(), 9000, Some(sk_pkcs.clone()));
        tunnel_builder2.start().await.unwrap();

        let tunnel_url = Url::parse("rtcp://9000@dev02/").unwrap();
        let tunnel = tunnel_builder1.create_tunnel(&tunnel_url).await.unwrap();
        tokio::time::sleep(Duration::from_secs(1)).await;
        let stream = tunnel.open_stream(8888).await.unwrap();
        info!("stream1 ok ");
        tokio::time::sleep(Duration::from_secs(5)).await;

        return;
        let tunnel_url = Url::parse("rtcp://8000@dev01").unwrap();
        let tunnel2 = tunnel_builder2.create_tunnel(&tunnel_url).await.unwrap();
        let stream2 = tunnel2.open_stream(7890).await.unwrap();
        info!("stream2 ok ");
        tokio::time::sleep(Duration::from_secs(20)).await;

        // test rudp with dev01 and dev02
        //let tunnel_url = Url::parse("rudp://dev02").unwrap();

        //let data_stream = tunnel.create_datagram_client(1000).await.unwrap();

        //let data_stream2 = tunnel2.create_datagram_server(1000).await.unwrap();
    }
}
