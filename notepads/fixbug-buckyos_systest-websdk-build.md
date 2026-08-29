## 为什么会有这个 Bug

`sys_test` 前端的 `websdk` 依赖仍然指向本地路径 `file:../../../../../buckyos-websdk`。在源码树本地开发时这个依赖可以解析，但一旦进入构建/拷贝后的环境，这个相对路径就不再成立，导致前端依赖安装阶段拿不到 `buckyos-websdk`，进而使 `sys_test` 的构建链条失效。

同时，`sys_test` 的构建任务还在使用 `npm install`。BuckyOS 内部应用不提交 lockfile，并始终跟随最新 WebSDK，因此这里需要统一改用 pnpm 安装并主动更新 GitHub 依赖。

## 我是如何修复的

把 `src/apps/sys_test/web/package.json` 中的 `buckyos` 依赖改成 `git+https://github.com/buckyos/buckyos-websdk#main`，让构建时直接从 GitHub 的 `main` 分支获取依赖，不再依赖本地目录布局。

同时把构建任务改为先执行 `pnpm install` 和 `pnpm update buckyos`，再构建 web workspace，确保每次构建都使用 WebSDK `main` 的最新版本。
