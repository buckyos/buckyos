/* HTML extension runtime (phase two §10.4, D16; `aiws` v2 of 许愿格 §9.4): an HTML Block definition
 * runs same-origin in an iframe — the iframe is for style and crash isolation and mount management,
 * not a security boundary (the Owner is the trust root). The host offers a versioned JS API
 * (`window.aiws`):
 *
 *   v1 (still available): context, read, submit (raw operations), upload, snapshot, notify, on (`query` of v1–v5: gone, see v6)
 *   v2: bindings, input(name).rows/fields/markdown/props/bytes, resolve/outline/find, request,
 *       watch(name, cb), table(name).upsert/setColumn, record(name).set, text(name).setMarkdown,
 *       createBlock, batch(fn)
 *   v3 (扩展控件 P2/P3): emit(name, params), context.interactive, createBlock({ parent, slot }); a
 *       definition of version 2 runs as before (`aiws.version` is the version it declares)
 *   v4 (控件机制 TODO H1, H4–H6): onConfig(cb) — the instance's properties as they change (also in
 *       `context.config`); serverNow() — the service's clock; userState.get(key) / set(key, value) — this
 *       person's own state of this Block (`block:<id>/<key>`); mediaUrl(binding, member?) — a Range-playable
 *       address of a bound audio / video asset (or a member of a bound directory)
 *   v5 (控件 on_bind TODO §2.9): nothing new on the page; a definition declaring `on_bind` / `on_init_control` "script"
 *       has its script run once more without a page before it is placed or re-bound (bindScript.ts,
 *       `aiws.context.role === 'bind'`) for its aiws.on('bind') / aiws.on('init_control')
 *   v6 (核心 API 调整 R17): query(sql, params?) — one SQL SELECT over the Workspace's tables, run by the service
 *       (`doc.query { sql }`; a table is named by its path, `FROM "/data/销售"`); 5 and older have no query: their
 *       `aiws.query` rejects. Reading a binding (`input(name).rows/fields`) needs no SQL and works offline
 *   locale: the host's language when the iframe starts ('en', 'zh-CN', …; also the document's `lang`),
 *       so an extension can show its own text in it; a language the extension has no text for should
 *       read English, as the host does.
 *
 * Every write goes through WorkspaceStore and therefore through permissions, locks, undo and save
 * states. Errors, crashes and silence are localised: a call that does not answer in time rejects,
 * and the Block falls back to its static snapshot. */

import type { Json, Operation } from '../../api/types'
import { aiwsT, currentAiwsLocale } from '../../i18n'

/** The host API versions this host runs (Q19: newer versions only add members); the newest is what it speaks. */
export const HTML_API_VERSIONS = [2, 3, 4, 5, 6] as const
export const HTML_API_VERSION = 6
/** A definition that names no version was written for API 2. */
export const HTML_API_DEFAULT = 2
/** The version an HTML definition runs on, or null when this host is older than it (Q33: shown as its snapshot). */
export function htmlApiOf(source: { api_version?: number } | undefined): number | null {
  const v = source?.api_version ?? HTML_API_DEFAULT
  return (HTML_API_VERSIONS as readonly number[]).includes(v) ? v : null
}
const READY_TIMEOUT_MS = 8000
const CALL_TIMEOUT_MS = 20_000

