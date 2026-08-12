# BuckyOS 系统对话框实现方案

## 1. 背景

BuckyOS 中的文件选择器并不是简单的下拉框或列表组件，而是一个接近完整应用的复杂交互界面，通常包含：

- 目录导航与面包屑
- 文件和目录列表
- 单选、多选与复选框
- 搜索、排序、筛选
- 文件预览
- 最近访问、收藏、共享空间等入口
- 权限、登录态与错误处理
- 移动端安全区、返回手势和软键盘适配

典型使用场景是：用户正在聊天、写邮件或编辑文档，点击“添加附件”，临时打开 BuckyOS 文件选择器；选择完成后，返回原界面继续当前操作。

这类交互的关键要求是：

1. 不离开当前页面，不破坏宿主应用的状态。
2. 视觉和交互上接近系统原生文件选择器。
3. 文件选择器与宿主页面之间尽量隔离，避免样式、依赖、事件和状态冲突。
4. 能够被不同技术栈、不同域名的 Web 应用统一接入。
5. 在桌面浏览器、移动浏览器和 WebView 中均能稳定工作。

---

## 2. 方案结论

采用以下结构：

```text
宿主页面
└── BuckyOS Dialog SDK 动态插入全屏 Overlay Shell
    └── iframe
        └── 独立运行的 BuckyOS File Picker React App
```

即：

- **SDK 负责对话框外壳和生命周期。**
- **iframe 负责承载完整文件选择器应用。**
- **宿主页面不发生路由跳转。**
- **选择结果通过受控的 `postMessage` 协议返回。**
- **调用方以 Promise 形式获得结果。**

推荐的调用方式：

```ts
const files = await buckyos.dialog.openFilePicker({
  multiple: true,
  accept: ["image/*", "application/pdf"],
});
```

用户取消时返回明确的取消结果或抛出可识别的取消异常；用户确认后返回轻量的 `FileRef`，而不是直接传输文件二进制内容。

---

## 3. 为什么不采用其他方案

### 3.1 不使用页面跳转

不推荐：

```text
/chat
  → /file-picker
  → /chat
```

页面跳转可能导致以下问题：

- 聊天草稿丢失
- 消息列表滚动位置丢失
- 当前输入框焦点丢失
- React 页面被卸载后，内存状态丢失
- 返回时需要额外恢复页面状态
- 移动端浏览器可能重新加载前一个页面
- WebView 的历史栈行为不完全一致

即使通过全局状态、Session Storage 或路由缓存恢复，也会显著增加宿主应用的实现复杂度。

### 3.2 不依赖 `window.open`

不推荐：

```ts
window.open(pickerUrl);
```

原因包括：

- 移动端通常不会表现为桌面上的独立小窗口
- 部分浏览器和 WebView 会拦截弹窗
- 新窗口的关闭和回传控制不稳定
- iOS Safari 的窗口、标签页和登录态行为较难统一
- 用户容易失去原页面上下文
- 无法稳定实现“系统选择器覆盖当前应用”的体验

### 3.3 不把复杂 Picker 直接挂到宿主 React 树中

纯 DOM Overlay 或直接加载 React 组件可以工作，但对于跨应用 SDK 存在较高冲突风险：

- CSS Reset、字体、主题变量相互影响
- 宿主和 Picker 的 React 版本可能不同
- Redux、Router、国际化等上下文相互耦合
- 全局键盘事件和 Pointer 事件冲突
- Portal、Focus Trap、弹窗层级相互影响
- 依赖包重复或版本不兼容
- 宿主应用可能使用 Shadow DOM、微前端或自定义渲染环境

因此，复杂文件选择器优先作为独立 Web App 运行在 iframe 内。

---

## 4. 总体架构

### 4.1 组件划分

```text
BuckyOS System Dialog
├── Dialog SDK
│   ├── Overlay Manager
│   ├── iframe Manager
│   ├── Message Channel
│   ├── Focus Manager
│   ├── Scroll Lock
│   ├── Back/Escape Handler
│   └── Lifecycle Manager
│
└── File Picker App
    ├── Directory Navigator
    ├── File List
    ├── Search / Sort / Filter
    ├── Selection Model
    ├── Preview
    ├── Authentication
    ├── Confirm / Cancel
    └── Error and Loading UI
```

