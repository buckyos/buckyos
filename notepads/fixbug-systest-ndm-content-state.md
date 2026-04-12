## 为什么会有这个 Bug

`sys_test` 里把上传前的 `InStore` 命中和 backend 查询返回的 `contentState` 当成了同一种状态来展示，但两者底层语义不同：

- 前者来自 `pickupAndImport()` 的 `lookupObject(qcid)` 命中，表示可以通过 QCID 直接复用已有内容，属于“秒传命中”语义。
- 后者之前调用的是 `isObjectStored(contentId)`，它检查的是 `contentId` 对应对象及其强依赖是否都已经在 store 中可递归取到，属于“fully stored”语义。

除此之外，`sys_test` backend 之前还会在 `contentId` 尚未 `fully stored` 时直接调用 structured store API 的 `add_chunk_by_same_as`。

当前这条 structured API 路径不会像另一条 HTTP store gateway 路径那样先校验 `chunk_list_id` 已存在且 sub-chunks 完整，因此可能提前写入 `qcid -> same_as(chunklist)` 映射。这样就会造成：

- 第二次上传前 `lookupObject(qcid)` 命中，UI 显示 `InStore`
- 但 backend 的严格 `isObjectStored(contentId)` 仍然是 `false`

这不是“严格检查错了”，而是 `same_as` 注册时机过早导致的假阳性秒传命中。

## 我是如何修复的

在 `src/apps/sys_test/main.ts` 和 `src/apps/sys_test/web/index.ts` 中做了两部分修复：

- 前端把上传前的 `materializationStatus/uploadStatus` 一起传给 backend，并继续在 UI 上单独展示 `InStore`。
- backend 将内容查询结果拆成两个字段：
  - `contentState`：表示 `contentId` 当前是否能查到。
    - chunk 内容走 `queryChunkState`
    - object/chunklist 内容走 `queryObjectById`
  - `contentStoredState`：继续保留严格的 `isObjectStored(contentId)` 结果，用来表示“递归 fully stored”
- backend 只有在 `contentStoredState.stored === true` 时才调用 `addChunkBySameAs`，避免继续写入“lookup 能命中，但内容并未 fully stored”的 `same_as` 映射。

这样一来：

- 返回 JSON 可以同时表达“秒传命中”和“内容是否 fully stored”，避免继续误读；
- 新生成的 `qcid -> same_as` 映射不会再早于 `content` 真正 fully stored。
