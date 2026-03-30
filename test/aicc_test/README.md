# aicc_test

最小的 DV 环境 AICC smoke 测试。

它会做三件事：

1. 用 `buckyos/node` 初始化 `AppClient`
2. 通过 `verify-hub.login_by_jwt` 换取可用 session token
3. 调用一次 `/kapi/aicc` 的 `complete`，并把返回结果打印出来

如果 AICC 首次返回 `running`，脚本会继续去 `task-manager` 查找对应的 AICC 任务并轮询，尽量把最终文本结果也打印出来。

## 安装

```bash
cd test/aicc_test
pnpm install
```

## 运行

```bash
cd test/aicc_test
pnpm run smoke
```

## 可选环境变量

```bash
BUCKYOS_SYSTEM_CONFIG_URL=http://127.0.0.1:3200/kapi/system_config
BUCKYOS_VERIFY_HUB_URL=http://127.0.0.1:3300/kapi/verify-hub
BUCKYOS_TASK_MANAGER_URL=http://127.0.0.1:3380/kapi/task-manager
AICC_URL=http://127.0.0.1:4040/kapi/aicc
BUCKYOS_TEST_APP_ID=control-panel
BUCKYOS_TEST_USER_ID=devtest
AICC_MODEL_ALIAS=llm.default
AICC_TEST_INPUT=请用一句话介绍 BuckyOS
AICC_WAIT_TIMEOUT_MS=90000
```

说明：

- 默认沿用 `test/app_installer_test` 的登录路径和 `control-panel` appid。
- 如果本机有 `/opt/buckyos/etc/.buckycli/user_private_key.pem`，脚本会优先自签登录 JWT；否则回退到 `buckyos.login()`。