### 4.2 DOM 结构

SDK 打开对话框后，在宿主页面的 `document.body` 下插入独立根节点：

```html
<body>
  <div id="app">
    <!-- 宿主应用 -->
  </div>

  <div
    id="buckyos-dialog-root"
    data-buckyos-dialog="file-picker"
    role="dialog"
    aria-modal="true"
  >
    <div class="buckyos-dialog-backdrop"></div>

    <div class="buckyos-dialog-surface">
      <iframe
        title="BuckyOS 文件选择器"
        src="https://picker.buckyos.com/"
      ></iframe>
    </div>
  </div>
</body>
```

对话框关闭后，SDK 应完整移除该节点，并恢复宿主页面此前的滚动、焦点和可访问性状态。

---

## 5. Overlay Shell 设计

### 5.1 Overlay 的职责

Overlay Shell 只负责宿主侧能力，不实现文件选择器内部业务：

- 创建与销毁全屏覆盖层
- 保证最高层级显示
- 锁定宿主页面滚动
- 保存和恢复焦点
- 管理背景页面的交互隔离
- 处理 `Escape`
- 协调浏览器返回键
- 显示 iframe 初始加载状态
- 处理 iframe 加载失败和超时
- 建立可信通信通道
- 将结果转换为 Promise 返回

### 5.2 推荐样式

```css
.buckyos-dialog-root {
  position: fixed;
  inset: 0;
  z-index: 2147483647;
  display: flex;
  width: 100%;
  height: 100dvh;
  overflow: hidden;
  overscroll-behavior: contain;
  isolation: isolate;
}

.buckyos-dialog-backdrop {
  position: absolute;
  inset: 0;
  background: rgba(0, 0, 0, 0.42);
}

.buckyos-dialog-surface {
  position: relative;
  width: 100%;
  height: 100%;
  background: #fff;
  overflow: hidden;
}

.buckyos-dialog-surface iframe {
  display: block;
  width: 100%;
  height: 100%;
  border: 0;
  background: transparent;
}
```

移动端默认使用真正的全屏 Surface；桌面端可以根据产品要求使用全屏或有限尺寸的居中窗口。

### 5.3 移动端高度

优先使用：

```css
height: 100dvh;
```

而不是仅使用：

```css
height: 100vh;
```

`100dvh` 能更好地适应移动浏览器地址栏的展开和收起。

为兼容旧环境，可以提供回退：

```css
height: 100vh;
height: 100dvh;
```

---

## 6. SDK API 设计

### 6.1 基础接口

```ts
interface OpenFilePickerOptions {
  multiple?: boolean;
  accept?: string[];
  startLocation?: FileLocationRef;
  maxSelection?: number;
  allowDirectories?: boolean;
  showPreview?: boolean;
  title?: string;
  locale?: string;
  theme?: "light" | "dark" | "system";
  signal?: AbortSignal;
}

interface FileRef {
  id: string;
  name: string;
  kind: "file" | "directory";
  mimeType?: string;
  size?: number;
  modifiedAt?: string;
  source: "buckyos";
  objectUrl?: string;
  thumbnailUrl?: string;
  metadata?: Record<string, unknown>;
}

interface FilePickerResult {
  files: FileRef[];
}

interface BuckyOSDialog {
  openFilePicker(
    options?: OpenFilePickerOptions
  ): Promise<FilePickerResult>;
}
```

调用示例：

```ts
try {
  const result = await buckyos.dialog.openFilePicker({
    multiple: true,
    maxSelection: 10,
    accept: ["image/*", "application/pdf"],
  });

  attachFiles(result.files);
} catch (error) {
  if (error instanceof BuckyOSDialogCancelledError) {
    return;
  }

  showError("无法打开文件选择器");
}
```

### 6.2 不返回二进制文件

iframe 与宿主之间不应通过 `postMessage` 直接传输大文件内容。

推荐返回：

- 文件对象 ID
- 文件名
- MIME 类型
- 大小
- 缩略图 URL
- 授权后的临时访问引用
- BuckyOS ObjectRef / FileRef

文件真正发送、上传或读取时，由宿主通过 BuckyOS API 使用该引用继续处理。

### 6.3 并发策略

默认同一页面仅允许一个系统对话框处于打开状态。

第二次调用可以选择：

