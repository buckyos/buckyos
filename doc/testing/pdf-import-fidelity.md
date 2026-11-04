# PDF 导入保真验证

默认输入是仓库自带的四页合成 PDF：`src/frame/aiworkspace/fixtures/pdf/synthetic-pdf-import.pdf`。它从零构造，不读取用户文档，不复用用户文档的文字、页面、图片、字体、元数据或资源。生成器、清单和 PDF 可一起审查；报告也只包含合成内容。

## 合成样本

| 页 | 构造 | 验证目标 |
| --- | --- | --- |
| 1 | 蓝色平铺图案、36 个独立文字块和 12 个 q/cm/path/Q 变换路径块 | PDF.js 提取流和优化渲染流是否对齐，底图是否完整 |
| 2 | 自绘网格和折线，256×192、1-bit CCITT Group4 压缩 | WASM 解码资源、原图与整页图片模式的可见内容 |
| 3 | 自绘 Type3 轮廓字形映射 U+E123，与普通文字叠放 | 不可编辑字形能否留在底图，是否被误当作 converted 过滤 |
| 4 | 普通、红色粗体、蓝色斜体同一行 | 局部样式的当前降级行为 |

标准字体仅 Times-Roman/Bold/Italic；自绘 Type3 字形不是外部嵌入字体。PDF 没有附件、链接、脚本或用户文档元数据。`fidelity.manifest.json` 记录 SHA256 和 CCITT 图像的生成规则。生成器使用已有的 `src/write.ts` 和一个可审查的合成 CCITT 数据常量，不需要原件或新增 npm/Python 依赖。

## 快速运行

在 Linux 的 `src/frame/desktop`，使用现有依赖、Playwright Chromium、Poppler `pdftoppm` 和预建 AIWorkspace/NFS。入口不会安装依赖、编译服务或部署。

```bash
export AIWS_BIN=/absolute/prebuilt/aiworkspace
export NFS_BIN=/absolute/prebuilt/nfs_server
pnpm test:pdf --scratch-dir /absolute/project/.work/pdf-tmp
```

不传 PDF 参数时始终使用合成样本，即使环境已有 `AIWS_PDF_FILE`。默认四页、editable/raster 两种模式、零重试、单 worker，并执行第 2 页的 WASM A/B 控制。`--project editable`/`raster` 缩小范围；`--control-only` 只跑解码控制。使用已有浏览器时设置 `PLAYWRIGHT_CHROMIUM_EXECUTABLE_PATH`。

显式传入其他 PDF 才会测试调用者文件。它会上传到隔离测试后台，并将文字清单、截图和 trace 写入报告；不得将私有输入或相应报告提交到仓库/Issue。示例：

```bash
pnpm test:pdf /absolute/explicit-input.pdf --pages 1,2 --output /absolute/project/.work/pdf-result
```

临时目录及输出必须在项目 `.work`。深路径需短 `--scratch-dir` 或 `TMPDIR`，避免 Chromium socket 路径过长。每次使用新的 output，Playwright 会清理该 output 下的 browser/artifacts。

没有 Poppler 时可通过 `--reference-dir DIR` 提供独立且人工核对过的 `page-N.png`。不要仅用相同 PDF.js 生成唯一参考图：它可能与导入功能同时漏图。`--decoder-control --decoder-page N` 可指定其他图片页的解码对照；这个控制只调整测试助手的资源地址，不改生产代码。

## 输出与断言

- `browser/html/index.html`、`browser/results.json`：HTML/CI 结果与失败 trace。
- 每例 `metrics.json`：告警、解码日志、指令数、对象数、可编辑字符覆盖率、文字颜色像素覆盖率、样式变化和被过滤的私用字形指令。
- 每例 `reference.png`、`imported.png`、`difference.png`：独立原图、实际画布和热图。
- `run.json`：输入、页号、耗时和退出码。

通过真实 UI 导入，访问真实独立 AIWorkspace/NFS 后台，检查持久化对象和刷新后的数量。Desktop 外壳使用现有 mock runtime、后台使用静态测试身份；不证明 Zone SSO/Gateway/生产部署，不调用外部模型。

断言没有页面异常、没有 pdf_raster_failed、解码器正常加载，底图颜色/纹理、图片墨迹及文字颜色得到保留；字符被排除出可编辑文本时必须有回退，相关原字形不能同时被标为 converted 过滤。raster 模式的可编辑字符覆盖率为 null。

底图 ROI 为 x=.01,y=.92,w=.10,h=.06，颜色均值差小于 .02，纹理标准差比大于 .4；图片 ROI 来自源图片几何，墨迹覆盖率比大于 .5。源文字颜色按 RGB 每通道容差 32 统计，覆盖率超过 .00005 时，导入后的覆盖率比须大于 .5。整页 MAE/热图作诊断，避免字体抗锯齿造成假失败。任意新文档仍需核对 ROI；字符多重集不证明顺序、断行或公式语义。现有 editable 采用段落主样式、通用字体，这些降级记录为指标。

## 重建及窄检查

在 `src/frame/aiworkspace`：

```bash
deno run --allow-write fixtures/pdf/src/fidelity.ts
deno test -A import-pdf/fidelity_test.ts
```

第一条应逐字节复现已提交 PDF；改变样本时同步更新清单 SHA256。第二条检查确定性、资源范围、四页内容、私用字形和混合样式触发条件。浏览器保真回归应保持红色，直到导入器实际修复；不能仅移除数量检查，skip 索引也必须与渲染流一致。

当前变更只涉及测试和文档，无 RPC、存储、应用工具、命令手册或跨仓库契约改动。

## 已验证的合成基线

源码 ec809dc、PDF.js 6.4.299：4 页两种导入模式及解码控制共 10 项，约 129 秒；5 失败、4 通过、1 跳过。通过项为 raster 页 1/3/4 和解码控制；跳过项为 raster project 中重复的解码控制。

- 页 1：原始 240 条、渲染 204 条，pdf_raster_failed，保存底图 0。
- 页 2：默认配置的 CCITT 解码失败，editable/raster 均漏图；WASM A/B 控制恢复。
- 页 3：U+E123 不在可编辑文本中，对应原始绘制指令还被标记 converted。
- 页 4：源红色文字像素覆盖率非零，导入后为零，颜色保留断言失败。

两项夹具检查通过（确定性/资源范围、四类触发内容）；Deno lint、TypeScript Node/test 配置、目标 ESLint、runner 语法检查通过。默认入口还在 AIWS_PDF_FILE 指向不存在的哨兵文件时运行成功，run.json 记录的输入仍是仓库合成样本。
