use thiserror::Error;

#[derive(Error, Debug)]
pub enum DCFSClientError {
    #[error("Failed due to reason: {0}")]
    ReasonError(String),
    #[error("File not found: {0}")]
    FileNotFoundError(String),
    // 其他错误类型
}

type Result<T> = std::result::Result<T, DCFSClientError>;
pub struct DCFSClient {
    chunk_client : ChunkClient,
    meta_client: MetaClient,
    bucket_id : String,
}

// a smb:// server can base on this client to implement
// there is MUST be only ONE DCFSClient in a machine
// 要考虑对COW或快照的支持
// 客户端要有对写入事务的支持
// 客户端能进行良好的cache管理，减少和服务器通信的次数

/*

关键流程，读取文件 path,range
方法一：通过Meta->Chunk逻辑读取文件(GetObject)。这类文件通常是Object 文件，意味着几乎很少修改。
Step1.根据path查找文件的meta信息
找到合适的meta server
meta server根据文件路径查找信息
1.路径中是否有符号链接（指针），有则解析符号链接：解析结果是？
   根据路径查询多级"目录的MetaData",比如路径是 /apps/app/data/abc.txt ,那么 /, /apps,/apps/app,/apps/app/data 都是目录,都有可能有Meta信息
   快照通过上述机制实现？加入版本好，比如可以查询 /apps@v2333 的元数据信息。意思是apps目录v2333版本的元数据
   由于版本号是全MetaServer(卷)共享的，因此相当于可以指定版本号来查询元数据（未指定用HEAD）
   MetaData DB一定是在高速SSD上的，可以先用sqlite/mysql实现，后续可以是定制的btree数据结构
   可能还需要等待必要的锁信息
2.根据最终的解析结果得到文件的Meta信息
3.根据Meta信息中的ChunkList得到Range所在的Chunk

Step2.根据Chunk信息读取数据
在ChunkCache中查找Chunk，如果没有则从ChunkServer中读取
根据disk_map得到桶信息(Chunk Location)列表
根据Chunk Location（包含桶类型）可以进一步查询到ChunkServer并读取（分片数据）



一台主机上最多一个ChunkServer，但ChunkServer可以挂载多个Chunk桶
同类型的Chunk桶内的Chunk大小一致，容量一致
Chunk桶是一个自描述的文件夹，可以很容易的被ChunkServer挂载
一个已经平衡的系统，所有同类型的桶里的使用率是一致的
桶算法的选择应该与Network Block Device的逻辑尽力一致，以提高性能

Zero Cache读取小文件的延迟分析
- 查询Meta
- 加载CacheFile
- 
Zero Cache读取大文件的IO吞吐分析
有Cache后对上述两种情况的影响

PutObject实现：
Put NamedObject

Put Chunk
1. 通过disk_map找到足够数量的桶，每个桶代表1个副本
2. 根据桶的类型

方法二：通过CacheFile逻辑访问文件，这类文件通常是高频读写的文件，大量修改。比如数据库文件
Step1：根据path找到文件的meta信息
Step2：根据Meta信息找到CacheFile（由一个CacheServer提供）
一个FullPath只会对应一个CacheFile,只有一个Session可以带写打开（可以多个共享读）
Step3：CacheFile提供了ReadStream和WriteStream，可以进行读写操作
CacheFile后面会有自动的副本写入操作，保证Write成功后的基本可靠性 （因此存在写放大）
** 使用ReadStream/WriteStream改变文件内容并不会立刻出发FileMeta的更新，因此
Step4：当CacheServer故障时，客户端有机会在另一台副本CachServer上打开相同的CacheFile
Step5：CacheFile关闭后，系统会自动将CacheFile的数据写入ChunkServer，并创建Meta信息。
CacheServer上已经写入Chunk的CacheFile会在到达一定容量后删除（LRU机制），热门文件几乎会一直被保留在CacheServer上

当CacheServer的CacheFile增加速度持续超过写入ChunkServer后，系统会给出警告，并尽量将空间保留给CacheFile
CacheFile通常都是写入在SSD上，因此当容量不够后转移到HDDD上，CacheFile的写入速度就会下降。

没有写入ChunkServer的CacheFile没有最新的元数据信息，因此无法被快照或通过版本访问。当有必要时，可以强制要求CacheFile->ChunkFile，此时会导致一些类似创建快照的工作需要等待文件关闭才能完成。



事务的概念
在DFS中引入事务的首要目的是减少写入次数，打包请求提高IO效率
由于标准的FS API是不支持事务的，因此我们需要在FUSE里透明的引入事务
透明引入事务的主要风险是应用层看来写入成功的数据，实际上并没有写入成功。但考虑到蓝色状态（可靠备份）的达成绝对不会是实时的，因此在我们的集群规模下应用要考虑新写入数据的丢失风险。


文件系统可以在目录层对默认使用方式1还是方式2打开文件，是否支持透明事务等进行配置

文件系统的多级缓存概念


*/
impl DCFSClient {
    pub fn new() -> DCFSClient {
        DCFSClient {
            chunk_client : ChunkClient::new(),
            meta_client : MetaClient::new(),
            
        }
    }