1. 拒绝并返回 `DialogAlreadyOpenError`
2. 排队等待
3. 关闭前一个再打开新对话框

第一阶段推荐使用“拒绝重复打开”，行为最明确。

---

## 7. iframe 通信协议

### 7.1 基本原则

通信协议必须具备：

- 固定命名空间
- 协议版本
- 唯一会话 ID
- 唯一请求 ID
- 严格的 origin 校验
- 明确的消息类型
- 明确的成功、取消和错误结果
- 防止其他 iframe 或页面伪造消息

### 7.2 消息结构

```ts
interface BuckyOSDialogMessage<T = unknown> {
  namespace: "buckyos.system-dialog";
  version: 1;
  dialogId: string;
  type: string;
  payload?: T;
}
```

### 7.3 建议消息类型

宿主发送给 iframe：

```text
host:init
host:close-request
host:theme-change
host:locale-change
```

iframe 发送给宿主：

```text
picker:ready
picker:selection-change
picker:confirm
picker:cancel
picker:error
picker:request-close
```

### 7.4 初始化流程

```text
1. SDK 创建 Overlay 和 iframe
2. iframe 页面加载
3. iframe 向宿主发送 picker:ready
4. SDK 校验 source、origin、dialogId
5. SDK 发送 host:init
6. Picker 进入可交互状态
```

初始化消息示例：

```ts
iframe.contentWindow?.postMessage(
  {
    namespace: "buckyos.system-dialog",
    version: 1,
    dialogId,
    type: "host:init",
    payload: {
      multiple: true,
      accept: ["image/*"],
      maxSelection: 10,
      locale: "zh-CN",
      theme: "system",
    },
  },
  PICKER_ORIGIN
);
```

### 7.5 结果回传

```ts
window.parent.postMessage(
  {
    namespace: "buckyos.system-dialog",
    version: 1,
    dialogId,
    type: "picker:confirm",
    payload: {
      files: selectedFiles,
    },
  },
  HOST_ORIGIN
);
```

### 7.6 校验要求

宿主收到消息时至少校验：

```ts
function isTrustedPickerMessage(
  event: MessageEvent,
  iframe: HTMLIFrameElement,
  expectedOrigin: string,
  dialogId: string
): boolean {
  const message = event.data;

  return (
    event.origin === expectedOrigin &&
    event.source === iframe.contentWindow &&
    message?.namespace === "buckyos.system-dialog" &&
    message?.version === 1 &&
    message?.dialogId === dialogId
  );
}
```

绝对不要使用：

```ts
postMessage(data, "*");
```

除非在一个极其受限、无法获得目标 origin 的特殊启动阶段，并且后续立即切换到确定的 origin。

### 7.7 推荐使用 MessageChannel

在握手完成后，可以通过 `MessageChannel` 建立每个对话框独享的通信通道：

```ts
const channel = new MessageChannel();

iframe.contentWindow?.postMessage(
  {
    namespace: "buckyos.system-dialog",
    version: 1,
    dialogId,
    type: "host:connect",
  },
  PICKER_ORIGIN,
  [channel.port2]
);

channel.port1.onmessage = handlePickerMessage;
```

这样可以减少全局 `window.message` 监听带来的冲突，也更容易隔离多个实例。

---

## 8. 生命周期

### 8.1 状态机

```text
idle
  ↓ open()
mounting
  ↓ iframe created
loading
  ↓ picker:ready
active
  ├── picker:confirm → closing → closed
  ├── picker:cancel  → closing → closed
  ├── host abort     → closing → closed
  └── error          → closing → closed
```

### 8.2 清理要求

无论成功、取消、异常或外部中止，都必须执行统一清理：

- 移除 `message` 监听器
- 关闭 `MessagePort`
- 清理加载超时计时器
- 移除 iframe
- 移除 Overlay 根节点
- 恢复 `body` 样式
- 恢复背景节点的 `inert` 或 `aria-hidden`
- 恢复打开前获得焦点的元素
- 移除 `popstate`、`keydown` 等监听器
- 解除 SDK 的“对话框已打开”状态

推荐将清理函数设计为幂等操作，避免多条结束路径造成重复清理。

---

## 9. 宿主页面状态保护

