```rust
fn handle_ndn_get(req:HttpRequest) HttpResp {
    let obj_id,inner_path = get_obj_id_from_req(req);
    if obj_id.is_none() {
        let relateve_path = get_relate(config.ndn.path,req.path);
        obj_id,inner_path,path_obj = select_objid_by_path(relatve_path);
    }

    if(obj_id.is_none()) {
        return Err::NotFound;
    }

    if inner_path.is_none() {
        //返回json或reader
        resp_body = load_obj_content(obj_id,query_param)

    } else {
        filed = get_obj_inner(obj_id,inner_path)
        if is_obj_id(filed) {
            resp_body = load_obj_content(filed,query_param)
        } else {
            resp_boduy = filed
        }

    }   

}
```


## cyfs ndn 协议简介

body的类型： field / json / chunk

content-length : body的长度
cyfs-obj-size: 如果返回的是named object,则是返回的NamedObject的大小（>= content-length)
cyfs-path-obj: 如果是基于路径查询返回的obj（get_obj_id_from_req返回none),则该字段存在，为path-obj的jwt,有zone-turst-publisher的签名。注意cyfs-path-obj在无inner path时，指向cyfs-obj-id,有inner_path时，指向cyfs-rootobj-id
cyfs-obj-id:如果返回的是named object,则是named object id,返回的是field则没有该字段？


cyfs-rootobj-id: 如果是基于 root_obj/inner_path 模式返回，则为root-obj-id
cyfs-proof: 如果是基于root_obj/inner_path 模式返回，且root_obj是big_container，则提供必要的证明，说明 root_obj/inner_path = cyfs-obj-id
cyfs-rootobj:如果是基于root_obj/inner_path模式返回，且root_obj不是big_container,那么为root_obj的json 
