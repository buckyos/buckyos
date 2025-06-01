# chunk2.0的一些结论

## Named Object的hash算法
我们计算objid的hash算法是不修改的，为sha256 （在代码里应显示指明该部分的不可扩展）


## Chunk的Hash算法是可扩展的

处理chunk的时候是否正确的获得了haser

## 默认的hash算法切换到mixhash：支持数据进行分块,并且支持直接得到长度

mix1k (纯智能合约使用），mix64k,mix(1m),mix16（TB以上文件使用） ,后缀说明了每片数据的大小


该扩展是在chunk层面做还是fileobj层面做？这涉及到默认的chunk hash算法选择

对任何文件只计算一次Hash虽然简单，但也有很多弊端
- 客户端无法对Range请求进行验证。
- 验证粒度太大，如果一个文件有2G大，那么一旦下载验证失败，所有的2G数据都要丢弃重新下载
- 因为上述验证的问题，也不好支持多源下载。一旦出错无法确定是哪个源的问题
- 不分块对断点续传没影响，现在的框架设计要求Hasher必须有能力保存hash状态的


在fileobj层面做会提升fileobj的复杂度:content可以指向chunkid或chunklist
或则fileobj可以同时有多个等价的chunklist+content(这符合http协议的扩展思路，但会让完美实现非常复杂)
chunklist的构造一定是非均质的，可以参考git的命令模式

## 定义chunklist

如果chunklist不大，使用mtree有意义么？



在默认情况下(mix),chunk大小为4MB，这对大部分文件来说，chunklist的分片数量都不会超过10K，这意味着可以将chunklist视作小对象来处理

客户端发起get chunklist请求时，明确的指定Range,会在header里返回Range里涉及到的所有chunklist



## 通过chunklist_id[index]访问chunk

http://$zoneid/chunklist_id/index

请求和返回值结构与访问标准的objec-container基本一致

## 特例：请求chunklist
当URL指向一个chunk_list_id时，在没有额外参数的情况下，可以根据请求中的Range参数（如没有是0 -- chunklist.length)返回chunk_list表到的stream

返回
```
cyfs-object:chunklist_data
<chunklist stream>
```

如果参数中有 ?resp_format=object  则在body中返回chunklist的数据
## 对历史版本进行追踪

结论：选择一个合适的chunklist,然后指望可以复用上一个版本的chunk。这样不用面对两个chunk之间计算diff的困难问题


- 有趣的事实是,git并没有包含任何diff算法，而是有一个有效的tree (dir) hash计算方法
- 如果使用相同的分块Hash算法，那么当新版本文件只是 Append时，自然就有Diff效果
- 如果追求最高的压缩比，那么选择哪个文件的那个一个版本做上一个版本都不是一个简单的事情
- 确定父版本后，一般使用滚动hash技术来寻找差异部分。随后可以得到一个补丁文件。此时可以用 (parent_chunk_id, patch_chunk_id) 来取代 chunk_id



## chunklist和chunk的一些伪代码

### 本地读取

常用接口，用chunkid当文件名来读取

```rust
reader = ndn_mgr.open_chunk_reader(chunk_id)
reader.read(offset, size)
```

常用接口，可以直接用chunklist_id当文件名来读取，内部根据offset先得到实际的chunkid在进行读取
```rust
reader = ndn_mgr.open_chunklist_reader(chunklist_id)
reader.read(offset, size)
```


```rust
reader = ndn_mgr.open_chunk_reader_by_chunklist(chunklist_id,index)
reader.read(offset, size)
```


### 本地写入

写入，这个语义上有锁定的含义，完全写完才能open chunk reader成功
```rust
writer = ndn_mgr.open_chunk_writer(chunk_id)
writer.write(offset, data)
```

写入一个chunklist通常意味着写入多个chunk，这个过程是可以并行的，并且写入成功的chunk立刻就可以被chunklist reader使用
```rust
for chunkid in chunklist {
    writer = ndn_mgr.open_chunk_writer(chunkid)
    writer.write(offset, data)
}
```
有一种特殊情况，就是写入的chunklist[index]必须通过chunklist_id + index才能访问。这通常是大量的小chunk组成的chunklist,chunklist通常通过一个文件来保存
```rust
// 这种只适合于不会保存chunklist里的chunkid的场景
writer = ndn_mgr.open_chunklist_writer(chunklist_id,index)
writer.write(offset, data)
```


### 具体的例子，使用chunklist来实现fileobject

将一个大文件写入ndn_mgr的过程，不再依赖hasher的状态保存与恢复，就可以实现断点续穿

