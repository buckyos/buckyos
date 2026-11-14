// deno test --allow-read --allow-write --allow-run --allow-env aiws_test.js
// The program host against a hand-made snapshot directory, run exactly as an executor runs it (`lib/run.js`, with
// the stdio bridge when the test plays the executor), and the executor's side of the bridge (host.js).

import { assert, assertEquals } from './test_assert.js'
import { canonical, createBridgeHost, extractJson, sha256Hex, validate } from './host.js'

const here = new URL('.', import.meta.url)

async function workdir(program, extra = {}) {
  const dir = await Deno.makeTempDir({ prefix: 'aiws-许愿格 space #%+-' })
  const w = (p, s) => {
    const full = `${dir}/${p}`
    Deno.mkdirSync(full.slice(0, full.lastIndexOf('/')), { recursive: true })
    Deno.writeTextFileSync(full, typeof s === 'string' ? s : JSON.stringify(s))
  }
  w('lib/aiws.js', Deno.readTextFileSync(new URL('aiws.js', here)))
  w('lib/run.js', Deno.readTextFileSync(new URL('run.js', here)))
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
  Deno.mkdirSync(`${dir}/output`, { recursive: true })
  return dir
}

const DENO_ARGS = (dir) => ['run', '--no-prompt', '--quiet', '--no-remote', `--allow-read=${dir}`, `--allow-write=${dir}/output`, 'lib/run.js']

async function run(dir) {
  const cmd = new Deno.Command(Deno.execPath(), { args: DENO_ARGS(dir), cwd: dir, clearEnv: true, stdout: 'piped', stderr: 'piped' })
  const out = await cmd.output()
  const results = JSON.parse(Deno.readTextFileSync(`${dir}/output/.aiws/results.json`))
  return { code: out.code, stdout: new TextDecoder().decode(out.stdout), stderr: new TextDecoder().decode(out.stderr), results }
}

/** Run with `--bridge`, answering each bridge call with `answer(op, req)` the way an executor does. */
async function runBridged(dir, answer) {
  const child = new Deno.Command(Deno.execPath(), { args: [...DENO_ARGS(dir), '--bridge'], cwd: dir, clearEnv: true, stdin: 'piped', stdout: 'piped', stderr: 'piped' }).spawn()
  const writer = child.stdin.getWriter()
  const enc = new TextEncoder()
  const calls = []
  let stdout = ''
  const reading = (async () => {
    const dec = new TextDecoder()
    let buf = ''
    for await (const chunk of child.stdout) {
      buf += dec.decode(chunk, { stream: true })
      let i
      while ((i = buf.indexOf('\n')) >= 0) {
        const line = buf.slice(0, i)
        buf = buf.slice(i + 1)
        if (!line.startsWith('\x1eaiws ')) { stdout += line + '\n'; continue }
        const msg = JSON.parse(line.slice(6))
        calls.push(msg)
        const body = await answer(msg.op, msg.req)
        await writer.write(enc.encode(JSON.stringify({ id: msg.id, body }) + '\n'))
      }
    }
  })()
  const stderr = new Response(child.stderr).text()
  const status = await child.status
  await reading
  await writer.close().catch(() => {})
  const results = JSON.parse(Deno.readTextFileSync(`${dir}/output/.aiws/results.json`))
  return { code: status.code, stdout, stderr: await stderr, results, calls }
}

