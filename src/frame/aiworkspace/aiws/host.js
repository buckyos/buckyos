// The executor's side of a macro's bridge (AI Native 第一期 §2.3–§2.6; 浏览器优先 Q1). The program host (aiws.js)
// asks; this answers, the same way in every executor — the browser's page for its Worker, the hosted runner for its
// Deno child — so budgets, the cache, schema checks and the Zone boundary mean one thing everywhere:
//
//   sql          → the stage's `proc.tool sql` (the service runs it on the run's snapshot)
//   ai.complete  → one inference with the executor's own AI session (`infer`), cached in `proc.tool llm_cache`
//   llm.map      → per-item judgements: batches, schema checks, single retries, a per-item cache
//   zone.fetch   → a resource of this Zone with the executor's session (`zoneFetch`); anything else is refused
//
// The executor gives: `tool(name, args)` (its `proc.tool`), `infer({ model, messages, schema, max_tokens })` →
// `{ text, tokens? }` (throws with `code` AI_QUOTA_EXHAUSTED / AI_QUOTA_BUDGET_EXCEEDED / AI_RESOURCE_NOT_PERMITTED /
// AI_UNAVAILABLE on a refusal), `zoneFetch(url, { method, headers, body })` → `{ status, headers, text? | base64? }`,
// `zoneHosts` (the Zone's host names), `budget` (the stage's), and `manifest` (the stored macro's, when running it).
// What the macro declares is the request's `declare` while coding, the manifest when running; this takes the smaller
// of each declared limit and the budget, and refuses what the macro did not declare.

export const MAP_SYSTEM = 'You are an item-by-item judge. Judge each input item independently according to the instruction. Output only JSON, with no explanation.'
const DEFAULT_COMPLETE_TOKENS = 4096
const MAP_TOKENS = 4096

/** JSON with sorted keys and no spaces: the text a cache key is the digest of. */
export function canonical(v) {
  if (v === null || typeof v !== 'object') return JSON.stringify(v ?? null)
  if (Array.isArray(v)) return `[${v.map(canonical).join(',')}]`
  return `{${Object.keys(v).sort().filter((k) => v[k] !== undefined).map((k) => `${JSON.stringify(k)}:${canonical(v[k])}`).join(',')}}`
}

export async function sha256Hex(text) {
  const d = await crypto.subtle.digest('SHA-256', new TextEncoder().encode(text))
  return [...new Uint8Array(d)].map((b) => b.toString(16).padStart(2, '0')).join('')
}

function typeOk(t, v) {
  switch (t) {
    case 'string': return typeof v === 'string'
    case 'number': return typeof v === 'number'
    case 'integer': return Number.isInteger(v)
    case 'boolean': return typeof v === 'boolean'
    case 'object': return v !== null && typeof v === 'object' && !Array.isArray(v)
    case 'array': return Array.isArray(v)
    case 'null': return v === null
    default: return true
  }
}

/** The JSON-schema subset a macro's answers are checked against: type, enum, properties, required, items. `null` = valid. */
export function validate(schema, v) {
  if (!schema || typeof schema !== 'object') return null
  const t = schema.type
  if (typeof t === 'string' && !typeOk(t, v)) return `expected ${t}, got ${JSON.stringify(v)}`
  if (Array.isArray(t) && !t.some((x) => typeOk(x, v))) return `expected one of ${JSON.stringify(t)}, got ${JSON.stringify(v)}`
  if (Array.isArray(schema.enum) && !schema.enum.some((e) => canonical(e) === canonical(v))) return `${JSON.stringify(v)} is not one of ${JSON.stringify(schema.enum)}`
  if (schema.properties && v && typeof v === 'object' && !Array.isArray(v)) {
    for (const [k, s] of Object.entries(schema.properties)) {
      if (k in v) {
        const e = validate(s, v[k])
        if (e) return `${k}: ${e}`
      }
    }
  }
  if (Array.isArray(schema.required) && v && typeof v === 'object' && !Array.isArray(v)) {
    for (const k of schema.required) if (!(k in v)) return `missing ${k}`
  }
  if (schema.items && Array.isArray(v)) {
    for (let i = 0; i < v.length; i++) {
      const e = validate(schema.items, v[i])
      if (e) return `[${i}]: ${e}`
    }
  }
  return null
}

