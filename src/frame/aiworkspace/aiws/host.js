// The executor's side of a macro's bridge (AI Native 第一期 §2.3–§2.6; 浏览器优先 Q1). The program host (aiws.js)
// asks; this answers, the same way in every executor — the browser's page for its Worker, the hosted runner for its
// Deno child — so budgets, the cache, schema checks and the Zone boundary mean one thing everywhere:
//
//   sql          → the stage's `proc.tool sql` (the service runs it on the run's snapshot)
//   ai.complete  → one inference with the executor's own AI session (`infer`), cached in `proc.tool llm_cache`
//   llm.map      → per-item judgements: batches, schema checks, single retries, a per-item cache
//   zone.fetch   → GET / HEAD of a resource of this Zone its `declare.zone` lists, with the executor's session
//                  (`zoneFetch`); anything else is refused (权限需求 R34)
//   ai.image     → one AICC image generation (`helper.text_to_image`, or `image.img2img` with reference images), and
//   ai.video       one video generation (`helper.video_generate`), with the executor's session; the output goes into
//                  the Workspace as a staged asset (`importAsset`), so the macro holds an object id, never the bytes.
//                  Cached in `proc.tool llm_cache` by the request: the same request gives the same picture.
//
// The executor gives: `tool(name, args)` (its `proc.tool`), `infer({ model, messages, schema, max_tokens })` →
// `{ text, tokens? }` (throws with `code` AI_QUOTA_EXHAUSTED / AI_QUOTA_BUDGET_EXCEEDED / AI_RESOURCE_NOT_PERMITTED /
// AI_UNAVAILABLE on a refusal), `zoneFetch(url, { method, headers, body })` → `{ status, headers, text? | base64? }`,
// `aicc(method, params)` (one AICC call; a refusal as for `infer`), `task(id)` (the task manager's record of an AICC
// task), `importAsset(obj_id)` → `{ object_id, media_type, size }` (`asset.import_object` of this Workspace),
// `zoneHosts` (the Zone's host names), `budget` (the stage's), and `manifest` (the stored macro's, when running it).
// What the macro declares is the request's `declare` while coding, the manifest when running; this takes the smaller
// of each declared limit and the budget, and refuses what the macro did not declare.

export const MAP_SYSTEM = 'You are an item-by-item judge. Judge each input item independently according to the instruction. Output only JSON, with no explanation.'
const DEFAULT_COMPLETE_TOKENS = 4096
const MAP_TOKENS = 4096
const MEDIA_DEFAULTS = { image: { model: 'image.txt2img', max_calls: 20 }, video: { model: 'video.generate', max_calls: 2 } }
const DEFAULT_MEDIA_TIMEOUT_SECS = 900
const MEDIA_POLICY = { profile: 'balanced', allow_fallback: true, runtime_failover: true }
const FORMATS = { png: 'image/png', jpeg: 'image/jpeg', jpg: 'image/jpeg', webp: 'image/webp' }
const OBJECT_ID = /^cyfile:[0-9a-f]{16,}$/

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

/** Whether `declared.zone` (path prefixes such as `/kapi/x/`, or URL prefixes) lists `url`. */
export function declaredZone(declared, url) {
  const list = declared && Array.isArray(declared.zone) ? declared.zone : []
  return list.some((entry) => {
    const e = String(entry)
    if (e.startsWith('/')) return url.pathname === e || url.pathname.startsWith(e.endsWith('/') ? e : `${e}/`)
    try {
      const d = new URL(e)
      return d.origin === url.origin && (url.pathname === d.pathname || url.pathname.startsWith(d.pathname.endsWith('/') ? d.pathname : `${d.pathname}/`))
    } catch {
      return false
    }
  })
}

/** A media reference of the macro (an `aiws.ai.image` / `ai.video` answer, an asset input's `media()`) → AICC's form. */
function mediaRef(v, what) {
  const id = typeof v === 'string' ? v : v && typeof v === 'object' ? v.object_id : null
  if (typeof id !== 'string' || !OBJECT_ID.test(id)) throw new Error(`${what}: an image of the workspace (an aiws.ai.image answer, or input(...).media())`)
  return { kind: 'named_object', obj_id: id }
}

