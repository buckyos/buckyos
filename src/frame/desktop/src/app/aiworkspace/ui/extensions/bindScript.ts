/* An HTML control that decides what it binds in its own script (控件 on_bind TODO §2.9, host API 5). Before the control is
 * placed or re-bound, the host runs its behavior.js without a page — in a dedicated Worker, with
 * `aiws.context.role === 'bind'` — and calls the handlers it registered there:
 *
 *   aiws.on('bind', ctx => ({ bindings, create? }) | null)
 *   aiws.on('init_control', ctx => true | 'why not')
 *
 * ctx is plain data (reason, no_ui, node, title, decls, given / bound, parent, children) plus what the host does for the
 * script: find / read the Workspace's data, run the default rule for some bindings (`defaults`), and open the host's
 * dialogs (with `no_ui` they fail with NEEDS_UI). An Agent's `buckyos aiws place` does not run the script: its given
 * bindings are taken as they are (owner 11-12). The Worker is no security boundary (D16); it keeps the page out of reach
 * so the script's binding logic needs no page. */

import { BindFailure, defaultOnBind, STANDARD_SCHEMAS, type BindCtx, type InitCtx, type OnBind, type OnInit } from 'buckyos'
import type { Json } from '../../api/types'
import { aiwsT, currentAiwsLocale } from '../../i18n'
import type { WorkspaceStore } from '../../state/store'

const LOAD_TIMEOUT_MS = 8000
/** How long the script may run without asking the host anything (time a person spends in a dialog does not count). */
const IDLE_TIMEOUT_MS = 20_000
const CODES = new Set(['NEEDS_UI', 'NO_DATA', 'NOT_READY', 'CANCELLED', 'INVALID'])

/** The Worker: `aiws` for the bind role, the definition's script loaded with importScripts, the handlers called. */
const WORKER = `
var seq = 0, waiting = {}, handlers = {};
function send(m) { postMessage(m); }
function ask(what, args) {
  return new Promise(function (resolve, reject) {
    var id = ++seq; waiting[id] = { resolve: resolve, reject: reject };
    send({ t: 'ask', id: id, what: what, args: args === undefined ? null : args });
  });
}
function hostError(e) { var err = new Error(e.message); err.code = e.code || null; err.binding = e.binding || null; return err; }
function dialogs() {
  return {
    pickOne: function (o) { return ask('pickOne', o || {}); },
    pickMany: function (o) { return ask('pickMany', o || {}); },
    confirm: function (o) { return ask('confirm', o || {}); }
  };
}
function bindCtx(p) {
  var ctx = Object.assign({}, p);
  ctx.find = function (q) { return ask('find', q || {}); };
  ctx.read = function (id) { return ask('read', { entity_id: String(id) }); };
  ctx.canCreate = function (b) { var d = typeof b === 'string' ? (p.decls[b] || {}) : (b || {}); return Boolean(d.schema_id && p.standard_schemas.indexOf(d.schema_id) >= 0); };
  ctx.defaults = function (names) { return ask('defaults', { names: names || null }); };
  ctx.dialogs = dialogs();
  return ctx;
}
self.onmessage = function (event) {
  var m = event.data || {};
  if (m.t === 'start') {
    self.aiws = { version: m.version, host: 'bind', locale: m.locale, context: m.context, bindings: {},
      on: function (name, fn) { handlers[String(name)] = fn; }, ready: function () {}, notify: function () {} };
    try { importScripts(m.script); } catch (e) { send({ t: 'crash', message: String((e && e.message) || e) }); return; }
    send({ t: 'loaded', handlers: Object.keys(handlers) });
  } else if (m.t === 'answer') {
    var w = waiting[m.id]; if (!w) return; delete waiting[m.id];
    if (m.error) w.reject(hostError(m.error)); else w.resolve(m.value);
  } else if (m.t === 'call') {
    var ctx = m.name === 'bind' ? bindCtx(m.payload) : Object.assign({}, m.payload, { dialogs: dialogs() });
    Promise.resolve().then(function () { return handlers[m.name](ctx); }).then(function (v) {
      try { send(v === undefined ? { t: 'result', id: m.id, none: true } : { t: 'result', id: m.id, value: v }); }
      catch (e) { send({ t: 'result', id: m.id, error: { message: 'the result is not plain data: ' + String((e && e.message) || e) } }); }
    }, function (e) {
      send({ t: 'result', id: m.id, error: { message: String((e && e.message) || e), code: (e && e.code) || null, binding: (e && e.binding) || null } });
    });
  }
};
`