/** What the host does for the extension; every method runs with the host's checks. */
export interface HostBridge {
  context: () => Json
  read: (entityId: string, selector?: Json) => Promise<Json>
  query: (params: Json) => Promise<Json>
  submit: (operations: Operation[], label: string) => Promise<Json>
  upload: (bytes: Uint8Array, fileName: string, mediaType?: string) => Promise<{ object_id: string; media_type: string; size: number }>
  snapshot: (dataUrl: string) => Promise<void>
  notify: (text: string) => void
  /** v2: reads by binding name (`rows`, `fields`, `markdown`, `props`, `bytes`) and the outline. */
  input?: (name: string, what: string, args: Json) => Promise<Json>
  outline?: (what: 'resolve' | 'outline' | 'find', args: Json) => Promise<Json>
  /** v2: typed writes (`upsert`, `setColumn`, `recordSet`, `setMarkdown`, `createBlock`); inside a batch they are collected. */
  write?: (kind: string, name: string, args: Json, batch: string | null) => Promise<Json>
  batch?: (action: 'begin' | 'end' | 'abort', id: string | null, label?: string) => Promise<Json>
  /** v2: call `notify(name)` whenever the data bound as `name` changes; returns the unsubscribe. */
  watch?: (name: string, notify: () => void) => (() => void) | null
  /** v2 (P1, additive): a file the definition lists in `assets`, from its source snapshot. */
  asset?: (name: string) => Promise<{ bytes: Uint8Array; media_type: string }>
  /** v3: raise a declared event (the host runs the instance's handlers in run mode). */
  emit?: (name: string, params: Record<string, Json>) => Promise<{ handled: boolean; event_id: string | null }>
  /** v4: this person's own state of the Block (H4). */
  userState?: (action: 'get' | 'set', key: string, value: Json | null) => Promise<Json>
  /** v4: a Range-playable address of a bound medium (H6). */
  mediaUrl?: (binding: string, member: string | null) => Promise<Json>
}

export interface HtmlSource { html: string; css?: string; js?: string; api_version?: number }

type Pending = { resolve: (value: Json) => void; reject: (error: Error) => void; timer: number }

