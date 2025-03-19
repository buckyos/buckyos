## 理解ndn_client.get_obj_by_url

该函数的实现是分离 服务器实现和客户端验证，并不需要保障服务器一旦支持NDN，就能通过O Link获得所有的Object
- URL是O Link: URL中包含ObjId和inner_path,能在http环境下提供可靠的verify
- URL是R Link: cyfs-http-resp中包含有新的验证信息，能在http环境下提供可靠的verify,

### inner_path优化

CYFS之前：

```python
# 该方案的问题
# - 为了获得一个对象，需要发起2次请求 （服务器需要支持两个URL可访问，第二个URL还是构造的）
# - parent_obj如果是一个容器对象，可能非常大

parent_obj = get_obj_by_url(parent_obj_id)
child_obj_id = parent_obj.get_inner_path(inner_path)
child_obj = get_obj_by_id(child_obj_id)
```

引入inner_path后：

```python
obj_str,parent_obj_id,cyfs_http_resp_header = get_obj_by_url(url_with_inner_path)
# 验证parent_obj_id存在于指定路径
verify(cyfs_http_resp_header.path_obj,parent_obj_id)
obj_id = gen_obj_id(obj_str)
# 通过默克尔树路径验证objid属于parent obj
verify_inner_path(cyfs_http_resp_header.obj_id,cyfs_http_resp_header.mtree_path,obj_id)
return obj_str
```
上述逻辑的优点
- 只需要一次网络请求就可以得到想要的obj
- 服务器只暴露 `指向最终obj的，包含inner_path` 的URL即可
- 服务器不必发布parent_obj,客户端也不需要缓存可能非常巨大的parent_obj

TODO : 目前设计，只支持 parent_obj.inner_path格式，是否可以支持parent_obj.inner_path.inner_path格式?
需要对cyfs_http_resp_header.mtree_path 支持多层结构,提供 parent_objid.mtree_path.mtree_path ... 的结构

## 伪代码实现

```rust
fn verify_obj(obj_id,obj_str) {
    json_obj = json.parse(obj_str)
    if gen_obj_id(json_obj) == obj_id {
        return json_obj
    }
    return Err;
}


pub fn get_obj_by_url(url,known_obj_id) {
    obj_id,inner_path = obj_id_from(url);
    obj_str,cyfs_resp_header = http_client.get(url)

    //下面是验证流程
    if known_obj_id {
        if inner_path == none {
            if obj_id? != known_obj_id {}
                return "verify error"
            }
        }
        obj = verify_obj(known_obj_id,obj_str)?
        return obj
    }

    if obj_id { //O Link 
        if inner_path == none {
            obj = verify_obj(obj_id,obj_str)?
            return obj
        } else {
            // 有inner_path的O Link 比如: http://test.buckyos.org/ndn/pub/$container_obj_id/pkgs/app_xxx/linux.tar.gz
            //O Link中的obj_id是root_obj_id
            if obj_id != cyfs_http_resp.root_obj_id {
                return "verify error"
            }

            obj = verify_obj(cyfs_http_resp.obj_id,obj_str)?
            //cyfs_http_resp.root_obj_id.inner_path = cyfs_http_resp.obj_id, 用cyfs_http_resp.mtree_path 证明
            verify_inner_path(cyfs_http_resp.root_obj_id,inner_path,cyfs_http_resp.mtree_path,cyfs_http_resp.obj_id)？
            return obj
        }
    } else {
        //下面是R Link （语义链接）流程了。
        obj = verify_obj(cyfs_resp_header.obj_id,obj_str)?  //这里验证失败就结束了
        if cyfs_resp_header.root_obj_id {
            cache_path_obj = local_cache.get(cyfs_resp_header.path_obj.path)
            inner_path = url.get_relative(cyfs_resp_header.path_obj.path)
            verify_inner_path(cyfs_http_resp.root_obj_id,inner_path,cyfs_http_resp.mtree_path,cyfs_http_resp.obj_id)？
        } else {
             cache_path_obj = local_cache.get(url)
        } 

        //验证R Link是否真的指向 obj 或 parent_obj
        if url.is_https: // R Link 本质上还是来源可信，因此https的保障足够了
            return obj
        if cache_path_obj == cyfs_resp_header.path_obj // 通过cache发现之前已经验证过了
            return obj
        if cache_path_obj.update_time > cyfs_resp_header.path_obj.update_time //这里潜在的依赖了客户端和服务器都有正确的时间
            return "verifiy error"
        // 对path_obj进行验证
        path_obj_jwt = cyfs_resp_header.path_obj // path_obj可以只是uri的一部分，此时url的剩下部分就是inner_path了
        pk = name_client.resolve_key(url.host) // pk 这里一般都有cache
        verify_jwt(path_obj_jwt,pk)?// path_obj必须有正确的签名
        local_cache.update(url,path_obj)
        return obj
    }
}

fn verify_inner_path(parent_obj_id,inner_path,mtree_path,targe_obj_id) {
    //parent_obj_id是root hash
    //下面实现可以参考Patricia Trie的构建流程 ,mtree_path里已经包含了必要的路径hash信息
    root_hash = cacle_mtree_root_hash(target_obj_id,inner_path,mtree_path)
    return root_hash == parent_obj_id
}

```
## 通过URL下载Chunk的验证问题

1. 用户明确的知道URL指向的是一个ChunkId,但并不知道这个ChunkId是什么 （R Link)
2. 用户明确的知道URL指向的是一个ChunkId，并且知道这个ChunkId的确切值
3. 用户明确的知道URL指向的是一个ChunkId，并计划通过URL推算出ChunkId