采用 DOM Overlay 而不是路由跳转后，宿主 React 页面不会卸载，因此以下状态可以自然保留：

- 聊天输入草稿
- 当前会话
- 消息列表滚动位置
- 已输入的富文本内容
- 页面内临时状态
- React Context
- Redux / Zustand 状态
- 当前路由
- 未提交表单

但是，仍需处理滚动和焦点。

### 9.1 滚动锁定

不要只使用简单的：

```ts
document.body.style.overflow = "hidden";
```

在 iOS 上，这可能导致页面位置变化。推荐保存当前滚动位置：

```ts
const scrollY = window.scrollY;

Object.assign(document.body.style, {
  position: "fixed",
  top: `-${scrollY}px`,
  left: "0",
  right: "0",
  width: "100%",
});

function restoreBodyScroll() {
  document.body.style.position = "";
  document.body.style.top = "";
  document.body.style.left = "";
  document.body.style.right = "";
  document.body.style.width = "";
  window.scrollTo(0, scrollY);
}
```

SDK 应保存宿主原来的内联样式，并在关闭时原样恢复，不能假设这些属性之前为空。

### 9.2 背景交互隔离

对话框打开后，宿主内容不应被点击或被辅助技术继续聚焦。

优先使用：

```html
<div id="app" inert></div>
```

对于不支持 `inert` 的环境，可以配合：

- `aria-hidden="true"`
- Overlay 拦截指针事件
- 焦点保护逻辑

关闭时恢复原属性值。

### 9.3 焦点恢复

打开时记录：

```ts
const previouslyFocused =
  document.activeElement instanceof HTMLElement
    ? document.activeElement
    : null;
```

关闭后：

```ts
previouslyFocused?.focus({ preventScroll: true });
```

如果原元素已被卸载，则将焦点恢复到触发附件选择的按钮或宿主应用根节点。

---

## 10. 移动端交互

### 10.1 视觉结构

移动端推荐：

```text
┌────────────────────────────┐
│ 取消      选择文件      完成 │
├────────────────────────────┤
│ 路径 / 搜索 / 过滤           │
├────────────────────────────┤
│                            │
│ 文件与目录列表               │
│                            │
├────────────────────────────┤
│ 已选择 3 项                  │
└────────────────────────────┘
```

整个 Picker 占满可视区域，看起来像系统临时界面，但技术上仍然留在当前宿主页面上。

### 10.2 安全区

Picker iframe 内部应适配：

```css
padding-top: env(safe-area-inset-top);
padding-right: env(safe-area-inset-right);
padding-bottom: env(safe-area-inset-bottom);
padding-left: env(safe-area-inset-left);
```

页面建议设置：

```html
<meta
  name="viewport"
  content="width=device-width, initial-scale=1, viewport-fit=cover"
/>
```

### 10.3 返回键

返回行为建议按以下优先级处理：

1. 如果 Picker 内部有预览页或子目录历史，先返回 Picker 内部上一层。
2. 如果已在 Picker 根级，浏览器返回键用于取消并关闭 Picker。
3. 不应直接让返回键离开宿主页面。

一种实现方式是在打开时压入临时 history state：

```ts
history.pushState(
  { buckyosDialog: dialogId },
  "",
  window.location.href
);
```

监听 `popstate` 后发出关闭请求。

需要注意：

- 不修改宿主 URL。
- 关闭对话框时只清理自己压入的 history 状态。
- 避免与 React Router、Vue Router 等宿主路由产生不可预测的竞争。
- 如果无法安全接管历史栈，则优先由 Picker 内部提供明确的“取消”按钮，不强制劫持返回键。

在 App WebView 中，应优先使用宿主提供的返回键桥接。

### 10.4 软键盘

搜索框聚焦后，软键盘会改变 iframe 的可视区域。Picker 内部应：

- 使用弹性布局，而非固定像素高度
- 监听 `visualViewport`
- 保证搜索结果和确认按钮可见
- 避免把关键操作固定在被键盘遮挡的位置
- 在关闭前主动让输入框失焦

---

## 11. React 实现策略

### 11.1 SDK 不依赖 React

SDK 应使用原生 DOM 和 TypeScript 实现，以便接入：

- React
- Vue
- Svelte
- 原生 JavaScript
- 微前端
- 第三方网站

SDK 不应要求宿主安装特定 React 版本。

