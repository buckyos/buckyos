按下面的文档呀求，升级现在的WebDesktop实现

===
# WebDesktop Grid Slot 规范


---

## 1. 核心模型

```
Slot 宽度 = 容器宽度 / 列数       （动态计算，永远不要写死 px）
Slot 高度 = 三档固定值             （由用户偏好切换）
Grid Gap  = 2px                   （固定，不为 0）
```

---

## 2. 响应式列数

列数由 JS 根据**容器宽度**（不是 window 宽度）在断点处设置 `--grid-columns`：

| 容器宽度             | 列数   | 典型场景            |
|---------------------|-------|-------------------|
| < 480px             | 4     | 手机竖屏            |
| 480px – 767px       | 5–6   | 手机横屏、小平板      |
| 768px – 1023px      | 6–8   | 平板竖屏            |
| 1024px – 1439px     | 8–10  | 平板横屏、小桌面窗口   |
| ≥ 1440px            | 10–12 | 桌面全屏            |

用 `ResizeObserver` 监听容器而非 window，以支持多窗口场景。

不要用 `auto-fill` + `minmax`。我们需要精确控制列数，保证 Widget span 的整除性。

---

## 3. Slot 高度三档

高度增量 16px，符合 8pt 网格，保证 @2x/@3x 屏幕下无亚像素模糊。

| 档位     | 字号    | 行高    | Slot 高度 | 图标尺寸  |
|---------|--------|--------|----------|---------|
| Small   | 12px   | 16px   | 92px     | 40×40   |
| Medium  | 14px   | 20px   | 108px    | 48×48   |
| Large   | 16px   | 24px   | 124px    | 56×56   |

**Medium 为默认档位。** Small 仅作为高密度偏好选项，不作为默认值。

---

## 4. CSS 变量

```css
:root {
  --grid-columns: 4;            /* JS 断点更新 */
  --grid-gap: 2px;

  /* 默认 Medium */
  --slot-h: 108px;
  --font-size-label: 14px;
  --line-height-label: 20px;
  --icon-size: 48px;
  --icon-padding-top: 14px;
  --label-padding-top: 6px;
  --label-max-lines: 2;
}

[data-density="small"] {
  --slot-h: 92px;
  --font-size-label: 12px;
  --line-height-label: 16px;
  --icon-size: 40px;
  --icon-padding-top: 12px;
  --label-padding-top: 4px;
}

[data-density="large"] {
  --slot-h: 124px;
  --font-size-label: 16px;
  --line-height-label: 24px;
  --icon-size: 56px;
  --icon-padding-top: 10px;
  --label-padding-top: 6px;
}
```

Grid 容器：

```css
.grid-container {
  display: grid;
  grid-template-columns: repeat(var(--grid-columns), 1fr);
  grid-auto-rows: var(--slot-h);
  gap: var(--grid-gap);
}
```

---

## 5. Slot 内部布局

Flexbox 纵向排列，结构固定：

```
┌──────────────────────────┐
│    icon-padding-top      │
│  ┌────────────────────┐  │
│  │     Icon Area      │  │  固定尺寸，水平居中
│  └────────────────────┘  │
│    label-padding-top     │
│  ┌────────────────────┐  │
│  │    Label Area      │  │  最多 2 行，置顶对齐，水平居中
│  │  (max 2 lines)     │  │  overflow: hidden + -webkit-line-clamp: 2
│  └────────────────────┘  │
│    剩余空间自然下沉       │
└──────────────────────────┘
```

关键规则：
- 文字**置顶对齐**，保证跨 Slot 的图标行形成整齐的水平线
- 文字超出 2 行用 ellipsis 截断
- 所有视觉间距都在 Slot 内部通过 padding 实现

---

## 6. Widget 尺寸

Widget 用 `column-span × row-span` 定义，不要用绝对像素：

| Widget 类型     | column-span | row-span |
|----------------|-------------|----------|
| 标准图标        | 1           | 1        |
| 横向小部件      | 2           | 1        |
| 横向大部件      | 4           | 1        |
| 方形部件        | 2           | 2        |
| 纵向部件        | 2           | 3        |

CSS 实现：

```css
.widget-2x1 { grid-column: span 2; grid-row: span 1; }
.widget-4x1 { grid-column: span 4; grid-row: span 1; }
.widget-2x2 { grid-column: span 2; grid-row: span 2; }
.widget-2x3 { grid-column: span 2; grid-row: span 3; }
```

布局引擎内部实际像素：

```
widget_width  = column-span × slot_width  + (column-span - 1) × gap
widget_height = row-span    × slot_height + (row-span - 1)    × gap
```

### Widget 内容安全区

Widget 内部内容必须预留**各方向至少 12px** 的内边距。Widget 开发者应使用百分比或 flex 布局，不要假设固定宽度。

### Widget 字号规则

- 正文：跟随全局字号档位（12/14/16px）
- 标题：全局字号 + 2px（即 14/16/18px），不超过此范围
- 辅助文字：全局字号 - 2px（即 10/12/14px），Small 档下不得小于 10px

---

## 7. Grid Gap 为什么是 2px 而不是 0

Gap = 0 时 @dnd-kit 的碰撞检测（closestCenter / rectIntersection）过于敏感，相邻 Slot 的 drop target 无缝拼接，拖拽容易误触相邻位置。

2px 提供最小碰撞缓冲，视觉上几乎不可见。如果某些密度下 2px gap 可察觉，用 `box-shadow` 或 `outline` 做视觉补偿。

---

## 8. 触控要求

所有可交互元素的触控热区 ≥ 44×44px（Slot 本身在所有档位下都满足，最小 92px）。Widget 内部的按钮/链接也必须满足此要求。

---

## 9. 实现检查清单

- [ ] Grid 容器使用 `repeat(var(--grid-columns), 1fr)`，Slot 宽度不写死
- [ ] `--grid-columns` 由 JS + `ResizeObserver` 按容器宽度断点更新
- [ ] Slot 高度通过 `data-density` 属性在三档间切换
- [ ] Slot 内部 Flexbox 纵向：图标在上、文字在下、文字置顶对齐
- [ ] 文字限制 2 行 + ellipsis
- [ ] Grid gap = 2px
- [ ] Widget 用 `grid-column: span N; grid-row: span M` 实现
- [ ] Widget 内部预留 ≥ 12px 安全区
- [ ] Widget 字号跟随全局档位
- [ ] 触控热区 ≥ 44×44px