/*
Beta3
需要提供3个视图
1）每台设备的raw fs视图
2）NamedStore视图 :涉及到是否要在这一层做soft raid,NamedStoreMgr本身是否要实现RS Code管理,s
3) cyfs(dfs) 视图

管理视图
- 备份恢复入口
- 管理NamedStoreMgr

扩容

*/