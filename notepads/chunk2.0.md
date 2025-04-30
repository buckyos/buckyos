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



## 最终目标：基础设施可以支持新的 DFS 和 GraphDB

虽然可以用soft link用文件系统实现知识图谱，但GraphDB支持的高级查询（这些查询还是比较学术的，并不清楚在实际产品中是否有价值）