```rust
file_reader = open_file(file_path)
chunk_list = []
loop {
    chunk_buffer = file_reader.read(CHUNK_SIZE)
    chunk_id = chunk_hasher.cacl_buffer(chunk_buffer)
    chunk_writer = ndn_mgr.open_chunk_writer(chunk_id)
    chunk_writer.write(chunk_buffer)
    chunk_list.append(chunk_id)
}
chunk_list_id = chunk_list.cacl_id();
ndn_mgr.put_chunklist(chunk_list_id, chunk_list);
file_obj = json!({ 
    "file_size": file_reader.size(),
    "content": chunk_list_id,
})
ndn_mgr.put_obj(file_obj_id, file_obj);
```

### ndn_client的伪代码

- 带验证的get chunk


- 带验证的get chunklist
```rust
reader,cyfs_resp_header = ndn_client.open_named_data_by_url(chunk_list_url,range)
copy_chunk_list(reader,cyfs_resp_header)

```


- put chunk

- put chunklist


### 高级: Link两个chunklist
对同一个数据，可以用对个不同切分方法的chunklist来描述。
这几个chunklist是等价的，系统只保存一份数据即可
可用于基于chunk切分的版本diff计算，较少文件新版本的实际存储空间 


## 基于ObjectMap的 DirObject

支持两种模式：
1. zip模式，使用 full_path:fileobjid 的模式
2. tree模式，使用 path:fileobjid , path:treeid 的模式

tree模式需要构造跟多的object map对象，但可以实现子目录的复用，对于目录的移动非常的友好。如果要组织的是日常使用的文件系统,tree模式肯定要好过zip模式

zip模式比较适合发布一个完整的package(包括基于该package构建一个有少量修改的新版本)。比如传统的bittorrent发布


### 站在备份的角度 （源目录是不变的）理解DirObject

1. 扫描磁盘，构建fileobject,逐层构建dirobject(深度扫描)
2. 对于大文件，fileobject可以使用diff模式(特殊的chunklist)模式构建，并边构建边上传： 计算得到2个hash
3. 根据备份源的写入速度，有两种备份模式：
3.1 写入速度不够：则进入快速扫描模式：先全力完成checkpoint的dir_obj的构建，再持续完成任务
3.2 写入速度足够：进入边扫描边备份模式（这也是系统里难度较大的模式）
4. 增量备份的特性基本利用类Git模式，但因为我们再fileobject种记录了chunklist,所以有机会只在新的checkpoint中保存改变的chunk
   该方案要优于git的GFS方案
   依旧允许针对特殊文件使用diff算法，来保存diff日志
5. 备份目的地需要使用基于Named Object的GC方法，才能正确的实现对某个特定备份的删除

#### buckyos system与备份系统集成的机制
1. 能通过操作系统(filesystem)的COW机制，尽快创建源文件夹的快照（同时不增加实际的磁盘占用）
2. buckyos应在备份操作发起前明确的通知各个应用，立刻完成（或放弃）手头的写入（防止半状态）‘提高有状态服务的状态原子性/事务性是一个系统性的课题’并等待系统快照创建完成
3. bucyos不会等待应用完成写入，而是固定的等待10秒后（和系统服务完成暂停后）开始创建快照，并在创建快照完成后，推进系统的当前快照版本

#### 主流操作系统创建文件夹快照的方法
- Linux（需要使用btfs)
- Windows：需要使用NTFS的全盘快照功能，这会创建一个新的逻辑磁盘
- OSX:


### 站在DCFS的基础设施的角度理解DirObject和FileObject

AI 时代的核心是为文件系统找到新需求（或则buckyos本质上需要什么样的文件系统），除了对基础能力提出更多或更严格的要求外，也可以要求设计更多的接口原语给应用开发者。使用这些原语可以让应用开发者更易于开发出状态管理难度更低，性能更好，更面向内容网络的应用

根本上是在思考，我们的数据结构在常见的FS操作下，所需要的理论IO次数（写放大），和读取时的Cache命中问题
这里可以完全站在块设备的角度来思考。优于SSD的普及，已经不需要思考操作时块(block)的连续性了


#### fd = create_file(base_dir,filename)
根据路径得到锁，如果没有则创建并锁定（内存操作）
创建一个内存（缓冲）区域等待写入
基于fd返回该内存区域

#### write(fd)
写入fd绑定的缓冲区，根据内存大小可能会出发磁盘写

### flush(fd)
将缓冲区的内容全部刷写进磁盘

### read(fd)
针对同一个fd,可以边写边读，其机制基本是系统的共享内存机制

### close(fd)


### commit(dir)
如果操作了目录下的一组文件，则此操作会原子性的对dir进行修改
此时必然会产生一个新版本的dirobject，构建新版本的dirobject一般会很大程度上复用旧版本的dirobjecty已经存在的元数据

### 站在DataGraph的角度来理解DirObject和FileObject