### 11.2 Picker 是独立 React App

文件选择器业务继续使用 React 实现，但作为独立应用构建：

```text
packages/
├── dialog-sdk/
│   ├── src/
│   └── dist/
│
├── file-picker-app/
│   ├── src/
│   └── dist/
│
└── dialog-protocol/
    ├── types.ts
    └── validators.ts
```

其中：

- `dialog-sdk`：发布为 npm 包或静态 SDK。
- `file-picker-app`：独立部署到 BuckyOS 服务。
- `dialog-protocol`：供 SDK 与 Picker 共同使用的类型和协议定义。

### 11.3 不共享运行时状态

SDK 和 iframe 内 React App 不直接共享：

- React Context
- Redux Store
- DOM 节点
- JS 对象引用
- 全局变量

所有交互通过协议完成。

这样可以显著降低跨应用耦合。

---

## 12. iframe 配置

### 12.1 sandbox

可以根据 Picker 所需能力设置：

```html
<iframe
  sandbox="
    allow-scripts
    allow-forms
    allow-same-origin
    allow-downloads
    allow-popups-to-escape-sandbox
  "
></iframe>
```

具体权限应遵循最小授权原则。

需要谨慎评估：

- 是否需要下载
- 是否需要打开新窗口
- 是否需要剪贴板
- 是否需要摄像头或麦克风
- 是否需要浏览器文件上传
- 是否需要 WebAuthn
- 是否需要同源 Cookie

不需要的能力不要加入。

### 12.2 Permissions Policy

可以通过 `allow` 属性限制能力：

```html
<iframe
  allow="
    clipboard-read 'none';
    clipboard-write 'none';
    camera 'none';
    microphone 'none';
    geolocation 'none'
  "
></iframe>
```

如果 Picker 未来需要某项能力，应按产品需求显式开启，而不是默认全部开放。

### 12.3 加载失败

SDK 应处理：

- DNS 或网络失败
- iframe 页面加载超时
- Picker 初始化失败
- 登录态失效
- 协议版本不兼容
- 跨域策略错误

建议在 Overlay Shell 中提供轻量错误页：

```text
无法加载 BuckyOS 文件选择器

[重试] [取消]
```

---

## 13. 登录态与授权

### 13.1 同站部署

理想情况下，宿主和 Picker 部署在可共享安全登录态的同站域名下，例如：

```text
app.example.com
picker.example.com
```

但不要依赖所有浏览器都允许第三方 Cookie。

### 13.2 推荐短期会话票据

宿主 SDK 可以先向 BuckyOS 服务请求一次性的 Picker Session：

```text
宿主应用
  → BuckyOS API：创建 Picker Session
  ← sessionId / short-lived token
  → iframe URL 或 host:init 消息
```

票据应具备：

- 短有效期
- 一次性或有限次数使用
- 明确的宿主 origin
- 明确的用户和权限范围
- 明确的可访问存储空间
- 明确的文件类型与数量限制

不要把长期访问令牌放在 iframe URL 中。

### 13.3 登录失效

Picker 检测到未登录时，可以在 iframe 内显示登录流程，但应尽量避免再创建第二层弹窗。

登录成功后，Picker 恢复此前的目录位置与选中状态。

---

## 14. 安全要求

必须满足：

1. SDK 固定或显式配置可信 Picker origin。
2. 所有 `message` 事件校验 `origin`。
3. 校验 `event.source === iframe.contentWindow`。
4. 校验 `namespace`、`version` 和 `dialogId`。
5. 消息 Payload 使用运行时 Schema 校验。
6. 不使用 `eval` 或动态注入不可信脚本。
7. FileRef 不包含超出调用方权限的真实存储路径。
8. 临时访问 URL 需要短有效期。
9. Picker Session 与宿主 origin 绑定。
10. iframe 使用合适的 CSP、sandbox 和 Permissions Policy。
11. 文件名、路径和元数据展示必须防止 XSS。
12. 关闭对话框后立即销毁会话和通信端口。
13. 对确认操作防止重复提交。
14. 对超大选择数量设置服务端和客户端双重限制。

---

## 15. 可访问性

Overlay 根节点：

```html
role="dialog"
aria-modal="true"
aria-label="BuckyOS 文件选择器"
```

建议满足：