/** The script that becomes `window.aiws` inside the iframe. Kept in a string so a definition ships as data. */
function bootstrap(nonce: string, locale: string, version: number): string {
  return `
(function(){
  var seq = 0, waiting = {}, handlers = {}, watchers = {}, currentBatch = null, assets = {}, configWatchers = [], clockOffset = ${JSON.stringify(0)};
  var nonce = ${JSON.stringify(nonce)};
  var noHandler = ${JSON.stringify(aiwsT('aiworkspace.extension.noHandler'))};
  function send(type, payload, id){ parent.postMessage({ aiws: nonce, type: type, id: id, payload: payload }, '*'); }
  function call(type, payload){
    return new Promise(function(resolve, reject){
      var id = ++seq; waiting[id] = { resolve: resolve, reject: reject };
      send(type, payload, id);
    });
  }
  window.addEventListener('message', function(event){
    var m = event.data; if (!m || m.aiws !== nonce) return;
    if (m.type === 'result') { var w = waiting[m.id]; if (!w) return; delete waiting[m.id]; if (m.error) w.reject(new Error(m.error)); else w.resolve(m.payload); return; }
    if (m.type === 'event') { var list = watchers[m.payload && m.payload.name] || []; list.forEach(function(cb){ try { cb(m.payload); } catch (e) { setTimeout(function(){ throw e; }, 0); } }); return; }
    if (m.type === 'config') { context.config = m.payload || {}; configWatchers.forEach(function(cb){ try { cb(context.config); } catch (e) { setTimeout(function(){ throw e; }, 0); } }); return; }
    if (m.type === 'clock') { clockOffset = Number(m.payload && m.payload.offset) || 0; return; }
    if (m.type === 'request') {
      var h = handlers[m.name];
      if (!h) { send('response', { error: noHandler.replace('{{name}}', function(){ return m.name; }) }, m.id); return; }
      Promise.resolve().then(function(){ return h(m.payload); }).then(function(r){ send('response', { value: r }, m.id); }, function(e){ send('response', { error: String(e && e.message || e) }, m.id); });
    }
  });
  function input(name){
    function q(what, args){ return call('input', { name: name, what: what, args: args || null }); }
    return {
      name: name,
      rows: function(opts){ var o = opts || {}; var f = o.filter; if (typeof f === 'function') { var fn = f; o = Object.assign({}, o, { filter: null }); return q('rows', o).then(function(rows){ return rows.filter(fn); }); } return q('rows', o); },
      fields: function(){ return q('fields'); },
      markdown: function(){ return q('markdown'); },
      props: function(){ return q('props'); },
      bytes: function(){ return q('bytes').then(function(a){ return new Uint8Array(a); }); },
      text: function(){ return q('bytes').then(function(a){ return new TextDecoder().decode(new Uint8Array(a)); }); }
    };
  }
  function write(kind, name, args){ return call('write', { kind: kind, name: name, args: args || null, batch: currentBatch }); }
  var context = ${JSON.stringify('__CONTEXT__')};
  window.aiws = {
    version: ${version},
    host: 'block',
    locale: ${JSON.stringify(locale)},
    context: context,
    request: context,
    bindings: context.bindings || {},
    ready: function(){ send('ready', null); },
    read: function(entityId, selector){ return call('read', { entity_id: entityId, selector: selector || null }); },
    query: function(sql, params){
      if (${version} < 6 || typeof sql !== 'string') return Promise.reject(new Error('aiws.query(sql, params?) is host API 6: one SQL SELECT (declare host_api 6)'));
      return call('query', { sql: sql, params: params || null });
    },
    submit: function(operations, label){ return call('submit', { operations: operations, label: label || '' }); },
    upload: function(data, fileName, mediaType){
      var bytes = typeof data === 'string' ? new TextEncoder().encode(data) : (data instanceof ArrayBuffer ? new Uint8Array(data) : data);
      return call('upload', { bytes: Array.from(bytes), file_name: fileName || 'file', media_type: mediaType || null });
    },
    snapshot: function(dataUrl){ return call('snapshot', { data_url: dataUrl }); },
    asset: function(name){
      name = String(name);
      if (!assets[name]) assets[name] = call('asset', { name: name }).then(function(r){ return URL.createObjectURL(new Blob([new Uint8Array(r.bytes)], { type: r.media_type || '' })); });
      return assets[name];
    },
    notify: function(text){ send('notify', { text: String(text) }); },
    on: function(name, handler){ handlers[name] = handler; },
    input: input,
    inputs: function(){ return Object.keys(context.bindings || {}); },
    resolve: function(pathOrName){ return call('outline', { what: 'resolve', args: { target: pathOrName } }); },
    outline: function(target, depth){ return call('outline', { what: 'outline', args: { target: target || null, depth: depth || 1 } }); },
    find: function(text){ return call('outline', { what: 'find', args: { text: text } }); },
    watch: function(name, cb){
      (watchers[name] = watchers[name] || []).push(cb);
      call('watch', { name: name });
      return function(){ watchers[name] = (watchers[name] || []).filter(function(x){ return x !== cb; }); };
    },
    table: function(name){ return {
      upsert: function(rows, opts){ return write('upsert', name, { rows: rows, key: (opts && opts.key) || null }); },
      setColumn: function(field, valuesById){ return write('setColumn', name, { field: field, values: valuesById }); }
    }; },
    record: function(name){ return { set: function(props){ return write('recordSet', name, { props: props }); } }; },
    text: function(name){ return { setMarkdown: function(md){ return write('setMarkdown', name, { markdown: String(md) }); } }; },
    createBlock: function(spec){ return write('createBlock', '', spec || {}); },
    ${version >= 3 ? `emit: function(name, params){ return call('emit', { name: String(name), params: params || {} }); },` : ''}
    ${version >= 4 ? `onConfig: function(cb){ configWatchers.push(cb); return function(){ configWatchers = configWatchers.filter(function(x){ return x !== cb; }); }; },
    serverNow: function(){ return Date.now() + clockOffset; },
    userState: {
      get: function(key){ return call('userState', { action: 'get', key: String(key), value: null }); },
      set: function(key, value){ return call('userState', { action: 'set', key: String(key), value: value === undefined ? null : value }); }
    },
    mediaUrl: function(binding, member){ return call('mediaUrl', { binding: String(binding), member: member == null ? null : String(member) }); },` : ''}
    batch: function(fn, label){
      return call('batch', { action: 'begin', id: null, label: label || '' }).then(function(id){
        var prev = currentBatch; currentBatch = id;
        return Promise.resolve().then(fn).then(
          function(){ currentBatch = prev; return call('batch', { action: 'end', id: id, label: label || '' }); },
          function(e){ currentBatch = prev; return call('batch', { action: 'abort', id: id }).then(function(){ throw e; }); });
      });
    }
  };
  window.addEventListener('pagehide', function(){ Object.keys(assets).forEach(function(k){ assets[k].then(function(u){ URL.revokeObjectURL(u); }, function(){}); }); });
  window.addEventListener('error', function(event){ send('crash', { message: String(event.message || 'error') }); });
  window.addEventListener('unhandledrejection', function(event){ send('crash', { message: String(event.reason && event.reason.message || event.reason || 'rejection') }); });
})();`
}

