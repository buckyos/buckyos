# cyfs protocol

## 从Chunk开始

我们使用sha256sum对一个文件进行计算，可以得到  
```
d1127e660d0de222a3383609d74ff8d4b4ba97a226f861e184e1f92eee25d3b9  README.md
```
此时，README.md文件的ChunkId为 sha256:d1127e660d0de222a3383609d74ff8d4b4ba97a226f861e184e1f92eee25d3b9。
其格式为 `{hashtype}:{hash_data_hex_text}`
上述ChunkId还有另一种base32的编码方式
```
base32_rfc4648_encode("sha256:" + hash_data)
```
使用buckycli工具可以得到结果
```
buckycli chunkid --encode base32 --input sha256:d1127e660d0de222a3383609d74ff8d4b4ba97a226f861e184e1f92eee25d3b9
```

在系统中使用上述两种chunkid来表达一个`不再修改的文件` 是等价的。

```
buckycli chunkid --encode hex --input {base32}
```
可以将base32 encode的chunkid转换为sha256

### 通过ChunkId来可靠的获取文件
在支持cyfs://的zone中我们可以使用3个常见的url来获取上述README.md

```
http://$zoneid/ndn/{sha256}
http://$zoneid/ndn/{base32}
http://{base32}.$zoneid/ （一般不会默认enable)
```

在标准浏览器里，使用http会触发“不安全警告”，但其实只要浏览器完整的支持了cyfs://,即使使用http也是安全的，因为浏览器完全可以基于URL中的hash信息对获得的数据进行校验，校验通过后该内容就是安全可靠的。通过`ndn_client.open_chunk_reader_by_url` + `copy_chunk`(目前该组件只有rust SDK支持)打开上述URL，就会在下载完成后对内容进行验证。

通过上述简单的流程，我们可以理解cyfs://的关键设计理念：
1. “已发布内容”是互联网上重要的一类数据，这类数据在发布后完全公开并且不再修改
2. “已发布内容”在cyfs://中被称作NamedObject,拥有Named Object Id (ObjectId),根据计算Hash的方法不同，ObjectId有不同的类型。我们可用 {objtype}:{hash_data} 可扩展的表达ObjectId
3. cyfs:// GET协议于HTTP GET的不同在于：客户端在发起请求时已经知晓Named Object Id,这样可以不依赖TLS的可信传输对获得的数据进行校验
4. cyfs://通过改进http:// 协议而不是https:// 协议来传输 NamedObject是cyfs协议的关键设计目标，因为http:// 的明文特性，可以更好的实现分布式CDN，减少404，提高传输速度，优化网络性能。

### 在现有http://协议上 增加ChunkId支持
上述流程能成功工作，有一个核心的前提`客户端在发起请求时已经知晓Named Object Id`，但并不是所有的URL中都适合带上编码后的ObjectId,这类传统的URL在cyfs://中又被称作"语义URL" (URL本身的内容是有意义的)，其指向一个逻辑路径，逻辑路径对应的已“发布内容”是允许改变的。 通过语义URL获得Named Object,理论上可以分为两步

第一步：通过语义URL得到Object Id
第二步：基于Object Id，使用前述流程获取完整的Named Object.

从实现简单的角度，我们可以要求先用传统的URL https://$zoneid/buckyos/readme.md 获得readme.md文件的chunkid,在用 http://$zoneid/ndn/$chunkid 来可信的获得readme.md的内容。cyfs://的设计是解耦的，并不反对实现上述流程。我们也相信，也许对有些系统来说，通过上述方法来集成cyfs://，实现减少https使用的目的，可能是最简单快捷。

能不能在一次http请求类完成？我们通过下面方法在一次返回中携带数据和用于验证的信息
1. 在HTTP头中以某种可验证的方法，说明该语义URL指向ChunkId
2. 继续返回文件内容，此时客户端可以根据上一步验证的

支持cyfs扩展的 http resp如下
```

cyfs-obj-id:
cyfs-path-obj:

<body>
```
cyfs-path-obj 这个扩展的http resp header是关键。其内容是一个JWT（签名的Json对象），解码后其内容如下
```
{
    "path": "/buckyos/readme.txt",
    "target":"sha256:xxxx",
    "uptime":232332,
    "exp":132323,
}
```
内容非常纯粹，就是说明一个路径（不含域名）指向的NamedObject的ObjId是什么，这个绑定信息什么时候过期（JWT规范的强制要求),这个绑定关系是什么时候上线的(uptime)

在使用target object id前，需要对path_obj (JWT) 进行验证
0. 获得可信的公钥
1. 使用公钥对JWT进行验证,确定该Path确实是指向objid的
2. 与cache的PathObject（如有）进行时间戳比较，防止重放攻击

获得可信的公钥的流程是解耦的。cyfs://协议通过一个可扩展的框架，我们目前支持下面3种方法来得到验证PathObj的公钥