/** The JSON a model answered (code fences and surrounding prose tolerated), `undefined` when there is none. */
export function extractJson(text) {
  const tryParse = (t) => { try { return { v: JSON.parse(t) } } catch { return null } }
  let t = String(text ?? '').trim()
  const direct = tryParse(t)
  if (direct) return direct.v
  t = t.replace(/^```json/, '').replace(/^```/, '').replace(/```$/, '').trim()
  const fenced = tryParse(t)
  if (fenced) return fenced.v
  for (const [open, close] of [['[', ']'], ['{', '}']]) {
    const a = t.indexOf(open)
    const b = t.lastIndexOf(close)
    if (a >= 0 && a < b) {
      const inner = tryParse(t.slice(a, b + 1))
      if (inner) return inner.v
    }
  }
  return undefined
}

function capped(declared, max) {
  const m = Number(max) || 0
  return typeof declared === 'number' && declared > 0 ? Math.min(declared, m) : m
}

function refusal(error) {
  const code = typeof error?.code === 'string' ? error.code : null
  return { error: error instanceof Error ? error.message : String(error), ...(code ? { code } : {}) }
}

/** Whether `url` is on one of the Zone's hosts (the host itself or a subdomain of it). */
export function inZone(url, zoneHosts) {
  const host = url.hostname.toLowerCase()
  return zoneHosts.some((z) => {
    const h = String(z).toLowerCase()
    return host === h || host.endsWith(`.${h}`)
  })
}

export function createBridgeHost({ tool, infer, zoneFetch, zoneHosts = [], zoneOrigin = null, budget = {}, manifest = null }) {
  const spent = { calls: 0, tokens: 0, items: 0 }
  const declaredOf = (req) => (manifest && typeof manifest === 'object' ? manifest : req?.declare ?? null)

  async function cacheGet(keys) {
    if (!keys.length) return {}
    const answer = await tool('llm_cache', { get: keys })
    return answer?.status === 'ok' ? answer.hits ?? {} : {}
  }

  async function cachePut(items) {
    if (items.length) await tool('llm_cache', { put: items }).catch(() => undefined)
  }

  async function sql(req) {
    const answer = await tool('sql', { query: req.query, params: req.params ?? null, ...(manifest ? {} : { declare: req.declare ?? null }) })
    if (answer?.status !== 'ok') return { error: answer?.text ?? 'aiws.sql failed', code: answer?.code ?? 'SQL_ERROR' }
    return answer.sql
  }

  async function complete(req) {
    const decl = declaredOf(req)?.ai?.complete
    if (!decl || typeof decl !== 'object') return { error: 'declare it first: aiws.declare({ ai: { complete: { max_calls, max_tokens } } })', code: 'AI_NOT_DECLARED' }
    const maxCalls = capped(decl.max_calls, budget.ai_max_calls)
    const maxTokens = capped(decl.max_tokens, budget.ai_max_tokens)
    const model = req.model ?? budget.complete_model
    const models = [budget.complete_model, ...(Array.isArray(decl.models) ? decl.models : [])]
    if (!models.includes(model)) return { error: `model ${model} is not declared (aiws.declare({ ai: { complete: { models: [...] } } }))`, code: 'AI_NOT_DECLARED' }
    const messages = []
    if (req.system) messages.push({ role: 'system', text: String(req.system) })
    if (Array.isArray(req.messages)) for (const m of req.messages) messages.push({ role: m.role === 'assistant' ? 'assistant' : m.role === 'system' ? 'system' : 'user', text: String(m.text ?? m.content ?? '') })
    else messages.push({ role: 'user', text: String(req.prompt ?? '') })
    const want = Math.max(1, Number(req.max_tokens) || DEFAULT_COMPLETE_TOKENS)
    const key = await sha256Hex(canonical({ kind: 'complete', model, messages, schema: req.schema ?? null, max_tokens: want }))
    const hit = (await cacheGet([key]))[key]
    if (hit && typeof hit === 'object') return { ...hit, cached: true, tokens: 0 }
    if (spent.calls >= maxCalls) return { error: `the AI budget of this run is spent: at most ${maxCalls} calls`, code: 'AI_BUDGET_EXCEEDED' }
    const left = maxTokens - spent.tokens
    if (left <= 0) return { error: `the AI budget of this run is spent: at most ${maxTokens} tokens`, code: 'AI_BUDGET_EXCEEDED' }
    spent.calls += 1
    let answer
    try {
      answer = await infer({ model, messages, schema: req.schema ?? null, max_tokens: Math.min(want, left) })
    } catch (error) {
      return refusal(error)
    }
    const tokens = Number(answer?.tokens) || Math.min(want, left)
    spent.tokens += tokens
    let out
    if (req.schema) {
      const value = extractJson(answer?.text)
      const bad = value === undefined ? 'no JSON in the answer' : validate(req.schema, value)
      if (bad) return { error: `the model's answer does not match the schema: ${bad}`, code: 'AI_INVALID_ANSWER' }
      out = { value }
    } else {
      out = { text: String(answer?.text ?? '') }
    }
    await cachePut([{ key, value: out }])
    return { ...out, cached: false, tokens }
  }

  async function map(req) {
    const decl = declaredOf(req)?.ai?.map
    if (!decl || typeof decl !== 'object') return { error: 'declare it first: aiws.declare({ ai: { map: { max_items } } })', code: 'AI_NOT_DECLARED' }
    const items = Array.isArray(req.items) ? req.items : []
    const instruction = String(req.instruction ?? '')
    const schema = req.schema ?? { type: 'string' }
    if (!instruction.trim()) return { error: 'llm.map needs an instruction' }
    const maxItems = capped(decl.max_items, budget.llm_map_max_items)
    spent.items += items.length
    if (spent.items > maxItems) return { error: `llm.map budget exceeded: at most ${maxItems} items per run — narrow the range`, code: 'AI_BUDGET_EXCEEDED' }
    const model = budget.map_model
    const keys = await Promise.all(items.map((x) => sha256Hex(canonical({ m: model, i: instruction, s: schema, x }))))
    const hits = await cacheGet([...new Set(keys)])
    const results = items.map((_, i) => (keys[i] in hits ? { value: hits[keys[i]] } : null))
    const cached = results.filter(Boolean).length
    let calls = 0
    const put = []
    const ask = async (user) => {
      calls += 1
      return infer({ model, messages: [{ role: 'system', text: MAP_SYSTEM }, { role: 'user', text: user }], schema: null, max_tokens: MAP_TOKENS })
    }
    const schemaText = JSON.stringify(schema)
    const todo = results.map((r, i) => (r ? -1 : i)).filter((i) => i >= 0)
    const batch = Math.min(50, Math.max(1, Number(req.batch) || 20))
    try {
      for (let s = 0; s < todo.length; s += batch) {
        const chunk = todo.slice(s, s + batch)
        const list = chunk.map((i) => items[i])
        const answer = await ask(`Instruction: ${instruction}\nThe output for each item must conform to the JSON schema: ${schemaText}\nInput items (a JSON array of ${list.length} items):\n${JSON.stringify(list)}\n\nOutput only one JSON array whose i-th element is the result for the i-th item; its length must be ${list.length}.`)
          .catch((error) => { if (error?.code) throw error; return null })
        const parsed = answer ? extractJson(answer.text) : undefined
        if (!Array.isArray(parsed) || parsed.length !== chunk.length) continue
        chunk.forEach((i, j) => {
          if (validate(schema, parsed[j]) === null) {
            results[i] = { value: parsed[j] }
            put.push({ key: keys[i], value: parsed[j] })
          }
        })
      }
      // what failed in a batch is asked again alone; what still fails is reported, never invented
      for (let i = 0; i < items.length; i++) {
        if (results[i]) continue
        let answer = null
        let failure = null
        try {
          answer = await ask(`Instruction: ${instruction}\nThe output must conform to the JSON schema: ${schemaText}\nInput item: ${JSON.stringify(items[i])}\n\nOutput only the JSON result for this item.`)
        } catch (error) {
          if (error?.code) throw error
          failure = error instanceof Error ? error.message : String(error)
        }
        let v = answer ? extractJson(answer.text) : undefined
        if (v === undefined && answer && schema.type === 'string') v = String(answer.text ?? '').trim()
        if (v === undefined) results[i] = { error: failure ?? 'no JSON in the answer' }
        else {
          const bad = validate(schema, v)
          if (bad) results[i] = { error: `invalid answer: ${bad}` }
          else {
            results[i] = { value: v }
            put.push({ key: keys[i], value: v })
          }
        }
      }
    } catch (error) {
      await cachePut(put)
      return refusal(error)
    }
    await cachePut(put)
    return { results, calls, cached }
  }

  async function fetchZone(req) {
    let url
    try {
      url = new URL(String(req.url ?? ''), zoneOrigin ?? undefined)
    } catch {
      return { error: `${req.url} is not a URL or a path of this Zone`, code: 'ZONE_ONLY' }
    }
    if (!['http:', 'https:'].includes(url.protocol) || !inZone(url, zoneHosts)) return { error: `${url.origin} is outside this Zone: a macro only reaches the Zone's own resources`, code: 'ZONE_ONLY' }
    try {
      const r = await zoneFetch(url.href, { method: req.method ?? 'GET', headers: req.headers ?? {}, body: req.body ?? null })
      return { ...r, url: url.origin + url.pathname }
    } catch (error) {
      return refusal(error)
    }
  }

  return {
    async handle(op, req) {
      try {
        switch (op) {
          case 'sql': return await sql(req ?? {})
          case 'ai.complete': return await complete(req ?? {})
          case 'llm.map': return await map(req ?? {})
          case 'zone.fetch': return await fetchZone(req ?? {})
          default: return { error: `unknown bridge call ${op}` }
        }
      } catch (error) {
        return refusal(error)
      }
    },
    spent: () => ({ ...spent }),
  }
}
