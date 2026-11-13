# 桌面拖动槽位背景修复

生产构建将共享的 `src/index.css` 拆入共享 CSS 文件，加载顺序早于 `react-grid-layout` 的库样式。双方原来使用相同的 `.react-grid-item.react-grid-placeholder` 选择器，库的 `background: red` 和 `opacity: 0.2` 最终覆盖桌面样式，导致槽位显示淡红色。

在 `src/frame/desktop/src/index.css` 为占位规则添加 `.react-grid-layout >`，使其优先级不依赖 CSS 文件加载顺序。背景改为 10% 透明度的主题蓝色，边框改为 25% 透明度的细实线，并显式设置 `opacity: 1`，让透明度由颜色控制。

验证通过：

- `pnpm run check`。
- `pnpm exec vite build --mode development --outDir /tmp/buckyos-desktop-drag-preview`：使用 Mock 数据构建，保留打包后的 CSS 分块与加载方式。
- `pnpm exec playwright test tests/e2e/pages/desktop.spec.ts --grep 'dragging a launcher icon keeps items that overflowed to another page' --output /tmp/buckyos-desktop-drag-e2e`：1 个用例通过。
- Chromium 打开构建产物，在浅色、深色主题下实际拖动图标，检查计算样式、截图和释放后的占位消失；背景透明度为 10%，边框透明度为 25%，未出现红色。

本次仅调整 CSS，不涉及 CLI、API、配置、运行路径或发布流程，内置 skill 手册无需修改。未运行完整系统构建或真实 Zone DV，未部署到运行环境。
