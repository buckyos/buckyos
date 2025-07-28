* 接口参数规范不统一，有的传递引用，有的传递值，特别是ChunkId/ObjId/String等小数据类型，比较随意
* download_chunk_to_local, Path参数用&Path类型可能比&PathBuf好
    no_verify: bool?
    有概率下会的内容出错
* pull_chunk_by_url 可能因为ChunkHasher::restore_from_state返回空导致没有校验chunk的正确性?