export interface BindScriptSource { defId: string; title: string; js: string; apiVersion: number; config: Record<string, Json> }

type FromWorker =
  | { t: 'loaded'; handlers: string[] }
  | { t: 'crash'; message: string }
  | { t: 'ask'; id: number; what: string; args: Record<string, Json> | null }
  | { t: 'result'; id: number; value?: Json; none?: true; error?: { message: string; code?: string | null; binding?: string | null } }

interface Running { id: number; ctx: BindCtx | InitCtx; resolve: (v: Json | undefined) => void; reject: (e: Error) => void }

export class BindScript {
  private worker: Worker | null = null
  private urls: string[] = []
  private loading: Promise<Set<string>> | null = null
  private loaded: ((handlers: Set<string>) => void) | null = null
  private failLoad: ((error: Error) => void) | null = null
  private running: Running | null = null
  /** Why the Worker stopped (a failure, a timeout, disposed): later calls fail the same way. */
  private dead: Error | null = null
  private idle: number | null = null
  private seq = 0
  private readonly store: WorkspaceStore
  private readonly source: BindScriptSource

  constructor(store: WorkspaceStore, source: BindScriptSource) {
    this.store = store
    this.source = source
  }

  readonly onBind: OnBind = async (ctx) => {
    const payload = { reason: ctx.reason, no_ui: ctx.no_ui, node: ctx.node, title: ctx.title, decls: ctx.decls, given: ctx.given, parent: ctx.parent, standard_schemas: STANDARD_SCHEMAS } as unknown as Json
    const v = await this.call('bind', payload, ctx)
    if (v === null) return null
    if (v === undefined || typeof v !== 'object' || Array.isArray(v)) throw this.returned('bind', v === undefined ? 'nothing' : JSON.stringify(v))
    return v as never
  }

  readonly onInit: OnInit = async (ctx) => {
    const payload = { reason: ctx.reason, no_ui: ctx.no_ui, node: ctx.node, title: ctx.title, decls: ctx.decls, bound: ctx.bound, children: ctx.children } as unknown as Json
    const v = await this.call('init_control', payload, ctx)
    if (v === true || typeof v === 'string') return v
    throw this.returned('init_control', v === undefined ? 'nothing' : JSON.stringify(v))
  }

  dispose() {
    this.fail(new BindFailure(aiwsT('aiworkspace.extension.unloaded'), 'CANCELLED'))
    this.worker?.terminate()
    this.worker = null
    for (const url of this.urls.splice(0)) URL.revokeObjectURL(url)
  }

  private returned(event: string, what: string) {
    return new BindFailure(aiwsT('aiworkspace.datactl.source.scriptReturn', { name: this.source.title, event, what: what.slice(0, 120) }), 'INVALID')
  }

  private failed(reason: string) {
    return new BindFailure(aiwsT('aiworkspace.datactl.source.scriptFailed', { name: this.source.title, reason }), 'INVALID')
  }

  private load(): Promise<Set<string>> {
    this.loading ??= new Promise<Set<string>>((resolve, reject) => {
      this.loaded = resolve
      this.failLoad = reject
      const url = (text: string) => { const u = URL.createObjectURL(new Blob([text], { type: 'text/javascript' })); this.urls.push(u); return u }
      const worker = new Worker(url(WORKER), { name: `aiworkspace-bind:${this.source.defId}` })
      this.worker = worker
      worker.onmessage = (event: MessageEvent<FromWorker>) => this.onMessage(event.data)
      worker.onerror = (event) => { event.preventDefault(); this.fail(this.failed(event.message || 'error')) }
      this.restartIdle(LOAD_TIMEOUT_MS)
      worker.postMessage({
        t: 'start', script: url(this.source.js), version: this.source.apiVersion, locale: currentAiwsLocale().replace(/[^\w-]/g, ''),
        context: { role: 'bind', definition: { entity_id: this.source.defId, title: this.source.title }, config: this.source.config },
      })
    })
    return this.loading
  }

