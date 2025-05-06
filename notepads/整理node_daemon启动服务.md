1. kernel service也尽量通过node_config管理 
2. 为了能进入正常的node_daemon_main_loop，需要管理
    - 都要启动cyfs-gateway,但要注意在desktop环境下，与客户端共享（每台物理设备原理上只运行一个cyfs-gateway)
        因为不管什么node都要运行cyfs-gateway,因此选择cyfs-gateway构建依赖链
    - 如果本机是OOD，那么就一定会运行system_config (固定在3200端口)
    - 如果本机是OOD，且是首次运行，那么会启动scheduler做一次启动调度。后续scheduler在哪台OOD上运行取决于调度的结果