/** The AICC request of `aiws.ai.image` / `ai.video` (without the route): what the cache key is made of. */
function mediaRequest(kind, req) {
  const prompt = typeof req.prompt === 'string' ? req.prompt.trim() : ''
  if (!prompt) throw new Error(`aiws.ai.${kind}: prompt (a string) required`)
  const out = { prompt }
  if (typeof req.negative === 'string' && req.negative) out.negative_prompt = req.negative
  if (Number.isInteger(req.seed)) out.seed = req.seed
  if (kind === 'image') {
    if (req.aspect) out.aspect_ratio = String(req.aspect)
    if (req.size) {
      if (!/^\d+x\d+$/.test(String(req.size))) throw new Error('aiws.ai.image: size is WxH, such as 1024x1536')
      out.size = String(req.size)
    }
    if (req.quality) out.quality = String(req.quality)
    if (req.format) {
      const media = FORMATS[String(req.format).toLowerCase()]
      if (!media) throw new Error(`aiws.ai.image: format is one of ${Object.keys(FORMATS).join(', ')}`)
      out.output = { media_type: media }
    }
    const images = req.images === undefined || req.images === null ? [] : Array.isArray(req.images) ? req.images : [req.images]
    if (images.length > 8) throw new Error('aiws.ai.image: at most 8 reference images')
    if (images.length) out.images = images.map((x) => mediaRef(x, 'aiws.ai.image images'))
    return out
  }
  const output = {}
  if (req.duration !== undefined && req.duration !== null) {
    const d = Number(req.duration)
    if (!(d > 0 && d <= 60)) throw new Error('aiws.ai.video: duration is in seconds (0–60)')
    output.duration_seconds = d
  }
  if (req.aspect) output.aspect_ratio = String(req.aspect)
  if (req.resolution) {
    const edge = Number(String(req.resolution).replace(/p$/i, ''))
    if (!(edge >= 240 && edge <= 4320)) throw new Error('aiws.ai.video: resolution such as 720p or 1080p')
    output.size = { short_edge: edge }
  }
  if (Object.keys(output).length) out.output = output
  const conditions = []
  if (req.first_frame) conditions.push({ type: 'keyframe', id: 'first', position: 'start', resource: mediaRef(req.first_frame, 'aiws.ai.video first_frame') })
  if (req.last_frame) conditions.push({ type: 'keyframe', id: 'last', position: 'end', resource: mediaRef(req.last_frame, 'aiws.ai.video last_frame') })
  for (const [id, v] of Object.entries(req.references ?? {})) {
    if (!/^[A-Za-z][A-Za-z0-9_]{0,31}$/.test(id)) throw new Error(`aiws.ai.video references: ${JSON.stringify(id)} is not a name (letters, digits, _)`)
    conditions.push({ type: 'reference_image', id, resource: mediaRef(v, `aiws.ai.video references.${id}`) })
  }
  if (conditions.length) out.conditions = conditions
  return out
}

/** The width and height the Workspace found in a staged picture. */
function dims(staged) {
  const i = staged?.image
  return i && Number(i.width) > 0 && Number(i.height) > 0 ? { width: Number(i.width), height: Number(i.height) } : {}
}

/** The answer of a terminal AICC task record: the inference response (`output.value`) with the stored files
 * (`output.artifacts`) beside it — what a request answered at once carries directly. */
function taskOutput(task) {
  const r = task?.result
  if (!r || typeof r !== 'object') return null
  const out = r.result && typeof r.result === 'object' && r.result.output && typeof r.result.output === 'object' ? r.result.output
    : r.output && typeof r.output === 'object' ? r.output : null
  if (!out) return null
  return out.value && typeof out.value === 'object' && !Array.isArray(out.value) ? { ...out.value, artifacts: out.artifacts ?? out.value.artifacts } : out
}