Deno.test('declares, reads declared inputs by name and writes results, facts and checks', async () => {
  const dir = await workdir(`
export default async function main(aiws) {
  aiws.declare({ inputs: { sales: ['月份', '销售额', '状态'], rules: [] } })
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
  if (aiws.input('sales').fields().length !== 3) throw new Error('only declared fields')
  if (Object.keys(aiws.input('sales').rows()[0]).sort().join() !== '_id,月份,状态,销售额') throw new Error('rows carry the declared fields')
  if ('resolve' in aiws || 'find' in aiws || 'outline' in aiws) throw new Error('no workspace tree at run time')
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
  assertEquals(results.declare, { inputs: { sales: ['月份', '销售额', '状态'], rules: [] } })
  assert(stdout.includes('table monthly: 2 rows'))
})

Deno.test('errors are reported, not hidden: undeclared inputs and fields throw', async () => {
  const undeclared = await workdir(`export default async function main(aiws) { aiws.input('sales') }`)
  assert((await run(undeclared)).results.error.includes('declare the inputs first'))
  const dir = await workdir(`export default async function main(aiws) { aiws.declare({ inputs: {} }); aiws.input('nope') }`)
  const r = await run(dir)
  assertEquals(r.code, 1)
  assert(r.results.error.includes('no input named "nope"'), r.results.error)
  const notListed = await workdir(`export default async function main(aiws) { aiws.declare({ inputs: { rules: [] } }); aiws.input('sales') }`)
  assert((await run(notListed)).results.error.includes('input sales is not declared'))
  const badField = await workdir(`export default async function main(aiws) { aiws.declare({ inputs: { sales: ['不存在'] } }) }`)
  assert((await run(badField)).results.error.includes('has no field "不存在"'))
  // reading a field of the table that was not declared throws with its name, never an empty value
  const readUndeclared = await workdir(`export default async function main(aiws) {
  aiws.declare({ inputs: { sales: ['月份'] } })
  const total = aiws.input('sales').rows().reduce((s, r) => s + r['销售额'], 0)
  aiws.facts({ total })
}`)
  const u = (await run(readUndeclared)).results.error
  assert(u.includes('field 销售额 is not declared'), u)
  const notDeclaredRows = await workdir(`export default async function main(aiws) { aiws.declare({ inputs: { sales: ['月份'] } }); aiws.input('sales').rows({ fields: ['客户'] }) }`)
  assert((await run(notDeclaredRows)).results.error.includes('has no declared field 客户'))
  const dir3 = await workdir(`export default async function main(aiws) { aiws.result.text('a/b', 'x') }`)
  assert((await run(dir3)).results.error.includes('invalid result name'))
  const dir4 = await workdir(`export default async function main(aiws) { aiws.declare({ inputs: {} }); await aiws.llm.map([1], 'x') }`)
  assert((await run(dir4)).results.error.includes('only available inside a wish run'))
  // the program cannot write outside output/
  const dir5 = await workdir(`export default async function main(aiws) { Deno.writeTextFileSync('program/x.js', 'x') }`)
  const err = (await run(dir5)).results.error
  assert(/NotCapable|PermissionDenied|write access/.test(err), err)
  // nor reach the network, nor read the environment
  const dir6 = await workdir(`export default async function main(aiws) { await fetch('https://example.com/') }`)
  assert(/NotCapable|PermissionDenied|net access/.test((await run(dir6)).results.error))
})

Deno.test('the bridge: sql, ai.complete, llm.map and zone.fetch go to the executor over stdio', async () => {
  const dir = await workdir(`
export default async function main(aiws) {
  aiws.declare({ inputs: { sales: ['月份', '销售额'] }, ai: { complete: { max_calls: 2 }, map: { max_items: 10 } } })
  const q = await aiws.sql('SELECT "月份", sum("销售额") AS s FROM sales GROUP BY 1 ORDER BY 1')
  aiws.result.table('monthly', q.rows, { key: ['月份'] })
  const { text } = await aiws.ai.complete({ system: 'brief', prompt: 'say hi' })
  const { value } = await aiws.ai.complete({ prompt: 'risk', schema: { type: 'object', properties: { risk: { enum: ['low', 'high'] } } } })
  const labels = await aiws.llm.map([1, 2], '大小', { type: 'string', enum: ['big', 'small'] })
  const res = await aiws.zone.fetch('/kapi/x?y=1')
  aiws.facts({ text, risk: value.risk, labels, zone: await res.json() })
  console.log('program output goes to stderr')
}`)
  const { code, results, stderr, calls } = await runBridged(dir, (op, req) => {
    if (op === 'sql') return { columns: ['月份', 's'], rows: [['2026-07-01', 150.5]], truncated: false }
    if (op === 'ai.complete') return req.schema ? { value: { risk: 'low' }, cached: false, tokens: 5 } : { text: 'hi', cached: true, tokens: 0 }
    if (op === 'llm.map') return { results: req.items.map((i) => ({ value: i > 1 ? 'big' : 'small' })), calls: 1, cached: 0 }
    if (op === 'zone.fetch') return { status: 200, headers: {}, text: '{"ok":1}', url: 'https://zone.example/kapi/x' }
  })
  assertEquals(code, 0, stderr)
  assert(stderr.includes('program output goes to stderr'))
  assertEquals(calls.map((c) => c.op), ['sql', 'ai.complete', 'ai.complete', 'llm.map', 'zone.fetch'])
  assertEquals(calls[0].req.declare.inputs.sales, ['月份', '销售额'])
  assertEquals(results.results[0].rows, [{ 月份: '2026-07-01', s: 150.5 }])
  assertEquals(results.facts, { text: 'hi', risk: 'low', labels: ['small', 'big'], zone: { ok: 1 } })
  assertEquals(results.ai, { complete: { calls: 1, cached: 1, tokens: 5 }, map: { calls: 1, items: 2, cached: 0 }, image: { calls: 0, cached: 0 }, video: { calls: 0, cached: 0 } })
  assertEquals(results.external, ['https://zone.example/kapi/x'])
  // a refusal keeps its code: the run fails with it
  const refused = await workdir(`export default async function main(aiws) { aiws.declare({ inputs: {}, ai: { complete: {} } }); await aiws.ai.complete({ prompt: 'x' }) }`)
  const r = await runBridged(refused, () => ({ error: 'quota', code: 'AI_QUOTA_EXHAUSTED' }))
  assertEquals(r.code, 1)
  assertEquals(r.results.error_code, 'AI_QUOTA_EXHAUSTED')
})

Deno.test('host.js: complete is cached by content, budgeted and refused when undeclared', async () => {
  const cache = new Map()
  let inferred = 0
  const tool = async (name, args) => {
    if (name !== 'llm_cache') return { status: 'error', text: 'no' }
    if (args.get) return { status: 'ok', hits: Object.fromEntries(args.get.filter((k) => cache.has(k)).map((k) => [k, cache.get(k)])) }
    for (const { key, value } of args.put) cache.set(key, value)
    return { status: 'ok' }
  }
  const infer = async (req) => { inferred += 1; return { text: req.schema ? '```json\n{"risk":"high"}\n```' : `answer ${inferred}`, tokens: 10 } }
  const budget = { complete_model: 'llm.chat', map_model: 'llm.chat', ai_max_calls: 2, ai_max_tokens: 100, llm_map_max_items: 3 }
  const host = createBridgeHost({ tool, infer, budget })
  const declare = { inputs: {}, ai: { complete: { max_calls: 5 } } }
  const a = await host.handle('ai.complete', { prompt: 'p', declare })
  const b = await host.handle('ai.complete', { prompt: 'p', declare })
  assertEquals([a.text, a.cached, b.text, b.cached], ['answer 1', false, 'answer 1', true])
  const v = await host.handle('ai.complete', { prompt: 'q', schema: { type: 'object', properties: { risk: { enum: ['low', 'high'] } } }, declare })
  assertEquals(v.value, { risk: 'high' })
  const over = await host.handle('ai.complete', { prompt: 'r', declare })
  assertEquals(over.code, 'AI_BUDGET_EXCEEDED')
  assertEquals((await host.handle('ai.complete', { prompt: 'p', declare: { inputs: {} } })).code, 'AI_NOT_DECLARED')
  assertEquals((await host.handle('ai.complete', { prompt: 'p', model: 'other', declare })).code, 'AI_NOT_DECLARED')
  // a stored macro's manifest wins over what the request says
  const running = createBridgeHost({ tool, infer, budget, manifest: { inputs: {}, results: {} } })
  assertEquals((await running.handle('ai.complete', { prompt: 'p', declare })).code, 'AI_NOT_DECLARED')
  const refusing = createBridgeHost({ tool, infer: async () => { const e = new Error('quota'); e.code = 'AI_QUOTA_EXHAUSTED'; throw e }, budget })
  assertEquals((await refusing.handle('ai.complete', { prompt: 'new', declare })).code, 'AI_QUOTA_EXHAUSTED')
})

Deno.test('generated pictures and clips: the macro holds media references and writes them as file results', async () => {
  const dir = await workdir(`
export default async function main(aiws) {
  aiws.declare({ inputs: { logo: [] }, ai: { image: { max_calls: 3 }, video: { max_calls: 1 } } })
  const logo = aiws.input('logo').media()
  const sheet = await aiws.ai.image({ prompt: '角色设定图', aspect: '3:4', images: [logo] })
  const clip = await aiws.ai.video({ prompt: 'the courier from @{hero} runs', duration: 6, references: { hero: sheet } })
  aiws.result.file('sheet', sheet, { title: '设定图' })
  aiws.result.file('clip', clip)
  aiws.facts({ kinds: [logo.kind, sheet.kind, clip.kind], frozen: Object.isFrozen(sheet) })
}`, { logo: { entity_id: 'logo', type_id: 'buckyos.asset-ref', dir: 'context/entities/logo', label: 'Logo' } })
  Deno.mkdirSync(`${dir}/context/entities/logo`, { recursive: true })
  Deno.writeTextFileSync(`${dir}/context/entities/logo/meta.json`, JSON.stringify({ object_id: 'cyfile:' + '1'.repeat(64), media_type: 'image/png', image: { width: 10, height: 20 } }))
  const { code, results, stderr, calls } = await runBridged(dir, (op, req) => {
    if (op === 'ai.image') return { kind: 'image', object_id: 'cyfile:' + '2'.repeat(64), media_type: 'image/png', size: 1000, cached: false }
    if (op === 'ai.video') return { kind: 'video', object_id: 'cyfile:' + '3'.repeat(64), media_type: 'video/mp4', size: 9000, duration: 6.04, cached: true }
  })
  assertEquals(code, 0, stderr)
  assertEquals(calls.map((c) => c.op), ['ai.image', 'ai.video'])
  assertEquals(calls[0].req.images, [{ kind: 'image', object_id: 'cyfile:' + '1'.repeat(64), media_type: 'image/png', width: 10, height: 20 }])
  assertEquals(calls[1].req.references.hero.object_id, 'cyfile:' + '2'.repeat(64))
  assertEquals(results.results, [
    { name: 'sheet', type: 'file', object_id: 'cyfile:' + '2'.repeat(64), media_type: 'image/png', size: 1000, title: '设定图' },
    { name: 'clip', type: 'file', object_id: 'cyfile:' + '3'.repeat(64), media_type: 'video/mp4', size: 9000 },
  ])
  assertEquals(results.facts, { kinds: ['image', 'image', 'video'], frozen: true })
  assertEquals(results.ai.image, { calls: 1, cached: 0 })
  assertEquals(results.ai.video, { calls: 0, cached: 1 })
})

Deno.test('host.js: pictures and clips are generated with the session, imported, cached, budgeted', async () => {
  const cache = new Map()
  const tool = async (name, args) => {
    if (name !== 'llm_cache') return { status: 'error', text: 'no' }
    if (args.get) return { status: 'ok', hits: Object.fromEntries(args.get.filter((k) => cache.has(k)).map((k) => [k, cache.get(k)])) }
    for (const { key, value } of args.put) cache.set(key, value)
    return { status: 'ok' }
  }
  const sent = []
  let n = 0
  const id = (c) => 'cyfile:' + c.repeat(64)
  const aicc = async (method, params) => {
    sent.push([method, params])
    if (method === 'route.resolve') return { selected_exact_model: 'gpt-image-2@codex-proxy' }
    if (method === 'helper.video_generate') return { status: 'running', task_id: 't-video' }
    n += 1
    return { status: 'succeeded', task_id: `t-${n}`, images: [{ kind: 'named_object', obj_id: id(String(n)) }] }
  }
  let polled = 0
  const task = async (tid) => {
    polled += 1
    return polled < 2 ? { task: { task_id: tid, phase: 'Running' } }
      // a task's output is the inference response under `value`, the stored files beside it (as AICC records it)
      : { task: { task_id: tid, phase: 'Terminal', outcome: 'Succeeded', result: { result: { output: {
        artifacts: [{ mime: 'video/mp4', name: 'video', resource: { kind: 'named_object', obj_id: id('9') }, metadata: { duration_ms: 4030, width: 1280, height: 720 } }],
        value: { videos: [{ media_type: 'video/mp4', resource: { kind: 'named_object', obj_id: id('9') }, measured_metadata: { duration_seconds: 4.03, width: 1280, height: 720 } }] } } } } } }
  }
  const imported = []
  const importAsset = async (obj) => { imported.push(obj); return obj === id('9') ? { object_id: obj, media_type: 'video/mp4', size: 123 } : { object_id: obj, media_type: 'image/png', size: 123, image: { width: 864, height: 1536 } } }
  const budget = { image_model: 'image.txt2img', video_model: 'video.generate', image_max_calls: 2, video_max_calls: 1, media_timeout_secs: 30 }
  const host = createBridgeHost({ tool, infer: async () => ({}), aicc, task, importAsset, budget })
  const declare = { inputs: {}, ai: { image: { max_calls: 5 }, video: { max_calls: 1 } } }
  const a = await host.handle('ai.image', { prompt: '海报', aspect: '3:4', format: 'jpeg', declare })
  assertEquals(a, { kind: 'image', object_id: id('1'), media_type: 'image/png', size: 123, width: 864, height: 1536, cached: false })
  assertEquals(sent[0], ['helper.text_to_image', { prompt: '海报', aspect_ratio: '3:4', output: { media_type: 'image/jpeg' }, logical_model: 'image.txt2img', requirements: {}, disable: {}, policy: { profile: 'balanced', allow_fallback: true, runtime_failover: true } }])
  // the same request: the same picture, staged again for this run, nothing generated
  const b = await host.handle('ai.image', { prompt: '海报', aspect: '3:4', format: 'jpeg', declare })
  assertEquals([b.object_id, b.cached, b.width, n, imported.length], [id('1'), true, 864, 1, 2])
  // reference pictures: the img2img twin of the model, routed first
  const c = await host.handle('ai.image', { prompt: '换成雨夜', images: [a], declare })
  assertEquals(c.object_id, id('2'))
  assertEquals(sent[1], ['route.resolve', { api_type: 'image.img2img', logical_model: 'image.img2img', requirements: {}, disable: {}, policy: { profile: 'balanced', allow_fallback: true, runtime_failover: true } }])
  assertEquals(sent[2], ['image.img2img', { prompt: '换成雨夜', images: [{ kind: 'named_object', obj_id: id('1') }], exact_model: 'gpt-image-2@codex-proxy' }])
  assertEquals((await host.handle('ai.image', { prompt: 'third', declare })).code, 'AI_BUDGET_EXCEEDED', 'the stage allows two')
  // a clip runs as a task: followed until it ends, with what AICC measured
  const v = await host.handle('ai.video', { prompt: 'the courier from @{hero} runs', duration: 4, aspect: '16:9', resolution: '720p', references: { hero: a }, declare })
  assertEquals(v, { kind: 'video', object_id: id('9'), media_type: 'video/mp4', size: 123, duration: 4.03, width: 1280, height: 720, cached: false })
  // only the stored file and what AICC measured in it: the same media reference
  const fromArtifacts = createBridgeHost({ tool, infer: async () => ({}), importAsset, budget: { ...budget, video_max_calls: 2 },
    aicc: async () => ({ status: 'running', task_id: 't-art' }),
    task: async () => ({ task: { phase: 'Terminal', outcome: 'Succeeded', result: { result: { output: { artifacts: [{ mime: 'video/mp4', resource: { kind: 'named_object', obj_id: id('9') }, metadata: { duration_ms: 8000, width: 1344, height: 768 } }], value: {} } } } } }) })
  const w = await fromArtifacts.handle('ai.video', { prompt: 'another shot', declare })
  assertEquals([w.object_id, w.duration, w.width, w.height], [id('9'), 8, 1344, 768])
  const [, vreq] = sent.find(([m]) => m === 'helper.video_generate')
  assertEquals(vreq.output, { duration_seconds: 4, aspect_ratio: '16:9', size: { short_edge: 720 } })
  assertEquals(vreq.conditions, [{ type: 'reference_image', id: 'hero', resource: { kind: 'named_object', obj_id: id('1') } }])
  assertEquals(host.spent().image, 2)
  assertEquals(host.spent().video, 1)
  // what the macro did not declare, an executor without AICC, a bad reference, a refusal
  assertEquals((await host.handle('ai.video', { prompt: 'x', declare: { inputs: {}, ai: {} } })).code, 'AI_NOT_DECLARED')
  assertEquals((await host.handle('ai.image', { prompt: 'x', model: 'image.txt2img.gemini', declare })).code, 'AI_NOT_DECLARED')
  assertEquals((await createBridgeHost({ tool, infer: async () => ({}), budget }).handle('ai.image', { prompt: 'x', declare })).code, 'AI_UNAVAILABLE')
  assert((await host.handle('ai.image', { prompt: 'x', images: ['logo.png'], declare })).error.includes('an image of the workspace'))
  const refusing = createBridgeHost({ tool, infer: async () => ({}), aicc: async () => { const e = new Error('quota'); e.code = 'AI_QUOTA_EXHAUSTED'; throw e }, importAsset, budget })
  assertEquals((await refusing.handle('ai.image', { prompt: 'new', declare })).code, 'AI_QUOTA_EXHAUSTED')
  // a request no model takes: refused before any cost, with what to change
  const unrouted = createBridgeHost({ tool, infer: async () => ({}), aicc: async () => { throw new Error('{"code":"no_candidate_model","message":"no permitted model is available for this request"}') }, importAsset, budget })
  assert((await unrouted.handle('ai.video', { prompt: 'x', first_frame: a, declare })).error.includes('try references instead of first_frame'))
})

Deno.test('host.js: llm.map batches, checks, retries and caches per item', async () => {
  const cache = new Map()
  const asked = []
  const tool = async (_name, args) => {
    if (args.get) return { status: 'ok', hits: Object.fromEntries(args.get.filter((k) => cache.has(k)).map((k) => [k, cache.get(k)])) }
    for (const { key, value } of args.put) cache.set(key, value)
    return { status: 'ok' }
  }
  const infer = async (req) => {
    const user = req.messages[1].text
    asked.push(user)
    if (user.includes('Input items')) {
      const items = JSON.parse(user.split('\n')[3])
      return { text: JSON.stringify(items.map((i) => (i === 'bad' ? 'neither' : i.includes('好') ? '正面' : '负面'))) }
    }
    return { text: '"负面"' }
  }
  const host = createBridgeHost({ tool, infer, budget: { map_model: 'm', llm_map_max_items: 10 } })
  const declare = { inputs: {}, ai: { map: { max_items: 6 } } }
  const schema = { type: 'string', enum: ['正面', '负面'] }
  const first = await host.handle('llm.map', { items: ['很好', '太慢', 'bad'], instruction: '情感', schema, declare })
  assertEquals(first.results, [{ value: '正面' }, { value: '负面' }, { value: '负面' }])
  assertEquals([first.calls, first.cached], [2, 0])
  const again = await host.handle('llm.map', { items: ['很好', '太慢'], instruction: '情感', schema, declare })
  assertEquals([again.calls, again.cached], [0, 2])
  assertEquals((await host.handle('llm.map', { items: ['a', 'b'], instruction: '情感', schema, declare })).code, 'AI_BUDGET_EXCEEDED')
})

Deno.test('host.js: zone.fetch reads only the Zone, only what declare.zone lists (权限需求 R34)', async () => {
  const fetched = []
  const host = createBridgeHost({ tool: async () => ({}), infer: async () => ({}), zoneFetch: async (url, init) => { fetched.push([url, init.method]); return { status: 200, headers: {}, text: 'ok' } },
    zoneHosts: ['test.buckyos.io'], zoneOrigin: 'https://test.buckyos.io' })
  const declare = { inputs: {}, zone: ['/kapi/a', 'https://app.test.buckyos.io/x'] }
  assertEquals((await host.handle('zone.fetch', { url: '/kapi/a', declare })).url, 'https://test.buckyos.io/kapi/a')
  assertEquals((await host.handle('zone.fetch', { url: '/kapi/a/b?c=1', method: 'head', declare })).status, 200)
  assertEquals((await host.handle('zone.fetch', { url: 'https://app.test.buckyos.io/x', declare })).status, 200)
  assertEquals((await host.handle('zone.fetch', { url: 'https://example.com/x', declare })).code, 'ZONE_ONLY')
  assertEquals((await host.handle('zone.fetch', { url: 'https://evil-test.buckyos.io.example.com/', declare })).code, 'ZONE_ONLY')
  // writes are refused, and so is anything the macro did not declare
  assertEquals((await host.handle('zone.fetch', { url: '/kapi/a', method: 'POST', body: '{}', declare })).code, 'ZONE_READ_ONLY')
  assertEquals((await host.handle('zone.fetch', { url: '/kapi/ab', declare })).code, 'ZONE_NOT_DECLARED')
  assertEquals((await host.handle('zone.fetch', { url: '/kapi/a', declare: { inputs: {} } })).code, 'ZONE_NOT_DECLARED')
  assertEquals(fetched, [['https://test.buckyos.io/kapi/a', 'GET'], ['https://test.buckyos.io/kapi/a/b?c=1', 'HEAD'], ['https://app.test.buckyos.io/x', 'GET']])
})

Deno.test('host.js: schema subset, JSON extraction and canonical keys', async () => {
  const s = { type: 'object', properties: { tag: { enum: ['正面', '负面'] }, n: { type: ['number', 'null'] } }, required: ['tag'] }
  assertEquals(validate(s, { tag: '正面', n: null }), null)
  assert(validate(s, { tag: '中性' }) !== null)
  assert(validate(s, { n: 1 }) !== null)
  assertEquals(extractJson('```json\n[1, 2]\n```'), [1, 2])
  assertEquals(extractJson('结果如下：["a"] 完'), ['a'])
  assertEquals(canonical({ b: 1, a: [{ d: 2, c: null }] }), '{"a":[{"c":null,"d":2}],"b":1}')
  assertEquals((await sha256Hex('abc')).length, 64)
})

Deno.test('a control input reads as an outline, control results carry their tree', async () => {
  const branch = { entity_id: 'n-market', type_id: 'buckyos.cell', dir: 'context/entities/n-market', label: 'Market', control: 'mindmap' }
  const dir = await workdir(`
export default async function main(aiws) {
  aiws.declare({ inputs: { branch: [] } })
  const b = aiws.input('branch')
  if (b.kind !== 'control' || b.control !== 'mindmap') throw new Error('kind ' + b.kind)
  const items = b.outline().split('\\n').filter((l) => l.startsWith('  - ')).map((l) => l.trim().slice(2))
  aiws.result.control('ideas', { kind: 'mindmap', children: items.map((t) => ({ text: t + ' next', children: [] })), under: 'branch' })
  aiws.result.control('plan', { kind: 'mindmap', config: { direction: 'both' }, children: '- Plan\\n  - A\\n', title: 'Plan' })
}`, { branch })
  Deno.mkdirSync(`${dir}/context/entities/n-market`, { recursive: true })
  Deno.writeTextFileSync(`${dir}/context/entities/n-market/outline.md`, '- Market\n  - interviews\n  - competitors\n')
  const { code, stdout, results } = await run(dir)
  assertEquals(code, 0, stdout)
  assertEquals(results.results[0], { name: 'ideas', type: 'control', kind: 'mindmap', children: [{ text: 'interviews next', children: [] }, { text: 'competitors next', children: [] }], under: 'branch' })
  assertEquals(results.results[1].config, { direction: 'both' })
  assert(stdout.includes('control ideas: mindmap under branch'), stdout)
  const bad = await workdir(`export default async function main(aiws) { aiws.declare({ inputs: { rules: [] } }); aiws.result.control('x', { kind: 'mindmap', children: '- a', under: 'rules' }) }`)
  assert((await run(bad)).results.error.includes('under must name a control input'))
  const none = await workdir(`export default async function main(aiws) { aiws.result.control('x', { children: '- a' }) }`)
  assert((await run(none)).results.error.includes('kind is required'))
  // a composite control of the Workspace: its slots take other results, inputs and new texts
  const board = await workdir(`export default async function main(aiws) {
  aiws.result.text('summary', '# Done')
  aiws.result.control('board', { def: 'did:bns:alice#board', config: { title: 'W44' }, slots: { notes: ['result:summary', { view: 'richtext', markdown: 'more' }] } })
}`)
  const b = await run(board)
  assertEquals(b.code, 0, b.stdout)
  assertEquals(b.results.results[1], { name: 'board', type: 'control', def: 'did:bns:alice#board', config: { title: 'W44' }, slots: { notes: ['result:summary', { view: 'richtext', markdown: 'more' }] } })
  assert(b.stdout.includes('control board: did:bns:alice#board (notes)'), b.stdout)
  const noSlots = await workdir(`export default async function main(aiws) { aiws.result.control('x', { def: 'did:bns:alice#board' }) }`)
  assert((await run(noSlots)).results.error.includes('slots is required'))
})
