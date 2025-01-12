# NDN Lib的各个组件的简单说明

ndn_client.rs | cyfs_gateway.chunk_mgr_router  通过网络实现命名对象的上传和下载
------------------
chunk_mgr.rs 提供了更多的状态管理：包括基于路径的文件系统并以此建立ChunkGC机制，也对Chunk的下载/上传和相关缓存进行了管理。通过同一个chunk_mgr_id,本机的任何进程都可以访问同一个ChunkMgr。为了性能优化，ChunkMgr并没有用CS模式来实现跨进程的状态同步，这会让其实现的难度有所增大（有可能是一个提前优化）
------------------
local_store.rs 实现了命名数据和命名对象的本地存储，通过文件系统是sqlite的特性实现了跨进程安全同时访问
------------------
Chunk.rs/Object.rs 定义了命名数据和命名对象