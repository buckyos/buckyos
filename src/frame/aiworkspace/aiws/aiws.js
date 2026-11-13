// aiws v3 — the program host of a wish macro (许愿格详细设计 §9.4; AI Native 第一期 §2).
//
// A macro reads the run's snapshot directory and writes candidate results; it never writes the Workspace. It starts
// with `aiws.declare` (the inputs and fields it reads, the AI it calls): only declared inputs and fields can be read,
// and every later run materializes exactly those. What reaches beyond the snapshot — SQL over the declared inputs, a
// model call, a resource of the Zone — goes through the executor's bridge (`globalThis.__aiwsBridge(op, request)`):
// the browser's Worker posts to the page, the hosted runner (`lib/run.js --bridge`) speaks over stdio. Every executor
// runs the same file.
//
//   context/inputs.json            name → { entity_id, type_id, dir, selector, … }
//   context/entities/<id>/…        schema.json + rows.jsonl | content.md / content.json | meta.json
//   context/assets/<object id>     asset bytes
//   request.json                   the run's fixed parameters
//   output/.aiws/results.json      what the host collects

const dec = new TextDecoder()

function readJson(path, fallback) {
  try {
    return JSON.parse(Deno.readTextFileSync(path))
  } catch (_) {
    return fallback
  }
}

function join(...parts) {
  return parts.join('/').replace(/\/+/g, '/')
}

