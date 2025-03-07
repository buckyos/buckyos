repo-server持有的几个meta-index-db

source-mgr
    source-meta-index1 (read-only)
    source-meta-index2 (read-ony)

如果打开了开发者模式，则有：
    local-meta (read-only)
    local-wait-meta

在TaskMg中?
    pub-task-db


问题:local-meta是否要和source-meta合并？


## pub_pkg(pkg_list) 将pkg_list发布到zone内，是发布操作的第一步
 
Zone内在调用接口前已经将chunk写入repo-servere可访问的named_mgr了

检查完所有pkg_list都ready后（尤其是其依赖都ready后），通过SQL事务插入一批pkg到 local-wait-meta

## pub_index_db,
将local-wait-meta发布（发布后只读）
发布后会计算index-db的chunk_id并构造fileobj,更新R路径的信息

## handle_pub_pkg_to_source(pkg_list) 发布pkg到源
因为是处理zone外来的Pkg，所以流程上要稍微复杂一点
1. 验证身份是合法的
2. 在pub_pkg_db库里创建发布任务，写入pkg_list，初始状态为已经收到
2. 检查pkg_list的各种deps已经存在了,失败在发布任务中写入错误信息
3. 尝试下载chunkid到本地，失败在发布任务中写入错误信息，下载成功的chunk会关联到正确的path,防止被删除
4. 所有的chunk都准备好了，本次发布成功（业务逻辑也可以加入审核流程，手工将发布任务的状态设置为成功）

## handle_merge_wait_pub_to_source_pkg
合并 `未合并但准备好的` 发布任务里包含的pkg_list到local-wait-meta
注意merge完后，也要调用pub_index_db发布
 

## sync_from_remote_source(source_url)
 将source-meta-index更新到指定版本
1.先下载并验证远程版本到临时db
2.根据业务逻辑检查pkg-meta,下载必要的chunk
3.下载并验证chunk
4.全部成功后，将临时db覆盖当前的source-meta-index


## 一些零散的查询接口（安需要添加）

 
 