    pub async fn mkdir(&self, path : String) -> Result<()> {
        // 1) use meta_client to create meta info
    }

    //notice: directory have lots of files cloud be slow
    pub async fn list(&self, path : String) -> Result<Vec<String>> {
        // 1) use meta_client to get meta info
    }

    pub async fn remove_dir(&self, path : String) -> Result<()> {
        // 1) use meta_client to delete meta info
    }

    pub async fn remove_file(&self, path : String) -> Result<()> {
        // 1) use meta_client to delete meta info
    }

    pub async fn move_file(&self, src_path : String, dst_path : String) -> Result<()> {
        // 1) use meta_client to move meta info
    }

    pub async fn copy(&self, src_path : String, dst_path : String) -> Result<()> {
        // 1) use meta_client to copy meta info
    }


    pub async fn link(&self, src_path : String, dst_path : String) -> Result<()> {
        // 1) use meta_client to link meta info
    }
    

    pub async fn open(&self, path : String,open_flags:u32) -> Result<()> {
        // 1) find opened file in the opened file list, if found return
        // 2) select a meta_server (some times , it is close to the device and file)
        // 2) use meta_client to create meta 
  
    }
    
    pub fn seek(&self, file_id : u128, offset:u64) -> Result<()> {
        // 1) find opened file in the opened file list, if not found return error
        // 2) change position of the file
    }

    //写入必然是独占的，默认写入会在主写入完成后返回 基本成功，然后等待local_cache写入足够的chunk 
    //       应用可以配置成高可靠cache写和高可靠写
    //       高可靠cache写：会连接至少一个cache server，写入成功后返回
    //       高可靠写：写完cache后立刻计算hash并写入chunk server，写入成功后返回
    pub async fn write(&self, file_id : u128, data:Vec<u8>) -> Result<()> {
        // 1) find opened file in the opened file list by file_id, if not found return error
        // 2) write data to local cache
        // 3) update meta info at local if needed
        // 4) notify local cache start working 
    }

    pub async fn read(&self, file_id : u128, offset:u64, size:u64) -> Result<Vec<u8>> {
        // 1) find opened file in the opened file list, if not found return error
        // 2) read data from local cache 
        // 3) if not found in local cache, try read data from other Cache Server
        // 4) if not found in other Cache Server, try read data from chunk_server
    }

    pub async fn flush(&self, file_id : u128) -> Result<()> {
        // 1) find opened file in the opened file list, if not found return error
        // 2) flush data to local cache
        // 3) flush data to all Cache Server (at least 1)
        // 4) if needed, flush data to chunk_server
    }

    //pub async fn stat(&self, path:String) -> Result<()> {
    //    // 1) use meta_client to get meta info
    //}

    pub async fn close(&self, file_id : u128) -> Result<()> {
        // 1) find opened file in the opened file list, if not found return error
        // 2) use meta_client to close the file
    }
}

