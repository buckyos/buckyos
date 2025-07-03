# 发布内容（文件的规范)

URL设计，和kapi一样，使用通用目录结构
用于发布的named_mgr的id统一为"pub", "default"默认不能通过网络访问
(要不要区分？)

*/ndn/$ndn_path 作为默认的发布位置

也可以启用 pub.*/$ndn_path 作为默认发布位置

发布内容的核心是构建$ndn_path, ndn_path通常没有用户信息（是全局的），基本上是 /ndn/$分类/$app_path
有一些通用的分类位置

# 已发布index-db
/repo/meta_index_db  我的默认meta-index-db
/repo/meta_index_db/content 具体的chunk

# 已发布pkg (原则上包含所有meta-index-db中depkg)
/repo/pkgs/pkg_name/version pkg-meta文件（可不发布）
/repo/pkgs/pkg_name/version/chunk pkg的内容文件,这个设置通常是为了不触发GC，实际上通过

/$chunkid 直接访问

上述NDN 路径，也是另一种不依赖API的语义网络查询路径


## helper函数
def pub_object()
- 将一个object(json) 发布到NDN里


def pub_lcoal_file_as_fileobj
- 将一个本地文件pub到ndn里,注意其chunk也要有path(否则会被GC)


def pub_local_file_as_chunk
- 将一个本地文件pub到ndn里,注意其chunk也要有path(否则会被GC)
- 对一个已经pub的文件补充签名 （外面可验证）

def sign_obj()
- 对一个已经pub的object补充签名 （外面可验证）


------获取
获取并要求验证：
meta_file_obj_jwt = client.get_obj_by_url("http://zoneid/repo/meta_index_db")
meta_file_obj = decode(meta_file_obj，zoneid_pk)
client.download_chunk(meta_file_obj.content,local_file)

一步到位：
client.download_chunk_by_path("http://zoneid/repo/meta_index_db?inner_path=/content",local_file)
download_chunk_by_path:
    if url.have_inner_path:
        resp = http_client.open("http://zoneid/repo/meta_index_db?inner_path=/content")
        obj_jwt = resp.cyfs-obj-body
        obj = decode_obj_jwt(obj_jwt,zoneid.pk)
        chunk_id = obj.query_inner_path("conent")
        assert obj.path == repo/meta_index_db # 安全风险：明文传输可以合法的将返回值劫持到已经发布过的老版本上：修正方法，jwt的签名时间,同一个URL的发布时间是不会变小的。只需要检查发布时间就可以防止这个攻击）。这个记录是cyfs的本地安全数据库，机器级别
        assert obj.create_time > last_obj.create_time
        assert resp.obj-id == chunk_id
        基于chunk_id开始边下载边验证
    else
        （这个流程没有验证签名的流程？）
        resp = http_client.open("http://zoneid/repo/meta_index_db")
        chunk_id = resp.obj-id
        基于chunk_id开始边下载边验证 


## 默认权限设置
- 允许 Zone内Push Chunk
- 静止 Zone外Push Chunk
- 允许 Zone外Get Chunk
- 允许 Zone外Get File(通过区分default named_mgr和pub named_mgr来隔离)
- 一般 SetFile / PutObject 都是应用逻辑，无通用接口



## 发布文件夹(Zip模式)
###zone内发布
1. client将构造DirObject，构造的过程中，会反复打开File并计算其hash：
    减少总IO次数：在计算Hash的给过程中，可以边算边上传(push chunk)给OOD
    提升体验：通过一些数据接口设计，可以缓存DirObject构造的中间结构，支持中断后继续
    加快上传速度：push chunk接口由机会提前中断（当chunk在OOD上已经存在的时候）
    加快计算速度：当对已经完成构造的Dir再次构造DirObject时，能精确的只计算“改变”的部分？

2，所有的FileObject都构造好以后，开始构造DirObjectId,这里有一个根据Key排序，并构造树的过程（由map对象的内部实现）是一个TireMTree。此时元数据应打包成chunk
    DirObject的打包文件格式和其索引格式是不同的


3. 调用backup-service的backup接口，传递DirObjectId和已经打包文件的chunkid
4. backup-service检查该DirObjectId的存在状态，返回 [不存在/ChunkId存在一部分/已存在] 3种情况
4.1 client根据DirObjectId的存在状态，调用push chunk接口，直到bakcup-serivce确认chunkid已经存在
4.2 backup-service ‘解压并校验' chunk,完成后再ood的namedobject中，该dirobject存在
5. client调用backup-service的 Check接口，传入DirObjectId
6. backup-service 遍历DirObject 的Path->ObjId对，确认ObjId是否存在
    返回3种情况： 1.所有的都不存在 2.返回不存在的objid set 3.返回存在的`objid set`
    返回set时，可以选择将set打包成一个chunk（patch格式）并放到特定路径，client根据该路径可以得到patch
    返回不存在的ObjId Set (注意有翻页机制，允许client控制单次获取的大小）
7. client根据上述结果，构造`objset`（objset是一个文件），先调用push chunk,再调用backup-service的put_dir_item接口传入chunkid,
8. backup-service在put-dir-item的实现中，会是用标准的函数将objset包解压，并把所有其包含的obj保存到named mgr
9. 重复7，8两步，完成后client再次调用check接口，直到check返回所有item都存在
10. DirObject备份完成

优化：6,7,8 步基于websocket实现object sync协议，client和OOD之间建立 Object Sync Channel，可以更高效的完成objset的同步？

几个个核心的传输优化：
    1. 传输Map<ObjId>,打包成chunk
    2. 传输Set<ObjId>,打包成Chunk
    3. 传递Set<Obj> 打包成chunk

2. 跨zone发布
将DirObjId传递给ZoneB
ZoneB的backup-service检查该DirObjectId的存在状态，返回 [不存在/ChunkId存在一部分/已存在] 3种情况
    优化：ZoneB可以根据业务逻辑，返回其持有某几个旧版本的DirObjectId  
ZoneA根据ZoneB的返回，构造Map<ObjId>或 DiffMap<ObjId>


## 发布文件(Git模式)
Git模式里，每个TreeObject都不会太大（可以用单个Json保存)
Client构造TreeObject->Call Backup.PutTreeObject-> 返回该Tree中不存在的Item
    Client PutFileObject / PutTreeObject
最后检查Root TreeObject是否存在，如果存在则发布完成