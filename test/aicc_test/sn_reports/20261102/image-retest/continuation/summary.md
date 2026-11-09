# Flare img2img 定向补全

- 本轮唯一新增模型调用：1；总范围累计：8/8（6 个目标 + 2 个前置生成）。无自动重试。
- Case：`t2.sn-ai-provider.sn-us-full-20261030-t2.gpt-image-2.5-flare.image.img2img`。
- 原始 run/idempotency namespace：`aicc-2026-11-02T04-09-58-748Z-ca7171c7`，未改原始请求意图或验收条件。
- 使用本轮已有前置图 `cyfile:752ac8fb2fdd495422818e01e2b87d1c5b3f46266e5f7fb1bdb70e6291e9104a`，没有再次生成输入。
- Task：`t-b806e60d37a34d0d81738e70e14d3ccb`；TaskMgr `Terminal/Succeeded`（收尾核验）。
- 结果：canonical image 结构及 NamedObject 引用检查通过，NDN 下载成功；人工图像内容复核通过。
- NDN 入口：`http://localhost:3180` 临时路由；读取与 RPC 的 `http://127.0.0.1:3180` 使用不同连接池。
- 配置：SN US_FULL、精确 Flare、最大新增调用 1、超时 900000ms、无重试，沿用总估算预算 $4。
- 复用现有 Gateway、Provider credential/settings transaction、canonical 请求和制品校验 helper。补全启动检查曾因本地脚本使用 operation 名 `images.edit` 而被 RPC 拒绝；无 task、无 Provider 调用，不计入 8 次真实调用。
- 临时 Provider 配置恢复：`restored`；临时 NDN 路由已移除。
- 此输出删除：HTTP 405，对象保留。与首次运行 7 个对象的删除失败合计 8 个残留，未改写 NDN 删除实现。

制品本地复核文件：`/tmp/sn-image-readback/t-b806e60d37a34d0d81738e70e14d3ccb-0.png`；不提交图片及原始响应。