const RESULT_NAME = /^[^/\\:*?"<>|]{1,64}$/

function checkName(name) {
  if (typeof name !== 'string' || !RESULT_NAME.test(name) || name.trim() !== name || name.includes('..')) {
    throw new Error(`invalid result name ${JSON.stringify(name)}: 1–64 chars, no path separators, colons or ..`)
  }
}

/** `{ 字段: 类型 }` or `[{ name, type, options? }]` → ordered array. */
function fieldList(fields) {
  if (!fields) return []
  if (Array.isArray(fields)) return fields.map((f) => (typeof f === 'string' ? { name: f } : { ...f }))
  return Object.entries(fields).map(([name, t]) => (typeof t === 'string' ? { name, type: t } : { name, ...t }))
}

/** The message and the stack: a stack of SpiderMonkey or JavaScriptCore (the browser's executor) does not repeat the message. */
function errorText(error) {
  const text = String(error)
  const stack = typeof error?.stack === 'string' ? error.stack : ''
  return stack.includes(text) ? stack : [text, stack].filter(Boolean).join('\n')
}

function compareValues(a, b) {
  if (a === b) return 0
  if (a === null || a === undefined) return 1
  if (b === null || b === undefined) return -1
  if (typeof a === 'number' && typeof b === 'number') return a - b
  return String(a).localeCompare(String(b), 'zh-CN')
}

/** What the executor's bridge answered, or the error it gave (its `code` kept: AI_QUOTA_*, SQL_ERROR, ZONE_ONLY…). */
async function bridge(op, req) {
  const call = globalThis.__aiwsBridge
  if (typeof call !== 'function') throw new Error(`aiws.${op} is only available inside a wish run`)
  const body = await call(op, req)
  if (!body || typeof body !== 'object') throw new Error(`aiws.${op}: no answer`)
  if (body.error) {
    const e = new Error(`aiws.${op}: ${body.error}`)
    if (body.code) e.code = body.code
    throw e
  }
  return body
}

class Input {
  #host
  #rows = null
  #declared
  constructor(host, name, info, declared = null) {
    this.#host = host
    this.#declared = declared
    this.name = name
    this.entity_id = info.entity_id
    this.type_id = info.type_id
    this.kind = info.control ? 'control' : (info.type_id || '').replace('buckyos.', '').replace('table-source', 'table').replace('asset-ref', 'asset').replace('container', 'folder')
    /** A control input (a control, or one of its nodes): the control's kind. */
    this.control = info.control ?? null
    this.title = info.label ?? info.title ?? name
    this.dir = join(host.root, info.dir ?? `context/entities/${info.entity_id}`)
    this.selector = info.selector ?? null
  }

  meta() {
    return readJson(join(this.dir, 'meta.json'), {})
  }

  profile() {
    return readJson(join(this.dir, 'profile.json'), {})
  }

  #schema() {
    return readJson(join(this.dir, 'schema.json'), { fields: [] }).fields ?? []
  }

  /** The declared fields in table order: `{ name, type, options?, id, visible }`. */
  fields() {
    this.#expect('table')
    return this.#schema().filter((f) => this.#declared === null || this.#declared.has(f.name)).map((f) => ({
      name: f.name, type: f.type, options: f.options ?? undefined, id: f.field_id, visible: f.visible !== false,
    }))
  }

  /** A field of the table this macro did not declare: reading it is an error, never an empty value. */
  #undeclared(key) {
    return typeof key === 'string' && key !== '_id' && this.#declared !== null && !this.#declared.has(key) && this.#schema().some((f) => f.name === key)
  }

  #guard(row) {
    const input = this
    return new Proxy(row, {
      get(target, key, receiver) {
        if (!(key in target) && input.#undeclared(key)) throw new Error(`input ${input.name}: field ${key} is not declared — add it to aiws.declare({ inputs: { ${input.name}: [...] } })`)
        return Reflect.get(target, key, receiver)
      },
    })
  }

  #expect(kind) {
    if (this.kind !== kind) throw new Error(`input ${this.name} is a ${this.kind}, not a ${kind}`)
  }

  #allRows() {
    this.#expect('table')
    if (this.#rows) return this.#rows
    const byId = new Map(this.fields().map((f) => [f.id, f.name]))
    let text = ''
    try {
      text = Deno.readTextFileSync(join(this.dir, 'rows.jsonl'))
    } catch (_) {
      throw new Error(`rows of input ${this.name} are not in the snapshot (is it an input of this run?)`)
    }
    const rows = []
    for (const line of text.split('\n')) {
      if (!line.trim()) continue
      const r = JSON.parse(line)
      const o = { _id: r.id }
      for (const [fid, v] of Object.entries(r.v ?? {})) {
        const name = byId.get(fid)
        if (name !== undefined) o[name] = v
      }
      rows.push(this.#guard(o))
    }
    this.#rows = rows
    return rows
  }

  /**
   * Rows as `{ 字段名: 值, _id }` in the input's order (a view input: the view's filter and order).
   * `fields`: names to keep; `filter`: a function or `{ 字段: 值 | [值…] }`; `sort`: `'字段'`,
   * `'-字段'` or `[{ field, direction }]`; `limit`.
   */
  rows(opts = {}) {
    let rows = this.#allRows()
    const known = new Set(this.fields().map((f) => f.name))
    for (const f of opts.fields ?? []) if (!known.has(f)) throw new Error(`input ${this.name} has no declared field ${f} (declared: ${[...known].join(', ')})`)
    if (typeof opts.filter === 'function') rows = rows.filter(opts.filter)
    else if (opts.filter && typeof opts.filter === 'object') {
      const conds = Object.entries(opts.filter)
      for (const [k] of conds) if (!known.has(k)) throw new Error(`input ${this.name} has no declared field ${k}`)
      rows = rows.filter((r) => conds.every(([k, v]) => (Array.isArray(v) ? v.includes(r[k]) : r[k] === v)))
    }
    if (opts.sort) {
      const keys = (Array.isArray(opts.sort) ? opts.sort : [opts.sort]).map((s) =>
        typeof s === 'string' ? (s.startsWith('-') ? { field: s.slice(1), desc: true } : { field: s, desc: false }) : { field: s.field, desc: s.direction === 'desc' })
      rows = [...rows].sort((a, b) => {
        for (const k of keys) {
          const c = compareValues(a[k.field], b[k.field])
          if (c !== 0) return k.desc ? -c : c
        }
        return 0
      })
    }
    if (opts.fields) {
      const keep = new Set(opts.fields)
      rows = rows.map((r) => {
        const o = { _id: r._id }
        for (const k of keep) o[k] = r[k]
        return o
      })
    }
    if (typeof opts.limit === 'number') rows = rows.slice(0, opts.limit)
    return rows.map((r) => this.#guard({ ...r }))
  }

  /** Rich text as Markdown. */
  markdown() {
    this.#expect('richtext')
    return Deno.readTextFileSync(join(this.dir, 'content.md'))
  }

  /** The stored structure (rich text AST, record `{ schema, props, by_name }`, annotation). */
  json() {
    return readJson(join(this.dir, 'content.json'), null)
  }

  /** Record properties by display name. */
  props() {
    this.#expect('record')
    return { ...(this.json()?.by_name ?? {}) }
  }

  /** Asset bytes. */
  bytes() {
    this.#expect('asset')
    const m = this.meta()
    if (!m.asset_file) throw new Error(`asset ${this.name} has no bytes in the snapshot`)
    return Deno.readFileSync(join(this.#host.root, m.asset_file))
  }

  text() {
    return dec.decode(this.bytes())
  }

  /** A control as a Markdown outline (a node: its branch, the node as the one root item). */
  outline() {
    this.#expect('control')
    return Deno.readTextFileSync(join(this.dir, 'outline.md'))
  }

  /** Folder members as inputs (each with its own kind). */
  members() {
    this.#expect('folder')
    return (this.meta().members ?? []).map((m) => new Input(this.#host, m.title ?? m.entity_id, { entity_id: m.entity_id, type_id: m.type_id, label: m.title }))
  }
}

export class ProgramHost {
  constructor(root) {
    this.root = root.replace(/\/$/, '')
    this.inputs = readJson(join(this.root, 'context/inputs.json'), {})
    this.request = readJson(join(this.root, 'request.json'), {})
    this.results = []
    this.factsObj = {}
    this.checks = []
    this.external = new Set()
    /** What `aiws.declare` said, as given (the service resolves it into the manifest). */
    this.declared = null
    this.aiStats = { complete: { calls: 0, cached: 0, tokens: 0 }, map: { calls: 0, items: 0, cached: 0 } }
    this.#trackFetch()
    this.aiws = this.#api()
  }

  /** A fetch the program made itself (the hosted runner allows the Zone's host): recorded once it got an answer. */
  #trackFetch() {
    const original = globalThis.fetch
    if (!original || original.__aiws) return
    const external = this.external
    const wrapped = async (input, init) => {
      const url = typeof input === 'string' ? input : input?.url ?? String(input)
      const res = await original(input, init)
      external.add(url.split('?')[0])
      return res
    }
    wrapped.__aiws = true
    globalThis.fetch = wrapped
  }

  #declare(spec) {
    if (this.declared) throw new Error('aiws.declare is called once, at the start of the macro')
    if (!spec || typeof spec !== 'object' || Array.isArray(spec)) throw new Error('aiws.declare({ inputs: { name: [fields…] }, ai?, zone?, params?, resources? })')
    for (const k of Object.keys(spec)) if (!['inputs', 'ai', 'zone', 'params', 'resources'].includes(k)) throw new Error(`aiws.declare: unknown key ${k}`)
    const inputs = {}
    for (const [name, want] of Object.entries(spec.inputs ?? {})) {
      const info = this.inputs[name]
      if (!info) throw new Error(`aiws.declare: no input named ${JSON.stringify(name)} (inputs: ${Object.keys(this.inputs).join(', ') || 'none'})`)
      const fields = Array.isArray(want) ? want : want && typeof want === 'object' && Array.isArray(want.fields) ? want.fields : []
      const input = new Input(this, name, info)
      if (input.kind === 'table') {
        const known = input.fields().map((f) => f.name)
        for (const f of fields) if (!known.includes(f)) throw new Error(`aiws.declare: input ${name} has no field ${JSON.stringify(f)} (fields: ${known.join(', ')})`)
      }
      inputs[name] = [...new Set(fields.map(String))]
    }
    this.declared = { ...spec, inputs }
  }

  #declaredInput(name) {
    if (!this.declared) throw new Error(`declare the inputs first: aiws.declare({ inputs: { ${name}: [fields…] } }) at the start of the macro`)
    const info = this.inputs[name]
    if (!info) throw new Error(`no input named ${JSON.stringify(name)} (inputs: ${Object.keys(this.inputs).join(', ') || 'none'})`)
    const fields = this.declared.inputs[name]
    if (!fields) throw new Error(`input ${name} is not declared: add it to aiws.declare({ inputs: { ${name}: [fields…] } })`)
    return new Input(this, name, info, new Set(fields))
  }

  #push(result) {
    checkName(result.name)
    if (this.results.some((r) => r.name === result.name)) throw new Error(`result ${result.name} was written twice`)
    this.results.push(result)
  }

  #api() {
    const host = this
    const input = (name) => host.#declaredInput(name)
    const declared = () => {
      if (!host.declared) throw new Error('declare first: aiws.declare({ inputs: { … }, ai: { … } }) at the start of the macro')
      return host.declared
    }
    return {
      version: 3,
      host: 'program',
      request: host.request,
      /** What this macro reads and may call; once, at its start. Every later run materializes exactly this. */
      declare: (spec) => host.#declare(spec),
      input,
      inputs: () => Object.keys(host.inputs),
      /**
       * One SELECT over the declared table inputs: a table is its input name, its columns the declared fields (display
       * values; `"<field>#raw"` the stored values) and `_id`. Returns `{ columns, rows: [{ column: value }], truncated }`.
       */
      async sql(query, params) {
        if (typeof query !== 'string') throw new Error('aiws.sql(query, params?): query must be a string')
        const body = await bridge('sql', { query, params: params ?? null, declare: declared() })
        const columns = body.columns ?? []
        const rows = (body.rows ?? []).map((r) => Object.fromEntries(columns.map((c, i) => [c, r[i]])))
        return { columns, rows, truncated: Boolean(body.truncated) }
      },
      ai: {
        /**
         * One model inference (no tools, no loop): `{ system?, prompt | messages, schema?, model?, max_tokens? }` →
         * `{ text }`, or `{ value }` checked against `schema`. Cached by content: the same request gives the same answer.
         */
        async complete(req) {
          if (!req || typeof req !== 'object') throw new Error('aiws.ai.complete({ system?, prompt | messages, schema?, model?, max_tokens? })')
          if (typeof req.prompt !== 'string' && !Array.isArray(req.messages)) throw new Error('aiws.ai.complete: prompt (a string) or messages ([{ role, text }]) required')
          const body = await bridge('ai.complete', { system: req.system ?? null, prompt: req.prompt ?? null, messages: req.messages ?? null, schema: req.schema ?? null,
            model: req.model ?? null, max_tokens: req.max_tokens ?? null, declare: declared() })
          if (body.cached) host.aiStats.complete.cached += 1
          else host.aiStats.complete.calls += 1
          host.aiStats.complete.tokens += body.tokens ?? 0
          return req.schema ? { value: body.value } : { text: body.text ?? '' }
        },
      },
      zone: {
        /** A resource of this Zone (a path such as `/kapi/…`, or a URL of the Zone's hosts), with the executor's session. */
        async fetch(url, init = {}) {
          const body = await bridge('zone.fetch', { url: String(url), method: init.method ?? 'GET', headers: init.headers ?? {}, body: init.body ?? null, declare: declared() })
          if (body.url) host.external.add(String(body.url).split('?')[0])
          const bytes = () => (typeof body.base64 === 'string' ? Uint8Array.from(atob(body.base64), (c) => c.charCodeAt(0)) : new TextEncoder().encode(body.text ?? ''))
          return { ok: body.status >= 200 && body.status < 300, status: body.status, headers: body.headers ?? {}, text: async () => body.text ?? dec.decode(bytes()), json: async () => JSON.parse(body.text ?? dec.decode(bytes())), bytes: async () => bytes() }
        },
      },
      result: {
        /** A table: rows of `{ 字段: 值 }`, `key` (logical key columns), optional `fields` types. */
        table(name, rows, opts = {}) {
          if (!Array.isArray(rows)) throw new Error(`result.table(${name}): rows must be an array`)
          const declared = fieldList(opts.fields)
          const order = declared.map((f) => f.name)
          for (const r of rows) for (const k of Object.keys(r)) if (k !== '_id' && !order.includes(k)) order.push(k)
          const fields = order.map((n) => declared.find((f) => f.name === n) ?? { name: n })
          const key = opts.key === undefined ? undefined : Array.isArray(opts.key) ? opts.key : [opts.key]
          host.#push({ name, type: 'table', rows: rows.map((r) => { const o = { ...r }; delete o._id; return o }), fields, key, title: opts.title, views: opts.views })
        },
        /** Derived columns on a table input: `{ record _id: { 列: 值 } }`. */
        columns(name, inputName, valuesById, opts = {}) {
          const i = input(inputName)
          if (i.kind !== 'table') throw new Error(`result.columns(${name}): ${inputName} is not a table input`)
          const values = valuesById instanceof Map ? Object.fromEntries(valuesById) : valuesById
          host.#push({ name, type: 'columns', input: inputName, values, fields: fieldList(opts.fields), title: opts.title })
        },
        record(name, props, schema) {
          host.#push({ name, type: 'record', props, schema: schema ? fieldList(schema) : undefined, title: undefined })
        },
        text(name, markdown, opts = {}) {
          if (typeof markdown !== 'string') throw new Error(`result.text(${name}): markdown must be a string`)
          host.#push({ name, type: 'text', markdown, title: opts.title, views: opts.views })
        },
        file(name, path, mediaType, opts = {}) {
          host.#push({ name, type: 'file', path, media_type: mediaType, title: opts.title, views: opts.views })
        },
        html(name, spec) {
          host.#push({ name, type: 'html', html: spec.html, css: spec.css ?? '', js: spec.js ?? '', bindings: spec.bindings ?? {}, title: spec.title })
        },
        /**
         * A Block-control (catalog/controls.json): `kind`, `children` a Markdown outline or `[{ text, children }]`,
         * optional `config` and `title`; `under` (an input that is a control's node) puts the tree below that node.
         * A composite control of the Workspace: `def` (`<author_did>#<name>`), `slots: { <slot>: ["result:<n>" |
         * "input:<n>" | { view: 'richtext', markdown }] }`, optional `config`, `title`, `under: { input, slot }`.
         */
        control(name, spec = {}) {
          if (typeof spec.def === 'string') {
            if (!spec.slots || typeof spec.slots !== 'object') throw new Error(`result.control(${name}): slots is required with def`)
            if (spec.under !== undefined && (typeof spec.under !== 'object' || input(spec.under.input).kind !== 'control')) throw new Error(`result.control(${name}): under must be { input, slot } of a composite input`)
            host.#push({ name, type: 'control', def: spec.def, config: spec.config, slots: spec.slots, title: spec.title, under: spec.under })
            return
          }
          if (typeof spec.kind !== 'string') throw new Error(`result.control(${name}): kind is required (see catalog/controls.json)`)
          if (typeof spec.children !== 'string' && !Array.isArray(spec.children)) throw new Error(`result.control(${name}): children must be a Markdown outline or [{ text, children }]`)
          if (spec.under !== undefined && input(spec.under).kind !== 'control') throw new Error(`result.control(${name}): under must name a control input`)
          host.#push({ name, type: 'control', kind: spec.kind, config: spec.config, children: spec.children, title: spec.title, under: spec.under })
        },
      },
      /** Numbers (and other values) the written text may cite. */
      facts(obj) {
        if (!obj || typeof obj !== 'object') throw new Error('facts(obj) takes an object')
        Object.assign(host.factsObj, obj)
      },
      /** Report an acceptance check of the analysis (`id` from the contract). */
      check(id, passed, detail) {
        host.checks = host.checks.filter((c) => c.id !== id)
        host.checks.push({ id, passed: Boolean(passed), detail: detail === undefined ? null : detail })
      },
      llm: {
        /**
         * One model judgement per item, validated against `schema` (a JSON-schema subset: type, properties, required,
         * enum). Cached by (model, instruction, item): a re-run only asks for new or changed items.
         */
        async map(items, instruction, schema = { type: 'string' }, opts = {}) {
          if (!Array.isArray(items)) throw new Error('llm.map(items, instruction, schema): items must be an array')
          const body = await bridge('llm.map', { items, instruction, schema, batch: opts.batch ?? 20, declare: declared() })
          host.aiStats.map.calls += body.calls ?? 0
          host.aiStats.map.items += items.length
          host.aiStats.map.cached += body.cached ?? 0
          const failed = (body.results ?? []).filter((r) => r && r.error)
          if (failed.length && !opts.allowErrors) throw new Error(`llm.map: ${failed.length} of ${items.length} items failed (${failed[0].error}); pass { allowErrors: true } to receive them as { error }`)
          return (body.results ?? []).map((r) => (r && r.error ? r : r.value))
        },
      },
    }
  }

  /** Write `output/.aiws/results.json`. */
  flush(error) {
    const dir = join(this.root, 'output/.aiws')
    Deno.mkdirSync(dir, { recursive: true })
    const out = {
      results: this.results,
      facts: this.factsObj,
      checks: this.checks,
      external: [...this.external],
      ai: this.aiStats,
      declare: this.declared,
      error: error ? errorText(error) : null,
      error_code: error && typeof error.code === 'string' ? error.code : null,
    }
    Deno.writeTextFileSync(join(dir, 'results.json'), JSON.stringify(out))
    return out
  }

  /** What a person debugging sees on stdout. */
  summary() {
    const lines = []
    for (const r of this.results) {
      if (r.type === 'table') {
        lines.push(`table ${r.name}: ${r.rows.length} rows, fields ${r.fields.map((f) => f.name + (f.type ? ':' + f.type : '')).join(', ')}, key ${JSON.stringify(r.key ?? null)}`)
        for (const row of r.rows.slice(0, 3)) lines.push('  ' + JSON.stringify(row))
      } else if (r.type === 'columns') lines.push(`columns ${r.name} on ${r.input}: ${Object.keys(r.values ?? {}).length} records`)
      else if (r.type === 'text') lines.push(`text ${r.name}: ${r.markdown.length} chars`)
      else if (r.type === 'control' && r.def) lines.push(`control ${r.name}: ${r.def} (${Object.keys(r.slots || {}).join(', ')})`)
      else if (r.type === 'control') lines.push(`control ${r.name}: ${r.kind}${r.under ? ` under ${r.under}` : ''}`)
      else lines.push(`${r.type} ${r.name}`)
    }
    if (Object.keys(this.factsObj).length) lines.push('facts ' + JSON.stringify(this.factsObj))
    for (const c of this.checks) lines.push(`check ${c.id}: ${c.passed ? 'passed' : 'FAILED'}${c.detail !== null ? ' — ' + JSON.stringify(c.detail) : ''}`)
    if (this.declared) lines.push('declare ' + JSON.stringify(this.declared))
    else lines.push('declare: none (the macro must start with aiws.declare)')
    const { complete, map } = this.aiStats
    if (complete.calls + complete.cached + map.items) lines.push(`ai: complete ${complete.calls} calls (${complete.cached} cached, ${complete.tokens} tokens), map ${map.items} items (${map.calls} calls, ${map.cached} cached)`)
    if (this.external.size) lines.push('zone / external data: ' + [...this.external].join(', '))
    return lines.join('\n')
  }
}

export function createHost(root) {
  return new ProgramHost(root)
}

