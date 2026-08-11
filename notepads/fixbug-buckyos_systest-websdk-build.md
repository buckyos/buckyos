## 为什么会有这个 Bug

`sys_test` 前端的 `websdk` 依赖仍然指向本地路径 `file:../../../../../buckyos-websdk`。在源码树本地开发时这个依赖可以解析，但一旦进入构建/拷贝后的环境，这个相对路径就不再成立，导致前端依赖安装阶段拿不到 `buckyos-websdk`，进而使 `sys_test` 的构建链条失效。

同时，`sys_test` 的 `deno.json` 构建任务还在使用 `npm install`，而这一侧本来就使用 `pnpm-lock.yaml` 管理依赖。修正为 GitHub 分支依赖后，继续使用 `npm` 会让构建链与锁文件体系不一致。

## 我是如何修复的

把 `src/apps/sys_test/web/package.json` 中的 `buckyos` 依赖改成 `github:buckyos/buckyos-websdk#beta2.2`，让构建时直接从 GitHub 的 `beta2.2` 分支获取依赖，不再依赖本地目录布局。

同时把 `src/apps/sys_test/deno.json` 和 `src/rootfs/bin/buckyos_systest/deno.json` 中的 `build` 任务改成 `cd web && pnpm install --frozen-lockfile && pnpm run build`，让构建链和现有 `pnpm-lock.yaml` 保持一致。