  private async call(name: 'bind' | 'init_control', payload: Json, ctx: BindCtx | InitCtx): Promise<Json | undefined> {
    if (this.dead) throw this.dead
    const handlers = await this.load()
    if (this.dead) throw this.dead
    if (!handlers.has(name)) throw new BindFailure(aiwsT('aiworkspace.datactl.source.scriptNoHandler', { name: this.source.title, event: name }), 'INVALID')
    return new Promise<Json | undefined>((resolve, reject) => {
      const id = ++this.seq
      this.running = { id, ctx, resolve, reject }
      this.restartIdle(IDLE_TIMEOUT_MS)
      this.worker?.postMessage({ t: 'call', id, name, payload })
    })
  }

  private restartIdle(ms: number) {
    this.stopIdle()
    this.idle = window.setTimeout(() => this.fail(new BindFailure(aiwsT('aiworkspace.datactl.source.scriptTimeout', { name: this.source.title, seconds: Math.round(ms / 1000) }), 'INVALID')), ms)
  }

  private stopIdle() {
    if (this.idle !== null) window.clearTimeout(this.idle)
    this.idle = null
  }

  /** Whatever is waiting fails; the Worker is not used again. */
  private fail(error: Error) {
    this.dead ??= error
    this.stopIdle()
    this.failLoad?.(error)
    this.failLoad = null
    this.loaded = null
    const running = this.running
    this.running = null
    running?.reject(error)
    if (this.worker) { this.worker.terminate(); this.worker = null }
  }

  private onMessage(m: FromWorker) {
    if (m.t === 'loaded') {
      this.stopIdle()
      this.loaded?.(new Set(m.handlers))
      this.loaded = null
      this.failLoad = null
    } else if (m.t === 'crash') {
      this.fail(this.failed(m.message))
    } else if (m.t === 'result') {
      const running = this.running
      if (!running || running.id !== m.id) return
      this.stopIdle()
      this.running = null
      if (m.error) {
        const code = m.error.code && CODES.has(m.error.code) ? m.error.code as BindFailure['code'] : null
        running.reject(code ? new BindFailure(m.error.message, code, undefined, m.error.binding ?? undefined) : this.failed(m.error.message))
      } else running.resolve(m.none ? undefined : m.value ?? null)
    } else if (m.t === 'ask') {
      const running = this.running
      if (!running) return
      // a person may take their time in a dialog: the script is not idle while the host works for it
      this.stopIdle()
      void this.answer(running.ctx, m.what, m.args ?? {}).then(
        (value) => { if (this.running === running) { this.restartIdle(IDLE_TIMEOUT_MS); this.worker?.postMessage({ t: 'answer', id: m.id, value }) } },
        (error: unknown) => {
          if (this.running !== running) return
          this.restartIdle(IDLE_TIMEOUT_MS)
          const e = error instanceof BindFailure ? { message: error.message, code: error.code, binding: error.binding ?? null } : { message: error instanceof Error ? error.message : String(error) }
          this.worker?.postMessage({ t: 'answer', id: m.id, error: e })
        },
      )
    }
  }

  /** What the host does for the script. */
  private async answer(ctx: BindCtx | InitCtx, what: string, args: Record<string, Json>): Promise<Json> {
    const bind = 'find' in ctx ? ctx : null
    switch (what) {
      case 'pickOne': return await ctx.dialogs.pickOne(args as never) as Json
      case 'pickMany': return await ctx.dialogs.pickMany(args as never) as Json
      case 'confirm': return await ctx.dialogs.confirm(args as never)
      case 'find': if (bind) return bind.find(args as never) as unknown as Json; break
      case 'read': if (bind) return await this.store.session.read<Json>(String(args.entity_id ?? '')) as unknown as Json; break
      case 'defaults': {
        if (!bind) break
        const names = Array.isArray(args.names) ? args.names.map(String) : Object.keys(bind.decls)
        const decls = Object.fromEntries(Object.entries(bind.decls).filter(([n]) => names.includes(n)))
        return await defaultOnBind({ ...bind, decls }) as unknown as Json
      }
      default:
    }
    throw new Error(aiwsT('aiworkspace.extension.unknownCall', { type: what }))
  }
}
