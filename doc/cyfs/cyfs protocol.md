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

### inner-path规范

1. 使用inner-path在named-obj中进行寻址，逻辑与json-path一致
2. 如果inner-path指向的是objid,则默认返回objid指向的object，否则返回字段的值

## 使用Named Object Container

目的：obj,proof = GET(container_id,key)
当container的元素总量小于（1024）时，将container当成一个标准的named object处理即可，
```
container_json = get_named_obj(container_id)
obj_id = container_json[key]
obj = get_named_obj(obj_id)
```
当conatiner的元素很多时，获取container_json的代价可能会远超过obj本身，此时切换到`容器的部分可验证获取模式`
```
obj_id,path_proof = mtree_get(container_id,key) 
obj = get_named_obj(obj_id)
```
从网络通信和mtree本身的特性来看，还有一个进阶版本
```
//用mtree保存字典
obj_id_list,path_proof = mtree_get(container_id,vec<key>)
//用mtree保存数组
obj_id_list,path_proof = mtree_get_range(container_id,start_index,end_index)
```

为了满足上述需求，cyfs用mtree做底层，实现3大容器:array(list,vector), set, map(dictionary,directory)
针对chunk实现的chunklist,与erc7585兼容，并提供了进一步的辅助设施。
    - 用最小内存完成计算（用于校验）
    - 允许缓存部分mtree信息，提高基于chunklist进行range请求/验证的性能
    - cyfs为chunlist的传输提供进一步的优化：在一个连接上可以连续传输chunk
        不带range,则默认目标是下载整个chunklist,此时会在http header里返回整个chunklist
        带range,则只返回range相关的chunklist和proof信息
        可以用 http://$zoneid/ndn/$chunklistid/2 的标准方法，来请求chunklist中的第3个chunk

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
客户端首先验证target_obj_id 与 target_obj_data是否匹配
然后验证:$container_id 是否与 $proof-data + $target_obj_id匹配




## 扩展Hash算法数据进行分块

该扩展是在chunk层面做还是fileobj层面做？这涉及到默认的chunk hash算法选择

对任何文件只计算一次Hash虽然简单，但也有很多弊端
- 客户端无法对Range请求进行验证。
- 验证粒度太大，如果一个文件有2G大，那么一旦下载验证失败，所有的2G数据都要丢弃重新下载
- 因为上述验证的问题，也不好支持多源下载。一旦出错无法确定是哪个源的问题
- 不分块对断点续传没影响，现在的框架设计要求Hasher必须有能力保存hash状态的


在fileobj层面做会提升fileobj的复杂度:content可以指向chunkid或chunklist
或则fileobj可以同时有多个等价的chunklist+content(这符合http协议的扩展思路，但会让完美实现非常复杂)
chunklist的构造一定是非均质的，可以参考git的命令模式

## 对历史版本进行追踪

结论：选择一个合适的chunklist,然后指望可以复用上一个版本的chunk。这样不用面对两个chunk之间计算diff的困难问题


- 有趣的事实是,git并没有包含任何diff算法，而是有一个有效的tree (dir) hash计算方法
- 如果使用相同的分块Hash算法，那么当新版本文件只是 Append时，自然就有Diff效果
- 如果追求最高的压缩比，那么选择哪个文件的那个一个版本做上一个版本都不是一个简单的事情
- 确定父版本后，一般使用滚动hash技术来寻找差异部分。随后可以得到一个补丁文件。此时可以用 (parent_chunk_id, patch_chunk_id) 来取代 chunk_id

## cyfs://对chunk的特殊支持
注意cyfs:// chunk的设计不适用于RTC领域，构造chunk的过程与RTC的低延迟的需求以及RTC数据可降级或丢失的假设是有更本矛盾的。

### 对断点续传的（错误回复）的一致性支持

### zone内鼓励push, 跨zone则应使用get


## 使用cyfs://来建立知识图谱


## 垃圾回收的问题
尽管cyfs:// 协议只约定了数据的交互方法，但根据我们的经验，什么数据应该永久保存，什么数据应该缓存，并不是一个简单的问题。尤其是当空间不足时，哪些数据是可删除的？

- 最简单的方法，就是不删除数据，这个对于企业级大型系统有利：增加存储空间的开销远比删除数据低
- 一个简单的规则：任何被保存的数据都应该有一个Path指向他 ，这和知识图谱的网络关系结合，意味着只有一种关系是用来确立所有权的
- 没有被Path指向的数据不会立刻被删除，而是应该根据最后访问时间（LRU策略）来进行Cache的淘汰。






 