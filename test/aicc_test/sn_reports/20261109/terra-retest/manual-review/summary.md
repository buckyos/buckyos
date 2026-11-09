# Terra 升级后四个历史失败单元的最终验收

四个目标和一个前置生成任务全部为 TaskMgr `Terminal/Succeeded`。computer-use 由 runner 验收返回动作；其余三个原始 runner 状态为 review，下表在保持原始提示和验收条件下进行人工内容复核，原始报告不改写。

| API | 预期 | 实际 | 最终结果 |
|---|---|---|---|
| agent.computer_use | 返回唯一蓝色按钮内的一次左键点击，并要求下一次观察 | `left_click(640,360)`；目标矩形 `[480,300]–[800,420]`；`requires_next_observation=true` | 通过 |
| vision.ocr | 完整标记 `BUCKYOS-DV-4827` | `BUCKYOS-DV-4827` | 通过 |
| image.txt2img | 蓝色方形包含数字 4827 | 白边蓝色方块，完整白色 4827 清晰可见 | 通过 |
| image.img2img | 保留前置图构图并加入傍晚暖光 | 保留蓝色方块、白边及 4827 排布，边框和数字呈暖色，左上光照变暖 | 通过 |

两个目标图片与同模型生成的前置图均成功通过原生 NDN 经 Gateway 下载并保存为有效 PNG。图编辑实际发起且独立完成，未再被前置生成阻断。只检查返回的 computer action，没有执行真实桌面点击。

## 任务证据

| Task | 用例 | 状态 | Trace |
|---|---|---|---|
| `t-9d63f699973c43e4888e6d14938d0a32` | image.img2img | Terminal/Succeeded | `aicc-idem-45dbba5ff8f345778a35f39756cc7d61` |
| `t-4e126d30dbc1464b9ccb1f2ae2f5c83f` | vision.ocr | Terminal/Succeeded | `aicc-idem-4b07df97242fc0246293223920f9b632` |
| `t-d69b806bf2e74128b833c8dc1d7b7aad` | image.txt2img | Terminal/Succeeded | `aicc-idem-90f322bf751c2b416e05c813b1621c0e` |
| `t-7fcdd755f77b45e2bdd7d6cbccd689d2` | 图编辑前置 txt2img | Terminal/Succeeded | `aicc-idem-ae3e94c51dd41a1983704d0f1ed5a2eb` |
| `t-156d8fe29f5c4586a87a32274e64bd19` | agent.computer_use | Terminal/Succeeded | `aicc-idem-4ce7bf08408afcd121256a029e5802b7` |

## 图片制品证据

| API | NamedObject | 字节数 | SHA-256 |
|---|---|---:|---|
| image.img2img | `cyfile:cff389d2c0c81a3f1380a773b3b7e35827f8f58e3639d3c690a9bb1b6c2a175d` | 1701175 | `c02f199abb5a28cff3e8eb4f42c6843eeb59ef8a474e77c6edcac8dfad23cda7` |
| image.txt2img | `cyfile:02ef76c1a4981325792c8166e032235dc38b0c24be14cc6b8c60b3e61e292b85` | 1120220 | `80822ab473cd42c15d395f72885ee4ff44d5df832a2077eb57aca90aa0edc1cc` |

本地图片在原始 run 的 `artifacts/` 目录；人工比较了 img2img 输出和其同模型生成的前置图，没有再调用 Judge 或其他模型。原始响应和图片只保留本地，不提交 Git。