export class HtmlRuntime {
  readonly nonce = Math.random().toString(36).slice(2)
  private frame: HTMLIFrameElement | null = null
  private readonly pending = new Map<number, Pending>()
  private seq = 0
  private readyResolve: (() => void) | null = null
  private readyReject: ((error: Error) => void) | null = null
  private readyTimer: number | null = null
  private readonly readyPromise: Promise<void>
  private disposed = false
  private readonly bridge: HostBridge
  private readonly source: HtmlSource
  private readonly listener = (event: MessageEvent) => this.onMessage(event)
  private readonly unwatch: (() => void)[] = []
  private readonly version: number
  /** A pointer or key gesture happened inside the mounted Block: until then `emit` is refused (§6.7). */
  private gestured = false
  /** Called when the extension reports an uncaught error. */
  onCrash: ((message: string) => void) | null = null

  constructor(source: HtmlSource, bridge: HostBridge) {
    const version = htmlApiOf(source)
    if (version === null) throw new Error(aiwsT('aiworkspace.extension.unsupportedApi', { version: String(source.api_version) }))
    this.version = version
    this.source = source
    this.bridge = bridge
    this.readyPromise = new Promise((resolve, reject) => { this.readyResolve = resolve; this.readyReject = reject })
    void this.readyPromise.catch(() => undefined)
  }

