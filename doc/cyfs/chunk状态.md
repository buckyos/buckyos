## Chunk状态（服务器视角）
0.client.OpenChunkWriter
1.如果Chunk不存在责初始是New,并知道了chunk的totalsize
2.如果未写入任何数据，client再次openchunkwriter,则返回的是New(已写入0字节)
2.client.Writer.write,开始写入数据
3.服务器会同步写入数据
4.此时再次client.openchunkwriter,chunk的状态变成(in_complete,写入了部分字节)
5.client.complete_chunk_writer,chunk的状态变成complete

## 断点续写的实现

1. 先用QueryState查询Chunk的状态，和已经写入的字节数
2. 使用正确的openwriter调用，传入正确的offset

## 竞争问题
1. ndn_mgr本身是否有对chunk的写入进行锁管理？即同时只能打开一个writer?
2. ndn_mgr是否对chunk的读写锁进行管理？无法打开不处于complete状态的chunkreader? 
    是的，做了这种限制