- 将公钥保存在dns record里，适用于完全没有https证书的情况
- 使用https://$zoneid/this_zone 获得公钥 适用于有https证书，希望减少Https流量的使用
- 使用bns(智能合约)来查询zoneid对应的可信公钥 适用于完全没有https证书的情况，需要客户端有能力读取智能合约的状态

服务的提供者，可以根据自己的实际情况，基于兼容性和性能的考虑来选择上述方案

## PathObject是NamedObject

构造可验证的PathObject的过程，分为下面两步
1. PathObject JSON进行稳定编码并计算Hash
2. 对Hash进行签名

因为有计算hash的过程，所以任何一个json都可以named object化并得到一个ObjectId.
```

pub fn build_obj_id(obj_type:&str,obj_json_str:&str)->ObjId {
    let vec_u8 = obj_json_str.as_bytes().to_vec();
    let hash_value:Vec<u8> = Sha256::digest(&vec_u8).to_vec();
    ObjId::new_by_raw(obj_type.to_string(),hash_value)
}

pub fn build_named_object_by_json(obj_type:&str,json_value:&serde_json::Value)->(ObjId,String) {
      
        fn stabilize_json(value: &serde_json::Value) -> serde_json::Value {
            match value {
                serde_json::Value::Object(map) => {
                    let ordered: BTreeMap<String, serde_json::Value> = map.iter()
                        .map(|(k, v)| (k.clone(), stabilize_json(v)))
                        .collect();
                    serde_json::Value::Object(serde_json::Map::from_iter(ordered))
                }
                serde_json::Value::Array(arr) => {
                    // 递归处理数组中的每个元素
                    serde_json::Value::Array(
                        arr.iter()
                            .map(stabilize_json)
                            .collect()
                    )
                }
                // 其他类型直接克隆
                _ => value.clone(),
            }
        }

        let stable_value = stabilize_json(json_value);
        let json_str = serde_json::to_string(&stable_value)
            .unwrap_or_else(|_| "{}".to_string());
        let obj_id = build_obj_id(obj_type,&json_str);
        (obj_id,json_str)
}

```

基于上述流程，可以得到下面结论
- 通过Named Object Id可以获得一个可验证的json.
- 在使用相同的稳定编码算法时，相同语义的json每次都会编码得到相同的Named Object Id

通过HTTP协议获得一个NamedObject被称作cyfs://里获得结构化化数据的部分。从接口语义上看我们总是假设named object不太大大，通过一个原子的GET行为即可完成获取。而打开一个Chunk(Named Data)通常是OpenStream,需要有断点续传等更复杂的支持。

因为这种接口语义的不同，ndn_client提供了两类接口来区分的处理NamedObject和NamedData. 这说明即使我们不知道一个URL具体指向什么数据，但必须知道URL指向数据（内容）的类型。

## 使用FileObject而不是Chunk

很多时候直接使用chunk来发布内容并不是很方便，应为我们总是需要在发布内容的同时，也发布一些基础的元信息。比如chunk的大小（并不是所有类型的chunkid都能直接得到chunk的大小），文件名，内容的MIME类型等信息。因此，我们可以发布一个包含必要原信息的的NamedObject,在这个NamedObject的元数据中去引用Chunk。cyfs定义了一个这样的标准对象FileObject，一个典型的FileObject如下
```json
{

}
```

编码后得到的objid为: `cyfile:513788234cfb679121c148ba4fd768390bf948bfb17d6cfced79b205d5c82c9d`
cyfile是cyfs://里定义的标准对象，标准对象约定了一些字段的含义和是否可选。得益于json的可扩展性，用户都可以在这个定义的基础上扩展自己的自定义字段。

通过FileObject发布Chunk后，我们可以通过下面流程完成文件的下载
```
fileObj = get_obj_by_url()
chunk_reader = open_chunk_by_url(fileObj.chunkId)
```
非常简单，但，这需要与服务器通讯两次。能只通信一次么？

```
chunk_reader = open_chunk_by_url("http://$zone_id/readme.md/content") 
```
即可,此时 /content被称作inner_path,cyfs协议要求支持的服务器在处理URL请求时，检查$file_obj_id的content字段(根据json path规范)的值。如果值是非ObjId,则返回该值。否则返回该ObjId指向的对象。从协议设计的角度，允许在一个URL执行多次inner_path解析，不过从减少服务器内存开销的角度考虑，目前协议规范只要求支持1层即可。

在引入inner_path后，我们依旧可以通过cyfs_header对返回的结果进行验证。



## 使用Named Object Container


核心设计在于：可以通过mtree的理论，在信任container objectid的情况，相信
container[key] = target_obj_id

因此在请求
http://$zoneid/$container_inner_path/key 时，返回

```
cyfs-obj-id;$target_obj_id
cyfs-root-obj-id:$container_id
cyfs-path-proof:$proof-data

<body: target obj data>

```

## 扩展Hash算法数据进行分块

（对大Chunk的分块下载）

## 对历史版本进行追踪


## 使用cyfs://来建立知识图谱









