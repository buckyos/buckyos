# 通过简单可靠的DFS保存集群的所有状态

## 基本设计
1. DFS通常都有一个基于对象存储的底层，支持两种寻址
a. 根据文件(chunk)的内容hash，通过disk_map算法找到location
b. 根据文件的path的hash,通过disk_map算法找到location,注意此时得到的可读写空间是有大小限制的（一个chunk大小）
2. 基于对象存储，通常需要本地缓存，即写入操作是
```
step1 写入本地缓存
step2 写入完成，等待提交（手工/自动）
step3 提交时计算chunkhash, 保存chunk
      同时创建/更新元信息

      元数据更新后，读取方就有机会读到文件
step4 所有的chunk都写入成功（并有足够的健康度） 提交完成，删除本地缓存
      commit成功
       
```
该流程中，运行在同一个Node上的服务，有机会通过文件系统来实现状态的共享。
当运行在强兼容模式时，所有的fflush/fclose等操作，都会等到commit成功才返回
默认只要写入了本地缓存就可以返回，可以通过高级api查询commit的进度

3. 多个进程读取同一个文件的强一致问题
在DFS的场景中，多个进程读取在DFS上的一个文件变得非常的普遍

但基于LOCAL CACHE的写入策略，会让file在commit之前（尤其是弱兼容模式），无法被其它进程读到。这对数据库文件的共享影响很大，这意味着一个数据库插入操作成功后，需要等待一会才能被读取到

解决思路：从Local Cache变成DFS Cache, 这样使用Cache协议可以一致性的，快速的读取到刚刚的写入。


4. 伪代码

```rust
function fopen(path) {
    pf = cache.find_opend(path)
    if pf {
        return pf
    }

    meta_info = meta.get(path)
    if meta_info {
        pf = create_pf_by_meta(meta)
        cache.set(path,pf)
        return pf 
    }

    //create new file
}

function fread(pf,length) {
    content,miss_list = cache.get(pf,pf.pos,length)
    if miss_list.length == 0 {
        return content
    }
    for miss_range in miss_list {
        chunk_id = pf.meta_info.get_by_range(miss_range)
        content = chunk_server.load(chunk_id)
        cache.set(pf,chunk_range,miss_range) // range data flags is read-only
    }
}

function fwrite(pf,buffer) {
    cache_ranges = cache.get_range(pf,pf.pos,buffer.length)
    for cache in cache_ranges {
        cache.flag = UPDATE
        cache.data = buffer[pos,length]
    }

}

function fappend(pf,buffer) {

}

function fflush(pf) {

}

function fclose(pf) {

}


funciton fcommit(pf) {

}


```

## 思考：基于DFS能彻底解决集群的数据可靠性问题么？
发生设备故障时，会丢失写入缓存中的数据
对数据的可靠性有强需求的应用，应手工等待commit
考虑sqlite的运行情况：（用日志跟踪一下）


## 思考，基于DFS能有足够的性能么》

--------------------------------------------------
## 思考, 是否需要将DRDB作为DFS的平级基础设施

## 思考，是否需要引入有一定状态持久能力的Pub/Sub 基础设施