# `publish_app_to_repo` 的 `local_dir` 格式说明

本文说明当前 `publish_app_to_repo` 对不同类型应用的 `local_dir` 输入要求。

## 通用规则

- `local_dir` 必须存在，且必须是目录。
- `app_type` 必须和 `app_doc_template` 中的应用类型一致。
- 当前不支持 `Service` 类型。
- 打包目录时，会递归包含普通文件，并忽略隐藏文件和隐藏目录。

## 1. Web

### 要求

- `local_dir` 本身就是静态网站根目录。
- `app_doc_template.pkg_list.web` 必须存在。

### 示例

```text
my-web-app/
├── index.html
├── assets/
│   ├── app.js
│   └── app.css
└── favicon.png
```

### 发布结果

- `local_dir` 整体会被打包成 `web` 子包。
- 顶层 `AppDoc` 是纯 meta，`content` 为空。

## 2. Agent

### 要求

- `local_dir` 本身就是 Agent 主目录。
- `app_doc_template.pkg_list.agent` 必须存在。
- 当前**不支持** `agent_skills` 子包。
- 如果模板里配置了 `pkg_list.agent_skills`，发布会直接报错。

### 示例

```text
my-agent/
├── prompt.md
├── agent.yaml
├── tools/
│   └── search.json
└── memory/
    └── seed.json
```

### 发布结果

- `local_dir` 整体会被打包成 `agent` 子包。
- 顶层 `AppDoc` 是纯 meta，`content` 为空。

## 3. AppService

`AppService` 当前只支持 docker 镜像 tar 文件，不再支持其它扫描行为。

### 支持的子包

只支持下面两个 key：

- `amd64_docker_image`
- `aarch64_docker_image`

如果模板里还有别的 `pkg_list` 项，发布会直接报错。

### 准确目录结构

推荐目录结构如下：

```text
my-appservice/
├── amd64_docker_image.tar
└── aarch64_docker_image.tar
```

说明：

- 这两个文件名是固定约定。
- 哪个子包在模板里存在，就检查对应哪个 tar。
- 如果模板里只配置了 `amd64_docker_image`，那只需要：

```text
my-appservice/
└── amd64_docker_image.tar
```

- 如果模板里只配置了 `aarch64_docker_image`，那只需要：

```text
my-appservice/
└── aarch64_docker_image.tar
```

### 最小配置模式

也允许 `local_dir` 里**一个 tar 都没有**，只在 `app_doc_template` 中配置 docker image 信息，例如：

- `docker_image_name`
- 可选 `docker_image_digest`

此时 `publish_app_to_repo` 仍然允许发布。

### 最小配置示例

```text
my-appservice/
```

配套的模板大致是：

```json
{
  "pkg_list": {
    "amd64_docker_image": {
      "pkg_id": "demo-img-amd64#0.1.0",
      "docker_image_name": "buckyos/demo:0.1.0-amd64"
    },
    "aarch64_docker_image": {
      "pkg_id": "demo-img-aarch64#0.1.0",
      "docker_image_name": "buckyos/demo:0.1.0-aarch64"
    }
  }
}
```

### 发布结果

- 如果有 `*.tar`，会为对应 docker 子包生成 `PackageMeta`，并把 tar 打包进 named-store。
- 包内文件名会统一写成 `{app_id}.tar`，兼容当前 `app-loader` 的加载方式。
- 顶层 `AppDoc` 永远是**纯 meta**，没有自己的 chunk。
- 如果 `local_dir` 中没有任何 tar，则不会生成 docker 子包的内容包；此时发布结果就是一个纯 meta 的 `AppDoc`。

## 4. 当前发布流程

当前实现流程如下：

1. 扫描 `local_dir`
2. 对各个子包需要打包的目录或 tar 文件生成包内容
3. 将这些子包内容写入 `NamedStore`
4. 为实际存在内容的子包生成 `PackageMeta`
5. 将这些 `PackageMeta` 写入 `NamedStore`
6. 构造最终 `AppDoc`（纯 meta，不承载内容 chunk）
7. 将最终 `AppDoc` 写入 `NamedStore`
8. 将这些对象固定到 repo
9. 返回最终 `AppDoc` 的 `ObjId`

## 5. 建议

- `Web` 和 `Agent`：直接把可运行目录作为 `local_dir`。
- `Agent`：不要在模板里放 `agent_skills`。
- `AppService`：只按固定文件名准备 docker tar，不要再放别的发布输入。
- 如果只想让系统运行时通过 `docker_image_name` 拉镜像，可以让 `local_dir` 保持空目录，走纯 meta 发布。
