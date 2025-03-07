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
