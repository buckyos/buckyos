# AICC Beta 2.2

本 crate 包含 AICC Beta 2.2 的模块化实现。旧 AICC 实现不属于本目录的依赖或参考实现。

## 模块边界

所有顶层业务模块均由 `lib.rs` 以 `pub(crate)` 暴露。WP-00 不预定义未经对应工作包确认的 trait、DTO、fixture 或 fake；这些内容由各模块 owner 在契约确定后放入自己的模块。公共 kRPC schema 仍由 `buckyos-api::aicc_client` 拥有，本 crate 不重复定义外部 RPC 类型。

## 编译顺序

模块按以下层级实施，同一层可以并行：

1. `error`
2. `matching`、`catalog`、`model`、`protocol`、`provider`、`settings`、`resource`、`observability`
3. `storage`、`routing`、`execution`
4. `call`、`runtime`、`api`
5. `service`、`lib.rs`、`main.rs`

依赖方向以 `doc/aicc/internal_module_architecture.md` 为准。增加反向依赖或跨 owner 引入公共 contract 时必须由对应 owner 和集成人共同评审。

## 最小验证

在 `src/` 下运行：

```bash
cargo test -p aicc
cargo check -p aicc --all-targets
```

`main.rs` 创建 Tokio runtime，并通过 crate 导出的 `run_service()` 启动 `/kapi/aicc`。服务使用 `KernelService` 身份登录 BuckyOS runtime，在 4040 端口注册 kRPC handler，并在退出时停止 metadata、Provider 和 runtime 后台任务。

模块单元测试不连接真实 Provider。rootfs、settings、RBAC 和调用方联动由 WP-17 完成后再进入 T1/T1.5 集成验收。

旧实现残留扫描：

```bash
rg "AIComputeCenter|model_session|metadata_updater|openai_protocol|claude_protocol" frame/aicc
```

扫描结果只能来自本 README 的命令文本。

## 上传对象作为输入

Playground 本地文件由 WebSDK NDM 导入，AICC 只接收 ready FileObject 的 `named_object` 引用。未由 AICC 产物登记的对象必须使用原请求 token 经 NDM `get_object` 验证读取权限；tenant artifact 继续使用已有 tenant 归属规则。不能用 AICC 服务凭据替代上传者的授权。

canonical FileObject 无 MIME 时，资源层读取最多 4096 字节做内容识别，支持常见 PNG/JPEG/GIF/WebP、WAV/FLAC/Ogg/MP3、MP4/WebM、PDF、ZIP，以及 UTF-8 `.txt`。元数据不参与修改 ObjId；未知 MIME 仍按 API resource policy 拒绝，不能只相信扩展名声明媒体类型。

请求读之前使用自有 owner 的 `Recursive` pin + 24 小时 TTL 保留整个对象依赖，运行时每 5 分钟续租。最后一个同步 guard 释放时清理该 owner；native provider task 保留 guard，持久化恢复描述，在重启后 poll 时重建，完成或确认取消后释放。provider 已接受但返回不确定的提交保留有界 TTL，避免提前清理；此路径不会永久续租。其他任务/应用的 pin 不受影响。
