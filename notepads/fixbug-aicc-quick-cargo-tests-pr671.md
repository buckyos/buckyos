# PR #671 AICC Quick Cargo Tests 修复记录（2026-10-17）

## 问题原因

PR #671 的 OpenRouter 分支 rebase 到 `test/claude-retest-20261014` 后，Gemini adapter 同时注册了同步 `interactions.create` 和后台 `interactions.background` 两个 `llm` operation。`web_search_inventory_requires_model_adapter_and_discovery_support` 使用手工构造的 discovery fixture，未像实际 Gemini Provider metadata 一样指定 operation，导致 inventory 构建无法在两个 operation 之间唯一选择并失败。

## 修复方式

保持 inventory 的 fail-closed 歧义检查和产品实现不变，在该测试的 Gemini discovery fixture 中显式声明本用例验证的同步 `interactions.create`。测试仍然覆盖模型能力、adapter 能力和 discovery 禁用三者对 `web_search` 的交集，不再意外依赖 adapter 只有一个 operation。

未修改协议、共享类型、数据结构或第三方依赖。

## 验证

- `cargo test -p aicc provider::tests::web_search_inventory_requires_model_adapter_and_discovery_support -- --test-threads=1`
- `cargo test -p aicc -- --test-threads=1`
- `cargo fmt -p aicc -- --check`
- `git diff --check`
