# 基于mtree的chunkid

## 传统的chunkid本质上是单chunk

- fileobj与单chunk绑定，实现简单
- 单chunk总是有一个合理大小的，太大的单chunk会带来很多问题。比如对10G的chunk,其验证周期很长，并且一旦验证失败，整个10G数据都废了，浪费很大
- 单chunk 限制了验证的粒度，从系统简洁的角度思考，从remote获得chunk只能是单来源的。

## mtree chunkid
chunktype: mt64 代表64MB叶子节点长度的mtree,256MB的文件会被切成4片
chunktype: mt1k 代表1k的叶子节点长度，这一般用于兼容ERC7585（2kb calldata,1K leafsize, 1024 / (2*16)=32层，最大文件大小为4TB)

给的range,可以确定leaf index-range
下载指定leaf时，可以不需要先下载完整的mtree,而是根据leaf index-range,获得与chunkid相关的mpath 数据，就可知道leaf的可信chunkid,并在下载的过程中进行验证
如果有需要，leaf chunkid可以不需要保存在chunk manger中，减少保存成本。
多源下载支持
    可以通过 leaf-chunk-id，向其他源发起请求
    可以通过 index@chunkid 的方法，向其他源发起请求


适合没有diff需求，但常有CDN需求的的常见大文件，比如照片，视频等

## mtree chunkid的缺点

因为是固定切片，所以对具有互动块相同的文件（通过在文件1的特定位置插入一块内容得到文件2的情况下）的支持不好。

## 支持不定长分片的mtree 



因为leaf是不定长的，所以只有有足够的leaf，才能知道range对应的leaf-range,越往后的range,需要知道的leaf就越多
从发起请求的角度来说，依旧可以通过leaf path验证，所请求的数据是属于mtree的

适合中大文件,并且有diff需求