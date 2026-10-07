# aiws v2 — 程序宿主接口

许愿格程序是一个 ES module：`program/main.js`，默认导出 `async function main(aiws)`。它读取本次运行的快照，写出**候选结果**；它不直接修改工作区。只使用 Deno 内置 API 与 `aiws`，不引入第三方依赖。

调试：`deno run -A lib/run.js`（打印结果摘要；完整输出在 `output/.aiws/results.json`）。宿主的 `run_program` 和以后的“只重跑程序”用同一个 `lib/run.js` 运行同一份程序（权限更严：只能读运行目录、只能写 `output/`）。

## 读取

```js
const sales = aiws.input('sales')        // 按输入名取得输入（输入名见 context/inputs.json 与输出约定）
sales.kind                               // 'table' | 'richtext' | 'record' | 'asset' | 'folder' | 'annotation'
sales.fields()                           // [{ name, type, options?, id, visible }]，按表格字段顺序
sales.rows()                             // [{ 字段名: 值, _id }]；视图输入按视图的筛选与排序
sales.rows({ fields: ['月份', '销售额'], filter: { 状态: '已结算' }, sort: '-销售额', limit: 10 })
sales.rows({ filter: (r) => r['销售额'] > 0 })
aiws.input('rules').markdown()           // 富文本 → Markdown
aiws.input('profile').props()            // 记录 → { 属性名: 值 }
aiws.input('logo').bytes()               // 资产 → Uint8Array（.text() 取文本）
aiws.input('folder').members()           // 文件夹成员（每个成员同样有 rows/markdown/…）
aiws.inputs()                            // 全部输入名
aiws.request                             // 固定参数：{ today, timezone, currency, … }；日期口径以它为准
aiws.resolve('订单') / aiws.find('销售') / aiws.outline()   // 只返回元数据；要读内容必须是输入
```

值的形式：数值是 number，单选是选项名称（字符串），多选是名称数组，日期是 `YYYY-MM-DD` 字符串，空值不出现在行对象里（`r['x'] === undefined`）。

## 写结果（只写候选）

```js
// 表格：key 是逻辑键（一列或多列），重跑时据此保持记录身份；fields 可声明类型与列顺序
aiws.result.table('monthly', rows, { key: ['月份'], fields: { 月份: 'date', 销售额: 'decimal', 订单数: 'number' } })
// 派生列：写到一个表格输入上，按该输入行的 _id 给值；只会写本许愿格拥有的列
aiws.result.columns('差值', 'salary', Object.fromEntries(rows.map((r) => [r._id, { 差值: r['工资'] - avg }])), { fields: { 差值: 'decimal' } })
aiws.result.record('kpi', { 销售额合计: 123.4, 订单数: 12 })
aiws.result.text('commentary', markdown)            // 由程序按模板生成的文字（只重跑程序时也会更新）
aiws.result.file('chart', 'output/chart.svg', 'image/svg+xml')
aiws.result.html('dashboard', { html, css, js, bindings: { orders: 'input:sales', summary: 'result:monthly' } })
```

字段类型：`text | number | decimal | boolean | date | datetime | select | multi_select`；不声明时按值推断。`decimal` 按值所需的小数位保存（最多 6 位，不会四舍五入到更少）；要固定小数位时写成数组形式 `fields: [{ name: '毛利率', type: 'decimal', scale: 4 }]`，此时程序自己负责舍入。表格按 `rows` 的顺序显示（不超过 1000 行时；用户在 Block 上自己排序后以用户为准），需要排序就在程序里排好。一个结果名只能写一次；结果名必须是输出约定中的名称。

## 数字、检查与逐项判断

```js
aiws.facts({ total: 3301.5, growth: 0.091 })        // 文字中引用的数字必须来自 facts 或结果表
aiws.check('total', Math.abs(a - b) < 0.01, { a, b }) // 报告分析约定的程序型检查：id、是否通过、说明
const tags = await aiws.llm.map(texts, '判断这条客户反馈的情感', { type: 'string', enum: ['正面', '中性', '负面'] })
const amounts = await aiws.llm.map(notes, '从备注中抽取金额（元），没有则为 null', { type: ['number', 'null'] })
```

`llm.map` 逐项调用模型并按 schema 校验，结果按（模型、指令、输入项）缓存，只重跑程序时只对新增或变化的项调用模型；失败项会让调用抛错（`{ allowErrors: true }` 时以 `{ error }` 返回）。用它做逐项语义判断，不要让模型通读全表。

## 例子

```js
export default async function main(aiws) {
  const rows = aiws.input('sales').rows({ fields: ['月份', '客户', '销售额', '状态'] })
  const settled = rows.filter((r) => r['状态'] !== '未结算')
  const sum = (xs) => Math.round(xs.reduce((s, x) => s + (x ?? 0), 0) * 100) / 100
  const byMonth = new Map()
  for (const r of settled) byMonth.set(r['月份'], [...(byMonth.get(r['月份']) ?? []), r])
  const monthly = [...byMonth].sort(([a], [b]) => a.localeCompare(b))
    .map(([月份, items]) => ({ 月份, 销售额: sum(items.map((r) => r['销售额'])), 订单数: items.length }))
  aiws.result.table('monthly', monthly, { key: ['月份'], fields: { 月份: 'date', 销售额: 'decimal', 订单数: 'number' } })
  const total = sum(settled.map((r) => r['销售额']))
  aiws.facts({ total, months: monthly.length })
  aiws.check('total', Math.abs(sum(monthly.map((m) => m['销售额'])) - total) < 0.01)
  aiws.check('months', ['2026-07', '2026-08', '2026-09'].every((m) => monthly.some((r) => r['月份'].startsWith(m))))
}
```
