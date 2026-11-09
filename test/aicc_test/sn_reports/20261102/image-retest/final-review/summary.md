# 六个目标单元的合并内容验收

两个模型所有目标调用均为 TaskMgr `Terminal/Succeeded`，输出均为可追踪 NamedObject，最终全部下载并检查为有效 PNG。结果宽高均为 1254×1254（本轮未显式固定 size）；两次图编辑与各自已有前置图进行人工比较。

| 模型 | API | 任务与下载 | 内容结果 | 预期与实际 |
|---|---|---|---|---|
| Flare | txt2img | 通过 | 通过 | 预期蓝色方形含 4827；实际蓝色方形、白色 4827 清晰完整 |
| Flare | img2img | 通过 | 通过 | 预期保留构图并增加傍晚暖光；实际保留方框和 4827 排布，整体变为橙色暖光 |
| Flare | inpaint | 通过 | 未通过 | 中央绿叶已生成；要求保留的蓝色区域变为透明，没有保留完整蓝色画布 |
| Sunburst | txt2img | 通过 | 通过 | 预期蓝色方形含 4827；实际蓝底和完整白色 4827 |
| Sunburst | img2img | 通过 | 通过 | 保留 4827 与构图，左上角增加橙黄暖光 |
| Sunburst | inpaint | 通过 | 未通过 | 中央绿叶已生成；蓝色画布变为透明区域 |

局部编辑请求明确要求蓝色画布和中央叶子、保留 mask 外区域。本轮不做像素级一致断言；两项失败是整体背景缺失，不是轻微色差。PNG 解码显示 alpha_min=0、alpha_max=254，没有 alpha=255 的完全不透明像素。图像查看时黑色背景是透明区域的显示效果，不能当成蓝背景已保留。

因此最终计数为：调用/结构/下载 6/6 通过；完整内容验收 4 通过、2 未通过。没有把 runner 的 review 自动改写成 passed，原始运行报告保留。

| Case 后缀 | Task |
|---|---|
| `gpt-image-2.5-sunburst.image.img2img` | `t-9f30ef9f12dd4d5da1ec757491997cd7` |
| `gpt-image-2.5-sunburst.image.txt2img` | `t-914dc369c2c64943828f47b0fba33253` |
| `gpt-image-2.5-sunburst.image.inpaint` | `t-869fe30b69164969b10c3ca9822db84a` |
| `gpt-image-2.5-flare.image.txt2img` | `t-2f3cb83a02f94101ab24b80195284656` |
| `gpt-image-2.5-flare.image.inpaint` | `t-4af88487b52e4ade89084e0e0983c20b` |
| `gpt-image-2.5-flare.image.img2img` | `t-b806e60d37a34d0d81738e70e14d3ccb` |

本地证据：初次运行报告目录的 `artifacts/` 和 `/tmp/sn-image-readback/`；均不提交 Git。叶子/背景差异仅记录本轮结果，未通过修改实现或放宽用例取得通过。既有 21 项 OCR/caption 质量结果保持不变。
