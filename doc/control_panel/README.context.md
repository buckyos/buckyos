# Control Panel Documentation

## Purpose

- `doc/control_panel/` 是 `control_panel` 的 canonical 文档目录。
- 目标不是堆 PRD，而是把产品意图、运行结构、接口规格、实现约束整理成可以长期维护的“规格即代码”文档集。
- 这里的文档默认服务两类读者：人类工程师，以及需要快速建立心智模型的 AI coding agent。

## Documentation Contract

- 所有 canonical 文档统一放在 `doc/control_panel/`。
- 所有 canonical 文件统一使用 `*.context.md` 命名。
- `README.context.md` 是第一入口，负责讲目标、边界、概念、读法、文档契约。
- `ARCHITECTURE.context.md` 负责讲运行结构、模块边界、数据流、鉴权流。
- `SPEC.context.md` 是主规格文件，负责讲路由、RPC、HTTP API、状态模型、实现与规划状态。
- `CONTEXT.context.md` 负责记录命名约定、非显而易见事实、技术债、迁移说明、不可破坏的原则。
- `product/control_panel/` 在迁移期保留为 historical product input，而不是长期 canonical source。

## Source Of Truth Policy

- 已实现行为以代码为最终事实来源。
- 对外契约以 `doc/control_panel/SPEC.context.md` 为文档事实来源。
- 运行结构与边界以 `doc/control_panel/ARCHITECTURE.context.md` 为文档事实来源。
- 约定、坑、技术债、不可变原则以 `doc/control_panel/CONTEXT.context.md` 为文档事实来源。
- 旧 PRD 中的内容在迁移后必须被标成 `Implemented`、`Planned` 或 `Historical`，不能继续以“当前事实”的语气悬空存在。

## Reading Order

- 第一次接触 `control_panel`：先读本文件。
- 要理解系统如何跑起来：读 `doc/control_panel/ARCHITECTURE.context.md`。
- 要改接口、页面、行为契约：读 `doc/control_panel/SPEC.context.md`。
- 要避免踩坑：读 `doc/control_panel/CONTEXT.context.md`。

## What Control Panel Is

- `control_panel` 是 BuckyOS 的系统级管理面板。
- 它不是单一页面，而是一组被统一承载的系统表面：web UI、系统管理 API、认证入口、嵌入式 Files 能力，以及若干安装/分享/运维相关流程。
- 在当前实现里，`control_panel` 既是用户看到的管理界面，也是多个后端能力的聚合入口。

## Goals

- 为个人、家庭、小团队提供 Zone 级别的统一管理体验。
- 覆盖系统主链路：登录与会话、概览与监控、网络与系统配置、文件与分享、应用与服务入口。
- 在不增加无谓分裂的前提下，把不同系统能力收敛在一个可理解、可演进、可验证的控制面板内。

## Scope And Boundaries

### In Scope

- `control_panel` 自身 UI 与其后端服务。
- `control_panel` 的 kRPC 表面：`/kapi/control-panel`。
- 内嵌 Files/Share HTTP 表面：`/api/*`。
- control panel 自己使用的认证与 SSO 相关浏览器流程。
- 与控制面板直接相关的产品概念、导航、状态模型、交互原则。

### Out Of Scope

- 独立 app store 服务本身的完整行为定义。
- 第三方应用内部逻辑。
- 纯远期设想但尚未进入近期实现边界的功能；这类内容只能以 `Planned` 形式出现。

## Core Concepts

- `Zone`: 用户拥有的逻辑云/集群，是 control panel 的管理对象范围。
- `OOD`: Zone 内承载关键系统能力的核心节点集合。
- `Node`: Zone 中的设备节点，可承载应用或系统服务。
- `NodeGateway`: 节点本地入口，常见一致入口为 `127.0.0.1:3180`。
- `Control Panel`: 面向系统管理的统一 UI 和 API hub。
- `Files`: 当前实现中嵌入在 `control_panel` 内的文件与分享子系统；产品方向上将从传统 file browser 演进为 AI-first 的数据工作台，但名称继续保持 `Files`。
- `Workspace`: 同属 control panel web 前端的一部分，但其主数据源并不是 Rust `control_panel` backend。
- `Message Hub`: 当前由 control panel desktop 提供启动入口的独立消息产品表面，主入口位于 `/message-hub/chat`；当前迁移阶段的 browser-safe chat adapter 仍暂时复用 `control_panel` service。
- `AI Models`: 当前作为 desktop 内的一等管理窗口，用于统一查看和管理 AI provider、模型别名、场景策略与诊断状态；现已通过 `control_panel` facade 对接 `AICC` 的 provider 配置、测试与 reload 流程。

## Design Philosophy

