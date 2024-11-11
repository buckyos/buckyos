import { kRPCClient } from "./krpc_client";
import { AuthClient } from "./auth_client";
import { set_zone_host_name,add_web3_bridge, get_zone_host_name, get_verify_rpc_url } from "./toolbox";


function init_buckyos(zone_host_name:string) {
    set_zone_host_name(zone_host_name);
}

const buckyos = {
    kRPCClient,
    AuthClient,
    init_buckyos,
    add_web3_bridge,        
    get_zone_host_name,
    get_verify_rpc_url
}

export default buckyos;

