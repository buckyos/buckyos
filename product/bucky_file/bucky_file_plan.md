# bucky-file 规划与里程碑

> 状态说明（2026-03）
>
> 本文最初按“独立服务进程”设计。当前代码主线已调整为：
> `files` 作为 `control_panel` 的内嵌模块提供能力，统一通过 `control_panel` 的 `/api` 对外暴露。
> 本文已按当前实现更新：`files` 由 `control_panel` 内嵌提供，
> 不再保留独立 `bucky-file` 服务代码路径。

## 1. 目标与定位

`bucky-file`（当前以内嵌 files 模块形态运行）是 BuckyOS 内建的新一代文件组件，面向以下目标：

- 在 BuckyOS 内部提供可控、可扩展、可持续演进的文件管理能力。
- 统一技术栈：前端 `React + TypeScript + Tailwind`，后端 `Rust`。
- 以服务化方式与系统网关、权限体系、控制台能力深度集成。
- 支持逐步迁移与灰度替换，保证上线过程可回滚。

## 2. 设计原则

- **系统一方实现**：核心能力由 BuckyOS 自有代码维护，便于后续定制。
- **兼容优先**：优先保持用户侧使用体验和入口稳定。
- **安全优先**：路径访问控制、会话安全、权限隔离为默认策略。
- **分阶段交付**：先打通最小闭环，再补齐高级能力，降低改造风险。

## 3. 架构总览

### 3.1 后端（Rust）

- 当前默认形态：`frame/control_panel` 内嵌文件模块（`file_manager`）。
- 对外接口由 `control_panel` 统一承载：
  - 外部/前端接口：`/api/*`（文件管理主接口）。
  - 控制面接口：`/kapi/control-panel`（统一控制面 RPC）。
- 核心子模块：
  - `auth`：登录、续签、会话验证。
  - `fs`：目录树、读写、移动、复制、删除、重命名。
  - `share`：分享链接、过期、口令保护。
  - `preview`：缩略图、媒体预览、文本预览。
  - `search`：文件搜索与流式返回。
  - `settings`：全局与用户设置。
  - `upload`：普通上传与分片上传。
  - `store`：元数据存储（SQLite）。

### 3.2 前端（React + TS + Tailwind）

- 当前前端位于 `frame/control_panel/web`，`FileManagerPage` 作为页面模块内嵌。
- 主要入口：Desktop 内嵌 Files 窗口、`/share/:shareId` 公共分享页。
- 页面与组件分层：
  - 页面层：文件列表、预览、编辑、设置、分享。
  - 组件层：工具栏、面包屑、上传面板、弹窗系统、预览组件。
  - API 层：统一请求封装、错误处理、重试与取消。

### 3.3 存储与数据

- 文件数据：使用用户目录与系统挂载点。
- 元数据：SQLite（用户、设置、分享、上传会话、索引状态）。
- 缓存：缩略图缓存、临时上传块缓存。

### 3.4 认证与权限

- 当前：Files 复用 Control Panel 会话，不再提供独立登录页面作为默认流程。
- 目标：与 BuckyOS 统一会话体系收敛，减少重复登录并保留细粒度权限校验。
- 权限模型：
  - 角色级：管理员 / 普通用户。
  - 动作级：读、写、创建、删除、重命名、分享、下载。
  - 路径级：用户作用域隔离，禁止越界访问。

## 4. 目录规划（当前主线）

```text
src/
  frame/
    control_panel/
      Cargo.toml
      src/
        main.rs
        file_manager.rs
        share_content_mgr.rs
      web/
        package.json
        vite.config.ts
        src/
          main.tsx
          routes/
          pages/
          components/
          api/
          styles/

doc/
  PRD/
    bucky_file/
      bucky_file_plan.md
      bucky_file_api.md (待补)
      bucky_file_migration.md (待补)
```

## 5. 功能分期

### P0（首发必须）

- 复用 control_panel 会话（无独立登录门槛）。
- 文件列表、上传、下载、删除、重命名、复制、移动。
- 文本文件查看与编辑。
- 基础预览（图片/文本）。
- 分享链接（含过期时间）。

### P1（高优先）

- 管理后台：用户与权限配置。
- 全局设置与用户默认设置。
- 搜索（流式返回）。
- 分片上传（大文件稳定性）。
- 更完善的预览能力（媒体/字幕）。

### P2（增强）

