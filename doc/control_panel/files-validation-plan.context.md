# Files Validation Plan

## Purpose

- 为 `control_panel` 第一阶段验证层提供 `files` 范围的执行清单。
- 当前目标不是重做 Files 架构，而是先把 `pnpm dev:mock` 跑通，并能验证 `desktop -> files` 主流程。
- 第一阶段以当前 `FileManagerPage` 为准，不把概念型 [FilesPage.tsx](/home/aa/app/base/buckyos/src/frame/control_panel/web/src/ui/pages/FilesPage.tsx) 纳入验证范围。

## Quick Start

```bash
cd src/frame/control_panel/web
pnpm install
pnpm dev:mock
```

验证入口：

- desktop 窗口模式：`http://127.0.0.1:4020/`
- 独立页面模式：`http://127.0.0.1:4020/files`
- 公开分享模式：`http://127.0.0.1:4020/share/share-welcome`

## Current Scope

### In Scope

- 文件浏览
- recent / starred / trash 切换
- 搜索
- 文本预览
- 图片预览
- 创建文件夹
- 重命名
- 移动到回收站 / restore / delete forever
- share list / create share / delete share
- public share 打开与预览
- upload session mock 流程

### Out Of Scope

- Office 文档在线预览
- 真实文件系统与真实权限链路
- 大规模 benchmark
- 真实 gateway 公开分享链路

## UI DataModel

### FilesShellState

- `mainTab: 'files' | 'shares' | 'editor'`
- `filesScope: 'browse' | 'recent' | 'starred' | 'trash'`
- `currentPath: string`
- `currentPathIsDir: boolean`
- `selectedPaths: string[]`
- `message: string`

### BrowseState

- `items: FileEntry[]`
- `loading: boolean`
- `searchActive: boolean`
- `searchResults: FileEntry[]`
- `searchTruncated: boolean`

### PreviewState

- `previewEntry: FileEntry | null`
- `previewKind`
- `previewTextContent`
- `previewImageSrc`
- `previewError`

### ShareState

- `shares: ShareItem[]`
- `shareDialog`
- `publicShareData: PublicShareResponse | null`
- `publicShareError: string`

### UploadState

- `uploadProgress: UploadProgressItem[]`
- `uploadPanelOpen: boolean`
- `uploadPaused: boolean`

### Required UI States

- `loading`
- `ready`
- `empty-directory`
- `empty-share-list`
- `empty-trash`
- `search-no-result`
- `preview-error`
- `uploading`

## Mock Runtime Design

- mock 模式通过 `VITE_CP_USE_MOCK=1` 打开
- 浏览器内 mock server 接管 `/api/*`
- 不依赖 Vite 代理到 `127.0.0.1:3180`
- session 由 mock auth 自动注入
- mock 数据目前是 in-memory state，刷新页面会回到初始 fixture

## Mock Dataset

默认 fixture 包含：

- `/Documents/Welcome.md`
- `/Documents/Runbook.json`
- `/Projects/ControlPanel/notes.txt`
- `/Pictures/nebula.svg`
- `/Uploads/`
- favorites 2 条
- recent 2 条
- recycle bin 1 条
- public shares 2 条

## Main Flows

1. 打开 `/`
2. 在 desktop 打开 `Files` 窗口
3. 浏览根目录
4. 进入 `Documents`
5. 搜索 `welcome`
6. 切到 `Recent`
7. 切到 `Starred`
8. 切到 `Trash`
9. 返回 `Browse`
10. 创建 share
11. 打开 `/share/share-welcome`
12. 返回 desktop 后执行一次 upload

## Done For This Stage

- `pnpm dev:mock` 可直接启动 Files
- `/files` 独立页面模式已可访问
- `desktop -> files` 主路径在 mock 环境下可演示
- 核心 `/api/*` 已有 mock handler
- 启动说明与覆盖范围已文档化

## Follow-up

- 将当前 in-memory mock server 逐步收口为 `FilesDataSource`
- 增加 Playwright mock smoke
- 给 fixture 增加场景切换
- 为 search / list / upload 加 benchmark 脚本
