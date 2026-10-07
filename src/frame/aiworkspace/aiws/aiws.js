// aiws v2 — the program host (许愿格详细设计 §9.4).
//
// A wish program reads the run's snapshot directory and writes candidate results; it never writes
// the Workspace. The same file serves the canonical run (`run_program`, "re-run the program") and
// the model's own debugging in the shell (`deno run -A lib/run.js`).
//
//   context/inputs.json            name → { entity_id, type_id, dir, selector, … }
//   context/entities/<id>/…        schema.json + rows.jsonl | content.md / content.json | meta.json
//   context/assets/<object id>     asset bytes
//   context/data-tree.json, block-tree.json, request.json
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

function compareValues(a, b) {
  if (a === b) return 0
  if (a === null || a === undefined) return 1
  if (b === null || b === undefined) return -1
  if (typeof a === 'number' && typeof b === 'number') return a - b
  return String(a).localeCompare(String(b), 'zh-CN')
}

class Input {
  #host
  #rows = null
  constructor(host, name, info) {
    this.#host = host
    this.name = name
    this.entity_id = info.entity_id
    this.type_id = info.type_id
    this.kind = (info.type_id || '').replace('buckyos.', '').replace('table-source', 'table').replace('asset-ref', 'asset').replace('container', 'folder')
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

  /** Field definitions in table order: `{ name, type, options?, id, visible }`. */
  fields() {
    this.#expect('table')
    return (readJson(join(this.dir, 'schema.json'), { fields: [] }).fields ?? []).map((f) => ({
      name: f.name, type: f.type, options: f.options ?? undefined, id: f.field_id, visible: f.visible !== false,
    }))
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
      rows.push(o)
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
    for (const f of opts.fields ?? []) if (!known.has(f)) throw new Error(`input ${this.name} has no field ${f} (fields: ${[...known].join(', ')})`)
    if (typeof opts.filter === 'function') rows = rows.filter(opts.filter)
    else if (opts.filter && typeof opts.filter === 'object') {
      const conds = Object.entries(opts.filter)
      for (const [k] of conds) if (!known.has(k)) throw new Error(`input ${this.name} has no field ${k}`)
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
    return rows.map((r) => ({ ...r }))
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
    this.dataTree = readJson(join(this.root, 'context/data-tree.json'), [])
    this.blockTree = readJson(join(this.root, 'context/block-tree.json'), [])
    this.results = []
    this.factsObj = {}
    this.checks = []
    this.external = new Set()
    this.llmStats = { calls: 0, items: 0, cached: 0 }
    this.hostUrl = Deno.env.get('AIWS_HOST') ?? null
    this.token = Deno.env.get('AIWS_TOKEN') ?? null
    this.#trackFetch()
    this.aiws = this.#api()
  }

  #trackFetch() {
    const original = globalThis.fetch
    if (!original || original.__aiws) return
    const host = this.hostUrl
    const external = this.external
    const wrapped = (input, init) => {
      const url = typeof input === 'string' ? input : input?.url ?? String(input)
      if (!host || !url.startsWith(host)) external.add(url.split('?')[0])
      return original(input, init)
    }
    wrapped.__aiws = true
    globalThis.fetch = wrapped
  }

  #push(result) {
    checkName(result.name)
    if (this.results.some((r) => r.name === result.name)) throw new Error(`result ${result.name} was written twice`)
    this.results.push(result)
  }

  #api() {
    const host = this
    const input = (name) => {
      const info = host.inputs[name]
      if (!info) throw new Error(`no input named ${JSON.stringify(name)} (inputs: ${Object.keys(host.inputs).join(', ') || 'none'})`)
      return new Input(host, name, info)
    }
    const find = (text) => {
      const t = String(text).toLowerCase()
      return [...host.dataTree, ...host.blockTree].filter((e) => String(e.title ?? '').toLowerCase().includes(t) || String(e.path ?? '').toLowerCase().includes(t))
    }
    return {
      version: 2,
      host: 'program',
      request: host.request,
      input,
      inputs: () => Object.keys(host.inputs),
      resolve: (pathOrName) => {
        const hits = host.dataTree.filter((e) => e.path === pathOrName || e.title === pathOrName || e.entity_id === pathOrName || e.handle === pathOrName)
        const names = Object.entries(host.inputs).filter(([, i]) => hits.some((h) => h.entity_id === i.entity_id)).map(([n]) => n)
        return hits.map((h) => ({ ...h, input: names.find((n) => host.inputs[n].entity_id === h.entity_id) ?? null }))
      },
      outline: (target, depth = 1) => {
        const root = target ? host.dataTree.find((e) => e.handle === target || e.entity_id === target || e.path === target) : null
        const parent = root ? root.handle : '@F0'
        const out = []
        const walk = (p, d) => {
          for (const e of host.dataTree.filter((x) => x.parent === p)) {
            out.push({ ...e, depth: d })
            if (d + 1 < depth) walk(e.handle, d + 1)
          }
        }
        if (root) walk(parent, 0)
        else out.push(...host.dataTree.filter((e) => !host.dataTree.some((x) => x.handle === e.parent)))
        return out
      },
      find,
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
         * One model judgement per item, validated against `schema` (a JSON-schema subset: type,
         * properties, required, enum). Cached by (model, instruction, item): a re-run only asks for
         * new or changed items.
         */
        async map(items, instruction, schema = { type: 'string' }, opts = {}) {
          if (!Array.isArray(items)) throw new Error('llm.map(items, instruction, schema): items must be an array')
          if (!host.hostUrl) throw new Error('llm.map is only available inside a wish run (AIWS_HOST is not set)')
          const res = await fetch(`${host.hostUrl}/llm_map`, {
            method: 'POST',
            headers: { 'content-type': 'application/json', 'x-aiws-token': host.token ?? '' },
            body: JSON.stringify({ items, instruction, schema, batch: opts.batch ?? 20 }),
          })
          const body = await res.json().catch(() => ({ error: `llm.map: HTTP ${res.status}` }))
          if (!res.ok || body.error) throw new Error(body.error ?? `llm.map failed: HTTP ${res.status}`)
          host.llmStats.calls += body.calls ?? 0
          host.llmStats.items += items.length
          host.llmStats.cached += body.cached ?? 0
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
      llm_map: this.llmStats,
      error: error ? String(error?.stack ?? error) : null,
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
      else lines.push(`${r.type} ${r.name}`)
    }
    if (Object.keys(this.factsObj).length) lines.push('facts ' + JSON.stringify(this.factsObj))
    for (const c of this.checks) lines.push(`check ${c.id}: ${c.passed ? 'passed' : 'FAILED'}${c.detail !== null ? ' — ' + JSON.stringify(c.detail) : ''}`)
    if (this.external.size) lines.push('external data: ' + [...this.external].join(', '))
    return lines.join('\n')
  }
}

export function createHost(root) {
  return new ProgramHost(root)
}

