# app_installer_test

独立工程示例，直接通过 `package.json` 里的 GitHub 依赖安装 `buckyos`：

```json
{
  "dependencies": {
    "buckyos": "github:buckyos/buckyos-websdk"
  }
}
```

运行：

```bash
pnpm install
pnpm run demo
```

可选环境变量：

```bash
BUCKYOS_TEST_APP_ID=buckycli
BUCKYOS_SYSTEM_CONFIG_URL=http://127.0.0.1:3200/kapi/system_config
```

示例代码从发布包的 Node 入口导入：

```js
import { buckyos, RuntimeType, parseSessionTokenClaims } from 'buckyos/node'
```

注意：

当前只有在 GitHub 上的 `buckyos/buckyos-websdk` 已经包含 `./node` 条件导出和 AppClient 实现时，这个示例才能直接跑通。
如果仓库还没推送到包含这些改动的提交，`pnpm install` 虽然会成功，但 `pnpm run demo` 会因为找不到 `buckyos/node` 而失败。
