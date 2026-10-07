// deno test --allow-read --allow-write --allow-run --allow-env aiws_test.js
// The program host against a hand-made snapshot directory, run exactly as the service runs it.

import { assert, assertEquals } from './test_assert.js'

const here = new URL('.', import.meta.url).pathname

async function workdir(program, extra = {}) {
  const dir = await Deno.makeTempDir({ prefix: 'aiws-' })
  const w = (p, s) => {
    const full = `${dir}/${p}`
    Deno.mkdirSync(full.slice(0, full.lastIndexOf('/')), { recursive: true })
    Deno.writeTextFileSync(full, typeof s === 'string' ? s : JSON.stringify(s))
  }
  w('lib/aiws.js', Deno.readTextFileSync(`${here}/aiws.js`))
  w('lib/run.js', Deno.readTextFileSync(`${here}/run.js`))
  w('program/main.js', program)
  w('request.json', { today: '2026-10-06', currency: 'CNY' })
  w('context/inputs.json', {
    sales: { entity_id: 'orders', type_id: 'buckyos.table-source', dir: 'context/entities/orders', label: '订单', selector: { kind: 'table_view', cell_id: 'v' } },
    rules: { entity_id: 'rules', type_id: 'buckyos.richtext', dir: 'context/entities/rules', label: '口径' },
    ...extra,
  })
  w('context/entities/orders/schema.json', { fields: [
    { field_id: 'm', name: '月份', type: 'date' }, { field_id: 'c', name: '客户', type: 'text' },
    { field_id: 'a', name: '销售额', type: 'decimal' }, { field_id: 's', name: '状态', type: 'select', options: ['已结算', '未结算'] }] })
  w('context/entities/orders/rows.jsonl', [
    { id: 'r1', v: { m: '2026-07-01', c: '甲', a: 100.5, s: '已结算' } },
    { id: 'r2', v: { m: '2026-07-01', c: '乙', a: 50, s: '未结算' } },
    { id: 'r3', v: { m: '2026-08-01', c: '甲', a: 200, s: '已结算' } },
  ].map((r) => JSON.stringify(r)).join('\n') + '\n')
  w('context/entities/rules/content.md', '# 口径\n\n排除未结算。\n')
  w('context/data-tree.json', [{ handle: '@T1', entity_id: 'orders', title: '订单', path: '/data/订单', parent: '@F9' }])
  w('context/block-tree.json', [])
  Deno.mkdirSync(`${dir}/output`, { recursive: true })
  return dir
}

async function run(dir, env = {}) {
  const cmd = new Deno.Command(Deno.execPath(), {
    args: ['run', '--no-prompt', '--quiet', `--allow-read=${dir}`, `--allow-write=${dir}/output`, '--allow-net', '--allow-env', 'lib/run.js'],
    cwd: dir, env, stdout: 'piped', stderr: 'piped',
  })
  const out = await cmd.output()
  const results = JSON.parse(Deno.readTextFileSync(`${dir}/output/.aiws/results.json`))
  return { code: out.code, stdout: new TextDecoder().decode(out.stdout), stderr: new TextDecoder().decode(out.stderr), results }
}

Deno.test('reads inputs by name and writes results, facts and checks', async () => {
  const dir = await workdir(`
export default async function main(aiws) {
  const rows = aiws.input('sales').rows({ fields: ['月份', '销售额', '状态'] })
  const settled = rows.filter((r) => r['状态'] !== '未结算')
  const by = new Map()
  for (const r of settled) by.set(r['月份'], (by.get(r['月份']) ?? 0) + r['销售额'])
  const monthly = [...by].map(([月份, 销售额]) => ({ 月份, 销售额 }))
  aiws.result.table('monthly', monthly, { key: ['月份'], fields: { 月份: 'date', 销售额: 'decimal' } })
  aiws.facts({ total: settled.reduce((s, r) => s + r['销售额'], 0) })
  aiws.check('total', monthly.reduce((s, r) => s + r['销售额'], 0) === 300.5, 'sum')
  aiws.result.text('note', '口径：' + aiws.input('rules').markdown().split('\\n')[2])
  aiws.result.columns('flag', 'sales', Object.fromEntries(rows.map((r) => [r._id, { 已结算: r['状态'] === '已结算' }])))
  if (aiws.request.today !== '2026-10-06') throw new Error('request')
}`)
  const { code, stdout, results } = await run(dir)
  assertEquals(code, 0, stdout)
  const t = results.results[0]
  assertEquals(t.fields.map((f) => f.name), ['月份', '销售额'])
  assertEquals(t.rows, [{ 月份: '2026-07-01', 销售额: 100.5 }, { 月份: '2026-08-01', 销售额: 200 }])
  assertEquals(results.facts.total, 300.5)
  assertEquals(results.checks, [{ id: 'total', passed: true, detail: 'sum' }])
  assertEquals(results.results[1].markdown, '口径：排除未结算。')
  assertEquals(results.results[2].values.r2, { 已结算: false })
  assert(stdout.includes('table monthly: 2 rows'))
})

Deno.test('errors are reported, not hidden', async () => {
  const dir = await workdir(`export default async function main(aiws) { aiws.input('nope') }`)
  const { code, results } = await run(dir)
  assertEquals(code, 1)
  assert(results.error.includes('no input named "nope"'))
  const dir2 = await workdir(`export default async function main(aiws) { aiws.input('sales').rows({ fields: ['不存在'] }) }`)
  assert((await run(dir2)).results.error.includes('has no field 不存在'))
  const dir3 = await workdir(`export default async function main(aiws) { aiws.result.text('a/b', 'x') }`)
  assert((await run(dir3)).results.error.includes('invalid result name'))
  const dir4 = await workdir(`export default async function main(aiws) { await aiws.llm.map([1], 'x') }`)
  assert((await run(dir4)).results.error.includes('only available inside a wish run'))
  // the program cannot write outside output/
  const dir5 = await workdir(`export default async function main(aiws) { Deno.writeTextFileSync('program/x.js', 'x') }`)
  const err = (await run(dir5)).results.error
  assert(/NotCapable|PermissionDenied|write access/.test(err), err)
})

Deno.test('llm.map calls the host and external fetches are recorded', async () => {
  const seen = []
  const server = Deno.serve({ port: 0, onListen() {} }, async (req) => {
    const url = new URL(req.url)
    if (url.pathname.endsWith('/llm_map')) {
      const body = await req.json()
      seen.push({ token: req.headers.get('x-aiws-token'), body })
      return Response.json({ results: body.items.map((i) => ({ value: i > 1 ? 'big' : 'small' })), calls: 1, cached: 0 })
    }
    return new Response('external')
  })
  const base = `http://127.0.0.1:${server.addr.port}`
  const dir = await workdir(`
export default async function main(aiws) {
  const labels = await aiws.llm.map([1, 2], '大小', { type: 'string', enum: ['big', 'small'] })
  aiws.facts({ labels })
  await (await fetch('${base}/other/data?x=1')).text()
}`)
  const { code, results, stderr } = await run(dir, { AIWS_HOST: `${base}/kapi/aiworkspace/wish-host/tok`, AIWS_TOKEN: 'secret' })
  await server.shutdown()
  assertEquals(code, 0, stderr)
  assertEquals(results.facts.labels, ['small', 'big'])
  assertEquals(seen[0].token, 'secret')
  assertEquals(results.llm_map.items, 2)
  assertEquals(results.external, [`${base}/other/data`])
})
