# SDK/Tool 与 Module 构建迁移：历史决策摘要

> 历史参考。合并 SDK/Tool 双分发 TODO 与 module type/format 设计中的一次性迁移内容。此处记录选择原因，不发布第二套 CLI、输入 schema 或构建验收清单。

## 保留的选择理由

- 将原 BuckyOS tools/buckyos-tool 迁至 WebSDK CLI，避免项目 npm Tool 与系统 Tool 各维护一套命令；npm 使用 Node bundle，系统入口使用随安装包提供的 Deno runtime。
- npm 本地 bin 与系统 PATH 是不同分发入口；元数据标签和 host policy 不等于服务端授权，Node host 也不能被描述为拥有 Deno 等价的进程级沙箱。
- 系统镜像消费一次构造并核验的 SDK/Tool 产物、manifest、SBOM 与 runtime；开发者项目用自己的包管理器 lockfile。双 host 的一致性仍需 conformance 证据。
- 拆开 builder 与 formatter，是为避免 `web-pikg` 组合类型、固定 App 名单和项目 build.mjs 自行打包；已删除的迁移步骤不再作为当前构建要求。

## 旧稿中的候选项

capability handshake、identity rotation、版本化 DV artifact、`pikg env-*` / `pikg test`、自动化发布和 typed client 完整化曾作为候选。旧稿未勾选与候选命名都不是能力状态；新增功能须按实际协议另立任务，不能从这里直接照抄参数或错误码。

`pikg doctor` 已进入当前命令表，但不实现旧设想的 Docker 健康检查、Zone 连接或兼容性握手。TypeScript 和 Rust reader 也没有按旧设想合成一份共享源码。

## 当前替代资料

- [SDK/Tool distribution](https://github.com/buckyos/buckyos-websdk/blob/main/doc/distribution.md)：双分发与产物规则。
- [PIKG CLI](https://github.com/buckyos/buckyos-websdk/blob/main/doc/modules/pikg.md)：本地六个命令。
- [Module 构建规范](../../CI/module_build_type_and_format.md)：当前 type/format/pikg 集成。

## 原始稿追溯

- [notepads/buckyos-sdk-tool-distribution-todo.md](https://github.com/buckyos/buckyos/blob/35f00c252b6ef1cefb24cbe1fb84537936bcba09/notepads/buckyos-sdk-tool-distribution-todo.md)
- [doc/CI/module_build_type_and_format.md](https://github.com/buckyos/buckyos/blob/3ee48781eec1ed5a8941b756a1f8d298e14d0803/doc/CI/module_build_type_and_format.md)