- 主题与品牌能力。
- 文件操作审计与可观测性增强。
- 性能优化（索引、缓存、批量操作）。

## 6. 里程碑计划

### M1：最小可用闭环（1-2 周）

- 完成 control_panel 内嵌 file_manager 与 Files Web 模块骨架。
- 打通会话复用、文件列表、上传、下载、删除。
- 完成本地构建与基本部署链路。

验收标准：

- 能在开发环境完整完成一次文件上传与下载回路。
- control_panel 服务可稳定启动，接口可观测，错误可追踪。

### M2：核心功能齐平（2-3 周）

- 完成移动/复制/重命名、预览、分享、设置、搜索。
- 完成权限动作与路径范围校验。
- 完成前后端错误码与提示文案收敛。

验收标准：

- 核心高频能力可用，权限校验符合预期。
- 大文件上传与并发操作场景通过冒烟验证。

### M3：系统集成与灰度替换（1-2 周）

- 接入网关默认入口与系统配置生成链路。
- 控制台增加跳转与基础状态展示。
- 保留回滚开关，支持按环境灰度。

验收标准：

- 默认入口可切换到 control_panel 内嵌 Files 模块。
- 回滚开关可在不改代码前提下生效。

### M4：稳定化与发布（1-2 周）

- 兼容性、性能、安全专项修复。
- 文档收口：部署、运维、迁移、故障排查。
- 发布版本并完成上线复盘。

验收标准：

- 关键缺陷清零。
- 发布流程可复现，回滚预案经过演练。

## 7. 需要修改的系统配置与构建项

以下为当前主线改造点（按文件）：

- `src/frame/control_panel/src/main.rs`
  - 在 control_panel 内初始化并挂载内嵌 file_manager，统一承载 `/api`。
- `src/frame/control_panel/src/file_manager.rs`
  - 实现文件浏览、编辑、上传会话、分享、公开访问等 HTTP API。
- `src/frame/control_panel/web/src/ui/pages/FileManagerPage.tsx`
  - Files 前端页面在 control_panel web 内运行，并对接 `/api/*`。
- `src/frame/control_panel/web/src/ui/pages/DesktopHomePage.tsx`
  - Desktop 集成 Files 窗口，并与 Storage 语义分离。

已完成的架构收敛（历史变更）:
- 从“独立 bucky-file 服务”收敛为“control_panel 内嵌 files 模块”。
- 调度与系统配置中不再将 bucky-file 作为默认独立服务启动项。

## 8. 开发与运行建议

### 8.1 本地开发

- 后端：在 `src/` 下使用 `cargo run -p control_panel`。
- 前端：在 `src/frame/control_panel/web` 下使用 `pnpm dev`。
- 联调：前端通过 Vite 代理转发到 `control_panel` 的 `/api` 与 `/kapi/control-panel`。

推荐部署流（本机）:
- `source /root/app/myenv/bin/activate`
- `cd src && buckyos-build -s control_panel control_panel_web`
- `cd src && buckyos-install`
- `systemctl restart buckyos`

### 8.2 集成构建

- 在 `src/` 执行模块化构建（`control_panel` + `control_panel_web`）。
- 安装后通过系统服务重启验证入口可达。

## 9. 测试计划

- 单元测试：路径安全、权限校验、分享策略、上传分片。
- 集成测试：登录流、文件 CRUD、批量操作、分享访问。
- 回归测试：控制台跳转、网关入口、系统配置读取。
- 压测与稳定性：并发上传、目录深层遍历、长时间搜索。

## 10. 迁移与回滚策略

- 迁移原则：先并行运行，再灰度切流，最后默认替换。
- 回滚方式：保留旧入口映射与配置开关，支持一键切回。
- 数据策略：元数据采用可迁移设计，避免与历史数据强耦合。

## 11. 风险与对策

- **权限模型不一致风险**：先落地最小权限矩阵并补齐自动化测试。
- **路径安全风险**：统一路径规范化与越权拦截中间层。
- **性能风险**：提前引入缓存、分页、流式返回策略。
- **发布风险**：通过灰度与回滚开关降低上线冲击。

## 12. 交付物清单

- 规划文档（本文件）。
- API 文档（待补）：`product/bucky_file/bucky_file_api.md`。
- 迁移文档（待补）：`product/bucky_file/bucky_file_migration.md`。
- 代码骨架（后续阶段）。