/** The one picture or video of an AICC answer: `{ obj_id, media_type?, metadata? }`, or a text saying why not. */
function mediaOf(kind, response) {
  const r = response ?? {}
  let resource = null
  let media = null
  let metadata = null
  if (kind === 'image') {
    resource = Array.isArray(r.images) ? r.images[0] : r.image
  } else {
    const v = Array.isArray(r.videos) ? r.videos[0] : null
    resource = v?.resource ?? null
    media = typeof v?.media_type === 'string' ? v.media_type : null
    metadata = v?.measured_metadata ?? null
  }
  // the files AICC stored, with what it measured in them (`duration_ms`, `width`, `height`)
  const stored = (Array.isArray(r.artifacts) ? r.artifacts : []).filter((a) => a?.resource?.kind === 'named_object' && String(a.mime ?? '').startsWith(`${kind}/`))
  const art = stored.find((a) => a.resource.obj_id === resource?.obj_id) ?? (resource ? null : stored[0])
  if (art) {
    resource ??= art.resource
    media ??= typeof art.mime === 'string' ? art.mime : null
    const m = art.metadata ?? {}
    if (!metadata && (Number(m.duration_ms) > 0 || Number(m.width) > 0)) metadata = { duration_seconds: Number(m.duration_ms) / 1000 || undefined, width: m.width, height: m.height }
  }
  if (!resource || typeof resource !== 'object') return { missing: `the AI service returned no ${kind}` }
  if (resource.kind !== 'named_object' || typeof resource.obj_id !== 'string') return { missing: `the AI service returned the ${kind} as ${resource.kind}, which a macro cannot keep` }
  return { obj_id: resource.obj_id, media_type: media, metadata }
}

