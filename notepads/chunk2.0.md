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


## 最终目标：基础设施可以支持新的 DFS 和 GraphDB

虽然可以用soft link用文件系统实现知识图谱，但GraphDB支持的高级查询（这些查询还是比较学术的，并不清楚在实际产品中是否有价值）


## 基于ObjectMap的 DirObject

支持两种模式：
1. zip模式，使用 full_path:fileobjid 的模式
2. tree模式，使用 path:fileobjid , path:treeid 的模式

tree模式需要构造跟多的object map对象，但可以实现子目录的复用，对于目录的移动非常的友好。如果要组织的是日常使用的文件系统,tree模式肯定要好过zip模式

zip模式比较适合发布一个完整的package(包括基于该package构建一个有少量修改的新版本)。比如传统的bittorrent发布


### 站在备份的角度

- 扫描磁盘，构建fileobject,构建dirobject(深度扫描)
- fileobject可以边构建边上传