- 打开后将焦点移入 Picker。
- 背景内容不可聚焦。
- 键盘用户可完成目录浏览、选择、确认和取消。
- `Escape` 可以取消，但不能误关闭未确认的重要操作。
- 列表项具备明确的选中状态。
- 图标按钮具备可读的 `aria-label`。
- 错误和加载状态使用适当的 Live Region。
- 关闭后焦点恢复到原附件按钮。

iframe 自身是独立焦点上下文，Picker App 内仍需要实现完整焦点管理。

---

## 16. 性能策略

### 16.1 预连接

宿主可以在用户较可能使用附件功能时进行：

```html
<link rel="preconnect" href="https://picker.buckyos.com" />
```

也可以在附件按钮进入视口或首次悬停时预加载必要资源。

### 16.2 不常驻复杂 iframe

默认不建议在页面启动时就常驻隐藏 iframe，原因包括：

- 长期占用内存
- 后台网络和定时器开销
- 登录态和协议状态更难管理
- 多宿主页面同时存在时浪费资源

推荐按需创建，关闭即销毁。

如果实际测量显示首次打开耗时不可接受，可以引入受控预热机制。

### 16.3 大目录列表

Picker App 内部应考虑：

- 虚拟列表
- 分页或增量加载
- 缩略图懒加载
- 搜索防抖
- 目录请求取消
- 浏览历史缓存
- 选中状态与列表数据解耦

---

## 17. 错误模型

建议定义：

```ts
class BuckyOSDialogError extends Error {
  code: string;
}

class BuckyOSDialogCancelledError extends BuckyOSDialogError {}
class BuckyOSDialogLoadError extends BuckyOSDialogError {}
class BuckyOSDialogProtocolError extends BuckyOSDialogError {}
class BuckyOSDialogAlreadyOpenError extends BuckyOSDialogError {}
class BuckyOSDialogAbortedError extends BuckyOSDialogError {}
class BuckyOSDialogPermissionError extends BuckyOSDialogError {}
```

宿主应能够区分：

- 用户主动取消
- 网络失败
- SDK 加载失败
- 权限不足
- 登录失效
- 协议不兼容
- 外部 AbortSignal 中止

用户主动取消不应记录为系统异常。

---

## 18. 核心伪代码

```ts
export async function openFilePicker(
  options: OpenFilePickerOptions = {}
): Promise<FilePickerResult> {
  if (activeDialog) {
    throw new BuckyOSDialogAlreadyOpenError();
  }

  const dialogId = crypto.randomUUID();
  const snapshot = captureHostState();

  const overlay = createOverlay(dialogId);
  const iframe = createPickerIframe();

  activeDialog = { dialogId, overlay, iframe };

  document.body.appendChild(overlay);
  lockHostPage(snapshot);
  overlay.querySelector(".buckyos-dialog-surface")?.appendChild(iframe);

  return new Promise<FilePickerResult>((resolve, reject) => {
    let settled = false;

    const finish = (
      action: () => void
    ) => {
      if (settled) return;
      settled = true;

      cleanup();
      action();
    };

    const onMessage = (event: MessageEvent) => {
      if (
        !isTrustedPickerMessage(
          event,
          iframe,
          PICKER_ORIGIN,
          dialogId
        )
      ) {
        return;
      }

      const message = event.data;

      switch (message.type) {
        case "picker:ready":
          sendInitMessage(iframe, dialogId, options);
          break;

        case "picker:confirm":
          finish(() => {
            resolve(validatePickerResult(message.payload));
          });
          break;

        case "picker:cancel":
          finish(() => {
            reject(new BuckyOSDialogCancelledError());
          });
          break;

        case "picker:error":
          finish(() => {
            reject(toDialogError(message.payload));
          });
          break;
      }
    };

    const onAbort = () => {
      finish(() => {
        reject(new BuckyOSDialogAbortedError());
      });
    };

    const cleanup = () => {
      window.removeEventListener("message", onMessage);
      options.signal?.removeEventListener("abort", onAbort);
      overlay.remove();
      restoreHostPage(snapshot);
      activeDialog = null;
    };

    window.addEventListener("message", onMessage);
    options.signal?.addEventListener("abort", onAbort, {
      once: true,
    });
  });
}
```

生产实现还应加入：

