# Module 编译类型与 PIKG 产物集成

> 当前权威：BuckyOS 的 `type/format/pikg` 配置与 PIKG rootfs 落位。旧迁移方案、版本切换步骤及计划验收矩阵已合并到 [历史记录](../pikg/history/tool-and-build-migration.md)。

本文依据当前 [bucky_project.yaml](../../src/bucky_project.yaml)、[buckyos-build.py](../../src/buckyos-build.py)，并核对已有 devkit 0.7.33 的 `project.py/module_build.py/formatters.py` 实现。devkit 来自 `main`，执行前应核对实际安装版本；本页不把历史的待实施清单当作当前工作。

## 1. 编译与格式是两个维度

| 配置 | 职责 |
| --- | --- |
| `type` | builder：none/web/rust，缺省 none |
| `format` | formatter：files/pikg，缺省 files |
| `pikg.meta_dir` | 相对 builder `BuildResult.work_dir` 的元数据目录，缺省 dapp_meta |
| `apps.<app>.modules.<module>` | 对应 app staging rootfs 中的安装目标 |

`type: web-pikg` 是已移除的旧组合，不使用兼容别名。builder 先准备源码构建输出；formatter 再消费 `BuildResult.work_dir`。PIKG formatter 不根据 builder 类型分支、不要求名为 dist 的固定目录；它读取 manifest 中声明的 source。

## 2. 当前系统配置示例

```yaml
modules:
  text_editor:
    type: web
    format: pikg
    name: text_editor
    src_dir: apps/text_editor
  jarvis_agent:
    format: pikg
    name: jarvis_agent
    src_dir: apps/jarvis_agent
apps:
  buckyos:
    rootfs: rootfs/
    modules:
      text_editor: data/cache/text-editor.buckyos.bns.did-0.1.0.pikg
      jarvis_agent: data/cache/jarvis.buckyos.bns.did-0.7.0.pikg
```

版本和完整 module map 以实际 YAML 为准。text_editor 和 buckyos_systest 是 web/pikg；Jarvis 直接以静态 AgentRootFS 和元数据进入 none/pikg。module 不通过自有 build.mjs 再打一次包。

## 3. 输入唯一来源

`app.json` 定义应用语义，`pikg.json` 定义 `output_dir/pikg_file/sub_pkgs`；这些字段不在 `bucky_project.yaml` 再声明一遍。元数据 schema 只在 [PIKG CLI](https://github.com/buckyos/buckyos-websdk/blob/main/doc/modules/pikg.md) 维护，包内结构只在 [格式规范](../pikg/format.md) 维护。

每个 pikg module 必须有同 key 的 app module 安装项。目标是 staging rootfs 内安全的相对 `.pikg` 文件路径，basename 必须与 `pikg.json.pikg_file` 一致。source 必须在 builder 完成后可用；formatter 没有项目专用 prepare/copy/clean hook。

## 4. Formatter 流程

1. 从 BuildResult work directory 解析 metadata，校验 source 与安装目标。
2. 以 `npx --yes buckyos@latest` 执行 `pikg build <meta_dir>`，为 manifest 中的 path source 显式传入全局 `--allow-read`。
3. 解析 JSON envelope，要求 `ready_for_pack=true`，返回 dist_dir 与 manifest 解析结果相同。
4. 对该 dist 执行 `pikg pack`，要求 `validation=passed`，输出必须是 dist 内符合命名的非 symlink 普通文件。
5. 执行 `pikg info`，交叉核对 build/pack/info 的 AppDoc Object ID、App DID/version、路径、文件大小及整包 digest。
6. 核对真实文件大小和 SHA-256，再事务性复制到所有声明的 staging 目标；失败恢复旧 staging 文件。

上述 npx 命令可能解析 npm 发布包，不等于离线 CLI 本身的网络需求。系统 SDK/Tool 分发包的准备是 `buckyos-build.py` 的独立阶段，不要把二者混为一套固定工具选择。

## 5. 构建入口与生命周期

从 `buckyos/src/` 使用 `uv run buckyos-build.py`，或用 `-s <module>` 选择模块。该入口先准备 SDK/Tool 分发包，再调用 devkit；直接调用 devkit 会绕过前一阶段。`--skip-web` 按 builder type 跳过 web module，因此也会跳过 web/pikg。

AppDoc 只在 PIKG 内作为自描述内容，不额外复制一份到 staging 的 local/did_docs 来补全包。普通 App 的 rootfs 包交给 Control Panel 预装，Agent 模板由 agent_templates 登记；消费职责见 [App 生命周期](../app_service/BuckyOS%20App基础设施职责边界.md)。

## 6. 变更时的验证

- 先核对 metadata、builder 输出、CLI 返回值与 app module 目标的绑定。
- 对 PIKG 跑 info；错误来源或损坏包必须在替换 staging 前失败。
- 校验 `--skip-web` 和模块选择，不增加固定 PIKG module 名单或手工 copy 旁路。
- 核对 `.gitignore`，build 生成物不应污染受版本控制源码。
- 安装、重启和 fresh-init 是后续运行阶段，不为验证文档或构建配置自动执行。

本页的整理验证为源码/路径/差异核对，没有重新构建系统或部署。旧版 devkit 的迁移顺序、旧 build.mjs 和一次性清理任务只供历史追溯。
