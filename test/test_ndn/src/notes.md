* 接口参数规范不统一，有的传递引用，有的传递值，特别是ChunkId/ObjId/String等小数据类型，比较随意
* download_chunk_to_local, Path参数用&Path类型可能比&PathBuf好
    no_verify: bool?