- 加载超时
- MessageChannel
- Schema 校验
- focus 和 history 管理
- iframe load/error 处理
- 幂等清理
- 遥测与日志

---

## 19. 桌面端与移动端差异

架构保持一致，只调整 Surface 展示方式。

### 移动端

```text
全屏 Overlay
全屏 iframe
顶部取消与完成
适配安全区
支持返回手势
```

### 桌面端

可选：

```text
居中大尺寸对话框
或全屏工作区
```

例如：

```css
@media (min-width: 768px) {
  .buckyos-dialog-root {
    padding: 32px;
  }

  .buckyos-dialog-surface {
    max-width: 1180px;
    max-height: 860px;
    margin: auto;
    border-radius: 16px;
    box-shadow:
      0 24px 80px rgba(0, 0, 0, 0.28);
  }
}
```

不过文件浏览空间需求较大，桌面端也可以默认接近全屏。

---

## 20. 测试矩阵

至少覆盖：

### 浏览器

- iOS Safari
- Android Chrome
- 桌面 Chrome
- 桌面 Safari
- Firefox
- Edge

### 容器环境

- 普通浏览器
- PWA
- iOS WKWebView
- Android WebView
- BuckyOS 自有客户端 WebView

### 宿主状态

- 页面已滚动
- 输入框内有草稿
- 软键盘已打开
- 宿主已有 Modal
- 宿主使用 React Router
- 宿主使用 CSS Transform
- 宿主有高 z-index 元素
- 页面处于横屏
- 浏览器地址栏展开和收起

### Picker 行为

- 单选
- 多选
- 取消
- 确认
- 目录切换
- 搜索
- 登录过期
- 网络中断
- iframe 加载失败
- 协议版本不一致
- 重复点击打开按钮
- 选择数量达到上限
- 文件权限在选择期间变化

### 恢复验证

关闭后验证：

- 宿主页面滚动位置不变
- 聊天草稿不变
- 当前路由不变
- 焦点回到附件按钮
- `body` 样式恢复
- 背景页面重新可交互
- 没有残留事件监听器
- 没有残留 iframe
- 浏览器返回行为正常

---

## 21. 分阶段实现建议

### 第一阶段：最小可用版本

- 原生 DOM SDK
- 全屏 Overlay
- iframe File Picker
- `postMessage` 通信
- origin、source、dialogId 校验
- Promise API
- 确认、取消、加载失败
- body 滚动锁定
- 焦点恢复
- 移动端 `100dvh` 和安全区适配

### 第二阶段：稳定性增强

- MessageChannel
- 协议 Schema 校验
- 加载超时和重试
- AbortSignal
- history / WebView 返回键支持
- 统一错误码
- 主题和语言同步
- 登录会话票据
- 完整可访问性

### 第三阶段：平台化

- 通用 `buckyos.dialog.open()` API
- 支持文件选择、目录选择、分享、授权等系统对话框
- 统一对话框协议
- SDK 多版本兼容
- 遥测、性能指标与故障监控
- 桌面端尺寸策略
- 预热和资源缓存

---

## 22. 可扩展为统一系统对话框框架

文件选择器可以作为第一个系统对话框，后续统一成：

```ts
buckyos.dialog.open({
  type: "file-picker",
  options: {
    multiple: true,
  },
});
```

未来可扩展：

```text
file-picker
directory-picker
save-file
share
permission
account-selector
device-selector
app-selector
```

每种对话框复用：

- Overlay Shell
- iframe 生命周期
- 通信协议
- 安全策略
- 焦点与滚动管理
- 错误处理
- 移动端适配

而业务 UI 作为不同的 iframe App 独立演进。

---

## 23. 最终决策

BuckyOS 文件选择器采用：

> **宿主页面内的全屏临时 Overlay + 独立 iframe Picker App + Promise SDK + 安全消息协议。**

它在用户体验上类似系统本地文件选择器，但不会离开当前页面，也不会卸载宿主应用。

该方案相较于页面跳转、`window.open` 和直接挂载复杂 React 组件，能够更好地满足：

- 状态不丢失
- 移动端稳定
- 跨应用接入
- 样式和依赖隔离
- 安全边界清晰
- 文件选择器独立迭代
- 后续扩展为 BuckyOS 统一系统对话框平台
