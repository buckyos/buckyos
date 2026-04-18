# rdb backend迁移

目的：能自由的在不同规模的环境中，切换sqlite/postresSql作为底层

## 底层设施
通过 buckyos_api::get_rdb_instance(appid,owner_user,instance_id) 来得到connection string
随后通过sqlx，基于connection string进行后续db访问

## 数据库初始化流程

统一管理db schema, 应用需要配置instance_id=>db schema
系统在版本升级时，可以及时发现“同一个instance对应的db schema改变了”
db schema保存在 system_config中，app在安装时，有系统根据其AppDoc里声明的"rdb_instance需求“，分配实例并保存在InstallConfig中。

## 开发者的要求
1）在pkg-meta中包含rdb instance->schema信息。该信息要和代码里的实际实现相同
2）在unitTest期间，通过调试用的db connect string,来构造调试环境


## 常见问题处理
- 修正现有基于sqlite模型时错误的锁模型：应正确依赖 db锁/table锁/行锁