  /** Build the document: markup + style + bootstrap + the definition's script, in the host's current language. */
  private document(): string {
    const locale = currentAiwsLocale().replace(/[^\w-]/g, '')
    const context = JSON.stringify(this.bridge.context()).replace(/<\//g, '<\\/')
    const boot = bootstrap(this.nonce, locale, this.version).replace(JSON.stringify('__CONTEXT__'), context)
    const css = this.source.css ? `<style>${this.source.css}</style>` : ''
    const js = this.source.js ? `<script>${this.source.js.replace(/<\/script/gi, '<\\/script')}</script>` : ''
    return `<!doctype html><html lang="${locale}"><head><meta charset="utf-8">${css}<script>${boot}</script></head><body>${this.source.html}${js}</body></html>`
  }

  /** Mount into `container` (visible) or into the body hidden (headless executor). */
  mount(container: HTMLElement | null): Promise<void> {
    if (this.disposed) return Promise.reject(new Error(aiwsT('aiworkspace.extension.unloaded')))
    if (this.frame) return this.readyPromise
    const frame = document.createElement('iframe')
    frame.className = 'aiws-html-frame'
    frame.setAttribute('title', aiwsT('aiworkspace.extension.html.title'))
    if (!container) { frame.style.position = 'fixed'; frame.style.width = '1px'; frame.style.height = '1px'; frame.style.opacity = '0'; frame.style.pointerEvents = 'none'; frame.setAttribute('aria-hidden', 'true') }
    window.addEventListener('message', this.listener)
    // the host watches the same-origin document for a real gesture (an extension cannot fake a trusted event)
    const gesture = (event: Event) => { if (event.isTrusted) this.gestured = true }
    frame.addEventListener('load', () => {
      const inner = frame.contentWindow
      inner?.addEventListener('pointerdown', gesture, true)
      inner?.addEventListener('keydown', gesture, true)
    })
    frame.srcdoc = this.document()
    ;(container ?? document.body).appendChild(frame)
    this.frame = frame
    this.readyTimer = window.setTimeout(() => this.failAll(new Error(aiwsT('aiworkspace.extension.readyTimeout', { seconds: READY_TIMEOUT_MS / 1000 }))), READY_TIMEOUT_MS)
    return this.readyPromise
  }

  get ready(): Promise<void> { return this.readyPromise }
  /** False once disposed: a late call of an unloaded runtime (an old build's `emit`) is dropped. */
  get alive(): boolean { return !this.disposed }

  private failAll(error: Error) {
    this.rejectWaiting(error)
    this.dispose()
    this.onCrash?.(error.message)
  }

  private rejectWaiting(error: Error) {
    this.readyReject?.(error)
    this.readyResolve = null
    this.readyReject = null
    if (this.readyTimer !== null) window.clearTimeout(this.readyTimer)
    this.readyTimer = null
    for (const [, waiter] of this.pending) { window.clearTimeout(waiter.timer); waiter.reject(error) }
    this.pending.clear()
  }

  /** v4 (H1): the instance's properties changed — the running extension hears it (`aiws.onConfig`). */
  pushConfig(config: Json) {
    if (this.version >= 4) this.post('config', null, config)
  }

  /** v4 (H5): the offset of the service's clock from this device's (`aiws.serverNow()`). */
  pushClock(offsetMs: number) {
    if (this.version >= 4) this.post('clock', null, { offset: offsetMs })
  }

  /** Ask the extension to handle `name` (e.g. `analyze`, `execute`, `render`). */
  request(name: string, payload: Json, timeoutMs = CALL_TIMEOUT_MS): Promise<Json> {
    if (this.disposed) return Promise.reject(new Error(aiwsT('aiworkspace.extension.unloaded')))
    return this.readyPromise.then(() => new Promise<Json>((resolve, reject) => {
      if (this.disposed) { reject(new Error(aiwsT('aiworkspace.extension.unloaded'))); return }
      const id = ++this.seq
      const timer = window.setTimeout(() => this.failAll(new Error(aiwsT('aiworkspace.extension.requestTimeout', { name, seconds: Math.round(timeoutMs / 1000) }))), timeoutMs)
      this.pending.set(id, { resolve, reject, timer })
      this.frame?.contentWindow?.postMessage({ aiws: this.nonce, type: 'request', id, name, payload }, '*')
    }))
  }

  private post(type: string, id: number | null, payload: Json | null, error?: string) {
    this.frame?.contentWindow?.postMessage({ aiws: this.nonce, type, id, payload, error }, '*')
  }

  private onMessage(event: MessageEvent) {
    const message = event.data as { aiws?: string; type?: string; id?: number; name?: string; payload?: Json; error?: string } | null
    if (!message || message.aiws !== this.nonce || event.source !== this.frame?.contentWindow) return
    switch (message.type) {
      case 'ready':
        if (this.readyResolve) {
          this.readyResolve(); this.readyResolve = null; this.readyReject = null
          if (this.readyTimer !== null) window.clearTimeout(this.readyTimer)
          this.readyTimer = null
        }
        return
      case 'response': {
        const waiter = message.id !== undefined ? this.pending.get(message.id) : undefined
        if (!waiter || message.id === undefined) return
        this.pending.delete(message.id)
        window.clearTimeout(waiter.timer)
        const body = (message.payload ?? {}) as { value?: Json; error?: string }
        if (body.error) waiter.reject(new Error(body.error))
        else waiter.resolve(body.value ?? null)
        return
      }
      case 'crash':
        this.failAll(new Error(String((message.payload as { message?: string } | undefined)?.message ?? 'error')))
        return
      case 'notify':
        this.bridge.notify(String((message.payload as { text?: string } | undefined)?.text ?? ''))
        return
      case 'read': case 'query': case 'submit': case 'upload': case 'snapshot': case 'input': case 'outline': case 'write': case 'batch': case 'watch': case 'asset': case 'emit': case 'userState': case 'mediaUrl': {
        const id = message.id ?? 0
        void this.handle(message.type, message.payload ?? null).then(
          (value) => this.post('result', id, value),
          (error: unknown) => this.post('result', id, null, error instanceof Error ? error.message : String(error)),
        )
        return
      }
      default:
    }
  }

  private async handle(type: string, payload: Json): Promise<Json> {
    const p = (payload ?? {}) as Record<string, Json>
    const need = <T>(f: T | undefined): T => { if (!f) throw new Error(aiwsT('aiworkspace.extension.notProvided', { type })); return f }
    switch (type) {
      case 'read': return this.bridge.read(String(p.entity_id ?? ''), p.selector ?? undefined)
      case 'query': return this.bridge.query(payload)
      case 'submit': return this.bridge.submit((p.operations as unknown as Operation[]) ?? [], String(p.label ?? ''))
      case 'upload': {
        const bytes = new Uint8Array(((p.bytes as number[]) ?? []).map((n) => n & 255))
        return this.bridge.upload(bytes, String(p.file_name ?? 'file'), typeof p.media_type === 'string' ? p.media_type : undefined) as unknown as Json
      }
      case 'snapshot':
        await this.bridge.snapshot(String(p.data_url ?? ''))
        return null
      case 'input': return need(this.bridge.input)(String(p.name ?? ''), String(p.what ?? ''), p.args ?? null)
      case 'outline': return need(this.bridge.outline)(String(p.what ?? '') as 'resolve' | 'outline' | 'find', p.args ?? null)
      case 'write': return need(this.bridge.write)(String(p.kind ?? ''), String(p.name ?? ''), p.args ?? null, typeof p.batch === 'string' ? p.batch : null)
      case 'batch': return need(this.bridge.batch)(String(p.action ?? '') as 'begin' | 'end' | 'abort', typeof p.id === 'string' ? p.id : null, typeof p.label === 'string' ? p.label : undefined)
      case 'emit': {
        if (this.version < 3) throw new Error(aiwsT('aiworkspace.extension.unknownCall', { type }))
        // mounting, a sync or a reload never raises an event: only what follows a person's gesture in this Block does
        if (!this.gestured) { this.bridge.notify(aiwsT('aiworkspace.extension.emitBeforeGesture', { name: String(p.name ?? '') })); return { handled: false, event_id: null } }
        const params = p.params && typeof p.params === 'object' && !Array.isArray(p.params) ? p.params as Record<string, Json> : {}
        const result = await need(this.bridge.emit)(String(p.name ?? ''), params)
        return result as unknown as Json
      }
      case 'userState':
        if (this.version < 4) throw new Error(aiwsT('aiworkspace.extension.unknownCall', { type }))
        return need(this.bridge.userState)(p.action === 'set' ? 'set' : 'get', String(p.key ?? ''), p.value ?? null)
      case 'mediaUrl':
        if (this.version < 4) throw new Error(aiwsT('aiworkspace.extension.unknownCall', { type }))
        return need(this.bridge.mediaUrl)(String(p.binding ?? ''), typeof p.member === 'string' ? p.member : null)
      case 'asset': {
        const got = await need(this.bridge.asset)(String(p.name ?? ''))
        return { bytes: Array.from(got.bytes), media_type: got.media_type }
      }
      case 'watch': {
        const name = String(p.name ?? '')
        const off = need(this.bridge.watch)(name, () => this.post('event', null, { name } as Json))
        if (!off) throw new Error(aiwsT('aiworkspace.extension.noBinding', { name }))
        this.unwatch.push(off)
        return null
      }
      default:
        throw new Error(aiwsT('aiworkspace.extension.unknownCall', { type }))
    }
  }

  dispose() {
    if (this.disposed) return
    this.disposed = true
    window.removeEventListener('message', this.listener)
    for (const off of this.unwatch.splice(0)) off()
    this.rejectWaiting(new Error(aiwsT('aiworkspace.extension.unloaded')))
    this.frame?.remove()
    this.frame = null
  }
}
