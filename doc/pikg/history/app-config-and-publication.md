# App 配置与正式发布：历史设计摘要

> 历史参考。合并旧权限配置长稿中的目标语义，以及旧 App 生命周期稿中未完成的签名、托管和发布设计。以下是当时的讨论与目标，不是当前可执行操作或 schema。

## 配置设计曾希望解决的问题

旧稿围绕 self-host 镜像提出：标准 AppData 与缓存分开、null mount 自动映射 AppData、受限模板变量展开、Storage Grant、mount 与 permission/action 联动、append-only、secret/env 注入、GPU/device typed grants、RDB 适配、healthcheck/run-as。

这些目标的实现程度应逐项核对当前源码和目标环境；旧稿中 2026-08-25 的差距表不能沿用为今天的事实。原稿的完整 self-host 场景矩阵、配置草案和待实现列表通过下方 Git 历史查看，不再散放在当前使用路径。

## 正式发布曾提出的流程

当时建议对已测试候选重验，只签相同 AppDoc 内容，先托管并确认可下载，再更新 BNS，最后回读；签名前后逻辑内容身份应保持不变，整包字节 digest 可能改变。Owner 签名、内容上传和 BNS 发布是三个独立权限边界。

该稿曾以 `APPDOC.jwt` 和 UnsignedLocal/SignedUnpublished/SignedAvailable/Published 描述未来状态。当前安装协议定义 detached envelope，reader 接收 JWT 形式的 candidate；二者尚未形成完成验签和 BNS 回读的统一公开发布闭环，不能把旧状态机当作已落地接口。

## 当前替代资料

- [App 包配置](../../sdk/app-package-config.md)：当前能解析的声明形状和运行边界。
- [App 生命周期](../../app_service/BuckyOS%20App基础设施职责边界.md)：当前职责及 Repo candidate 边界。
- [PIKG 格式](../format.md)：reader 当前支持和签名校验限制。
- [App 模块](https://github.com/buckyos/buckyos-websdk/blob/main/doc/modules/app.md)：当前 app.publish 的准确输入输出。

## 原始稿追溯

- [notepads/service_config_tips + permissions 实用配置指南.md](https://github.com/buckyos/buckyos/blob/b50d757f71cd0ad57bef6292c6df102197e8bd2d/notepads/service_config_tips%20%2B%20permissions%20%E5%AE%9E%E7%94%A8%E9%85%8D%E7%BD%AE%E6%8C%87%E5%8D%97.md)
- [doc/app_service/BuckyOS App基础设施职责边界.md](https://github.com/buckyos/buckyos/blob/aea412165c5dbe4eb2484f278430b42d8e068e8b/doc/app_service/BuckyOS%20App%E5%9F%BA%E7%A1%80%E8%AE%BE%E6%96%BD%E8%81%8C%E8%B4%A3%E8%BE%B9%E7%95%8C.md)