- 单一入口，明确边界：同一运行表面可以承载多个产品表面，但文档必须明确它们的所有权和数据源。
- 单向事实来源：实现现状由代码抽取结构，文档在此基础上补充意图，而不是反过来手写想象中的系统。
- 共享认证：Files、主控制面板、相关页面应尽量复用同一套 session 语义，重复造登录体系是退化。
- 文档分层：README 讲 why，Architecture 讲 how，Spec 讲 what，Context 讲 constraints。
- 规划显式化：愿景可以保留，但必须显式标记为 `Planned`，不能和已实现行为混写。

## UI Style Description

- 当前 control panel 的 UI 不是传统企业后台，也不是营销站，而是“带桌面隐喻的系统控制台”。
- 整体气质应接近个人云 / 家用 NAS / 本地系统桌面的控制中心：可靠、清爽、可操作，而不是高压、密集、官僚化的运维面板。
- 主体验是“系统工作台”而不是“表单堆叠器”：导航、状态、窗口、文件、监控面板都应服务于用户对系统状态的直觉理解。

### Visual Character

- 色彩上以青绿色系统主色为核心，配合暖色强调，形成“冷静控制 + 温和提醒”的双轴情绪。
- 造型上以圆角卡片、轻边框、柔和阴影、浅层半透明为主，避免厚重、拟物过强、或极端扁平。
- 背景上允许使用轻度渐变、柔和雾面底色、局部光晕来构建空间感，但不应演变成夸张装饰。
- 信息层级应通过版式、字号、留白、色彩浓度表达，而不是依赖大量分割线或高对比块面。

### Interaction Character

- 交互风格应克制、稳定、直接，避免炫技型动画。
- 所有 hover、focus、active 状态都应清楚，但不应造成布局跳动。
- 控件应保留足够触控尺寸，桌面与移动都要维持“像系统工具一样可靠”的操作感。

### Desktop Metaphor

- `DesktopHomePage` 代表当前 control panel 的核心风格方向：它不是普通 dashboard，而是轻量系统桌面。
- 窗口、dock、面板、快捷入口这些元素说明 control panel 的理想体验更接近 OS shell，而不是单页 BI 面板。
- 后续新增页面即便不使用窗口式布局，也应保持“系统工作台”语义，不要突然退化为模板化 CRUD 后台。

### Desktop As Product Core

- `/` 对应的 desktop 不是普通首页，而是 control panel 的核心产品表面。
- 它承载的是“全集成系统工作台”模型：monitor、network、containers、files、storage、logs、apps、settings、users 等模块不是靠一级路由切页组织，而是在同一个 desktop 容器里以窗口方式被打开、切换、最小化、最大化和聚焦。
- 这意味着 control panel 的首页语义更接近 window manager / workspace shell，而不是传统 dashboard landing page。
- 文档、设计、实现都应承认这一点：desktop 是主交互框架，路由只是进入 desktop 的入口，而不是 desktop 内部模块分割的唯一组织方式。

## Visual Consistency Direction

- 标题使用更有结构感的几何无衬线，正文使用高可读的人文无衬线；当前约定为 `Space Grotesk` + `Work Sans`。
- 颜色、圆角、阴影、间距都应被当成系统 token 维护，而不是在页面里零散硬编码。
- 左侧导航、卡片、表格、弹窗、工具栏、文件预览面板应共享同一套视觉语法：
  - 同类容器应有相近圆角等级
  - 同级控件应有相近高度和描边强度
  - 主按钮、次按钮、危险按钮要稳定复用同一组语义样式
- Files 虽然是更偏工具型的子系统，但仍应保持与主控制面板一致的配色、边框、状态语义和交互反馈。
- 任何视觉刷新都不应只改单页，应优先检查 `RootLayout`、Desktop、Files、弹窗、表格、状态卡片是否一起保持统一。

## UI References

- 当前 UI/UX 规则的实现侧说明见 `src/frame/control_panel/SKILL.md`。
- 入口壳层风格参考 `src/frame/control_panel/web/src/ui/RootLayout.tsx`。
- 桌面隐喻和系统工作台风格参考 `src/frame/control_panel/web/src/ui/pages/DesktopHomePage.tsx`。
- Files 工具面板风格参考 `src/frame/control_panel/web/src/ui/pages/FileManagerPage.tsx`。

## Ownership And Audience

- 产品设计、前端、后端、测试、文档维护者、AI coding agent 都应从这里开始。
- 本文件故意保持高信号、低细节，不直接承载大段 RPC 或路由清单。

## Glossary

- `Implemented`: 已可从代码或运行行为验证的事实。
- `Planned`: 目标明确但尚未完全落地的规格。
- `Derived`: 从代码结构抽取并整理后的事实表达。
- `Historical`: 保留为迁移背景或设计历史的内容。

## Migration Status

- `product/control_panel/README.md` 的产品定位、入口页意图主要迁移到这里。
- `product/control_panel/control_panel.md` 中的目标与边界、核心概念、角色模型、用户旅程、非功能性要求主要迁移到这里。
- 路由、RPC、HTTP API、鉴权细节不在本文件定义，统一迁移到 `SPEC.context.md` 与 `ARCHITECTURE.context.md`。
