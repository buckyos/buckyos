# cyfs://$zone_id/cache/

当前设计里，`cache/` 主要用于服务缓存，而不是旧的 `fs://cache` 应用目录模型。

- `cyfs://$zone_id/cache/$service_name/`：服务缓存数据，对应本地 `data/cache/$service_name`
- 这类数据主要用于提升性能，系统会尽量保留，但在重新安装服务或空间紧张时可以被清理

应用自己的临时数据请使用 `/tmp/buckyos/$appid`，不要再把应用缓存写到旧的 `cache/$username/app_name` 目录模型中。