export function createBridgeHost({ tool, infer, zoneFetch, aicc = null, task = null, importAsset = null, zoneHosts = [], zoneOrigin = null, budget = {}, manifest = null }) {
  const spent = { calls: 0, tokens: 0, items: 0, image: 0, video: 0 }
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

  /** One AICC call and, while its task runs, the task manager until it ends (at most the stage's media timeout). */
  async function generate(kind, model, request) {
    let response
    if (kind === 'video') {
      response = await aicc('helper.video_generate', { ...request, logical_model: model, requirements: {}, disable: {}, policy: MEDIA_POLICY })
    } else if (request.images) {
      const logical = model.replace(/^image\.txt2img/, 'image.img2img')
      const exact = logical.includes('@') ? logical
        : (await aicc('route.resolve', { api_type: 'image.img2img', logical_model: logical, requirements: {}, disable: {}, policy: MEDIA_POLICY }))?.selected_exact_model
      if (!exact) throw new Error(`no model serves image.img2img for ${logical}`)
      response = await aicc('image.img2img', { ...request, exact_model: exact })
    } else {
      response = await aicc('helper.text_to_image', { ...request, logical_model: model, requirements: {}, disable: {}, policy: MEDIA_POLICY })
    }
    if (response?.status === 'succeeded') return response
    if (response?.status === 'failed') throw new Error(`the AI service failed: ${JSON.stringify(response.error ?? response.message ?? response)}`)
    const id = response?.task_id
    if (typeof id !== 'string' || typeof task !== 'function') throw new Error(`the AI service answered without a result: ${JSON.stringify(response).slice(0, 500)}`)
    const secs = Number(budget.media_timeout_secs) || DEFAULT_MEDIA_TIMEOUT_SECS
    const deadline = Date.now() + secs * 1000
    for (;;) {
      const t = await task(id)
      const rec = t && typeof t === 'object' && 'task' in t ? t.task : t
      if (rec?.phase === 'Terminal') {
        if (rec.outcome !== 'Succeeded') throw new Error(`AICC task ${id} ${String(rec.outcome ?? 'failed').toLowerCase()}: ${rec.message ?? JSON.stringify(rec.error ?? null)}`)
        return taskOutput(rec) ?? {}
      }
      if (Date.now() > deadline) throw new Error(`AICC task ${id} is still running after ${secs} s`)
      await new Promise((resolve) => setTimeout(resolve, 2000))
    }
  }

  async function media(kind, req) {
    const decl = declaredOf(req)?.ai?.[kind]
    if (!decl || typeof decl !== 'object') return { error: `declare it first: aiws.declare({ ai: { ${kind}: { max_calls } } })`, code: 'AI_NOT_DECLARED' }
    if (typeof aicc !== 'function' || typeof importAsset !== 'function') return { error: `this executor cannot generate ${kind === 'image' ? 'images' : 'videos'}`, code: 'AI_UNAVAILABLE' }
    const maxCalls = capped(decl.max_calls, budget[`${kind}_max_calls`] ?? MEDIA_DEFAULTS[kind].max_calls)
    const preset = typeof budget[`${kind}_model`] === 'string' && budget[`${kind}_model`] ? budget[`${kind}_model`] : MEDIA_DEFAULTS[kind].model
    const model = req.model ?? preset
    const models = [preset, ...(Array.isArray(decl.models) ? decl.models : [])]
    if (!models.includes(model)) return { error: `model ${model} is not declared (aiws.declare({ ai: { ${kind}: { models: [...] } } }))`, code: 'AI_NOT_DECLARED' }
    let request
    try {
      request = mediaRequest(kind, req)
    } catch (error) {
      return { error: error.message }
    }
    const key = await sha256Hex(canonical({ kind, model, request }))
    const hit = (await cacheGet([key]))[key]
    if (hit && typeof hit === 'object' && typeof hit.object_id === 'string') {
      // the same request made this before: stage it again for this run (it may have been applied, or have expired)
      const staged = await importAsset(hit.object_id).catch(() => null)
      if (staged) return { ...hit, media_type: staged.media_type ?? hit.media_type, size: staged.size ?? hit.size, ...dims(staged), cached: true }
    }
    if (spent[kind] >= maxCalls) return { error: `the ${kind} budget of this run is spent: at most ${maxCalls} calls`, code: 'AI_BUDGET_EXCEEDED' }
    spent[kind] += 1
    let out
    try {
      const got = mediaOf(kind, await generate(kind, model, request))
      if (got.missing) return { error: got.missing, code: 'AI_INVALID_ANSWER' }
      const staged = await importAsset(got.obj_id)
      out = { kind, object_id: staged.object_id ?? got.obj_id, media_type: staged.media_type ?? got.media_type, size: staged.size ?? null, ...dims(staged) }
      const m = got.metadata
      if (m && typeof m === 'object') {
        if (Number(m.duration_seconds) > 0) out.duration = Number(m.duration_seconds)
        if (Number(m.width) > 0 && Number(m.height) > 0) Object.assign(out, { width: Number(m.width), height: Number(m.height) })
      }
    } catch (error) {
      const r = refusal(error)
      // AICC refuses a request no model of this Zone can honour before any cost: say what to change
      if (/no_candidate_model/.test(r.error)) {
        r.error += kind === 'video'
          ? ' — no video model of this Zone takes this request; try references instead of first_frame / last_frame (or the other way round), another duration, aspect or resolution'
          : ' — no image model of this Zone takes this request; try fewer reference images, another size or aspect'
      }
      return r
    }
    await cachePut([{ key, value: out }])
    return { ...out, cached: false }
  }

  async function fetchZone(req) {
    let url
    try {
      url = new URL(String(req.url ?? ''), zoneOrigin ?? undefined)
    } catch {
      return { error: `${req.url} is not a URL or a path of this Zone`, code: 'ZONE_ONLY' }
    }
    if (!['http:', 'https:'].includes(url.protocol) || !inZone(url, zoneHosts)) return { error: `${url.origin} is outside this Zone: a macro only reaches the Zone's own resources`, code: 'ZONE_ONLY' }
    // a macro reads the Zone, it never changes it, and only where it said it would (权限需求 R34): GET / HEAD of a
    // resource its `declare.zone` lists (a path prefix, or a URL prefix of the Zone's hosts)
    const method = String(req.method ?? 'GET').toUpperCase()
    if (method !== 'GET' && method !== 'HEAD') return { error: `zone.fetch ${method}: a macro only reads the Zone (GET or HEAD)`, code: 'ZONE_READ_ONLY' }
    if (!declaredZone(declaredOf(req), url)) return { error: `${url.origin}${url.pathname} is not in declare.zone: list what the macro reads`, code: 'ZONE_NOT_DECLARED' }
    try {
      const r = await zoneFetch(url.href, { method, headers: req.headers ?? {}, body: null })
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
          case 'ai.image': return await media('image', req ?? {})
          case 'ai.video': return await media('video', req ?? {})
          default: return { error: `unknown bridge call ${op}` }
        }
      } catch (error) {
        return refusal(error)
      }
    },
    spent: () => ({ ...spent }),
  }
}
