# PIKG test samples

这里保存的是 `npx buckyos pikg` 使用的标准构建工程，不提交 `dapp_dist/` 或
`.pikg` 构建结果：

- `static-web/`：平台无关的静态网页包（`pkg_list.web`）。
- `script-host/`：平台无关的 Python 服务包（`pkg_list.script`）。
- `agent/`：OpenDAN Agent 包（`pkg_list.agent`）。
- `docker/`：Docker 服务包；仓库中的 `dapp_meta` 是 linux/amd64 示例，现场构建脚本会按宿主机架构切换为 `amd64_docker_image` 或 `aarch64_docker_image`。

每个目录都包含 `dapp_meta/app.json`、`dapp_meta/pikg.json` 和对应的构建输入。
普通 path 类型样例可以直接构建：

```bash
npx buckyos pikg build static-web/dapp_meta
npx buckyos pikg pack static-web/dapp_dist
```

统一现场构造并离线校验全部样例：

```bash
cd ..
pnpm run generate:pikg-samples
```

生成结果默认写入系统临时目录的 `buckyos-pikg-samples/`，也可通过
`BUCKYOS_PIKG_OUTPUT_DIR` 指定目录。Docker 样例会先从 `docker/image/Dockerfile`
构建本地镜像，再交给 `npx buckyos pikg build/pack/info`；该流程不依赖运行中的
BuckyOS、Control Panel、身份或网络服务。
