/* HTML extension runtime (phase two §10.4, D16; `aiws` v2 of 许愿格 §9.4): an HTML Block definition
 * runs same-origin in an iframe — the iframe is for style and crash isolation and mount management,
 * not a security boundary (the Owner is the trust root). The host offers a versioned JS API
 * (`window.aiws`):
 *
 *   v1 (still available): context, read, query, submit (raw operations), upload, snapshot, notify, on
 *   v2: bindings, input(name).rows/fields/markdown/props/bytes, resolve/outline/find, request,
 *       watch(name, cb), table(name).upsert/setColumn, record(name).set, text(name).setMarkdown,
 *       createBlock, batch(fn)
 *
 * Every write goes through WorkspaceStore and therefore through permissions, locks, undo and save
 * states. Errors, crashes and silence are localised: a call that does not answer in time rejects,
 * and the Block falls back to its static snapshot. */

import type { Json, Operation } from '../../api/types'

export const HTML_API_VERSION = 2
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
}

export interface HtmlSource { html: string; css?: string; js?: string; api_version?: number }

type Pending = { resolve: (value: Json) => void; reject: (error: Error) => void; timer: number }

/** The script that becomes `window.aiws` inside the iframe. Kept in a string so a definition ships as data. */
function bootstrap(nonce: string): string {
  return `
(function(){
  var seq = 0, waiting = {}, handlers = {}, watchers = {}, currentBatch = null;
  var nonce = ${JSON.stringify(nonce)};
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
    if (m.type === 'request') {
      var h = handlers[m.name];
      if (!h) { send('response', { error: 'no handler for ' + m.name }, m.id); return; }
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
    version: ${HTML_API_VERSION},
    host: 'block',
    context: context,
    request: context,
    bindings: context.bindings || {},
    ready: function(){ send('ready', null); },
    read: function(entityId, selector){ return call('read', { entity_id: entityId, selector: selector || null }); },
    query: function(params){ return call('query', params); },
    submit: function(operations, label){ return call('submit', { operations: operations, label: label || '' }); },
    upload: function(data, fileName, mediaType){
      var bytes = typeof data === 'string' ? new TextEncoder().encode(data) : (data instanceof ArrayBuffer ? new Uint8Array(data) : data);
      return call('upload', { bytes: Array.from(bytes), file_name: fileName || 'file', media_type: mediaType || null });
    },
    snapshot: function(dataUrl){ return call('snapshot', { data_url: dataUrl }); },
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
    batch: function(fn, label){
      return call('batch', { action: 'begin', id: null, label: label || '' }).then(function(id){
        var prev = currentBatch; currentBatch = id;
        return Promise.resolve().then(fn).then(
          function(){ currentBatch = prev; return call('batch', { action: 'end', id: id, label: label || '' }); },
          function(e){ currentBatch = prev; return call('batch', { action: 'abort', id: id }).then(function(){ throw e; }); });
      });
    }
  };
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
  /** Called when the extension reports an uncaught error. */
  onCrash: ((message: string) => void) | null = null

  constructor(source: HtmlSource, bridge: HostBridge) {
    if ((source.api_version ?? HTML_API_VERSION) !== HTML_API_VERSION) throw new Error(`不支持 HTML API 版本 ${source.api_version}`)
    this.source = source
    this.bridge = bridge
    this.readyPromise = new Promise((resolve, reject) => { this.readyResolve = resolve; this.readyReject = reject })
    void this.readyPromise.catch(() => undefined)
  }

  /** Build the document: markup + style + bootstrap + the definition's script. */
  private document(): string {
    const context = JSON.stringify(this.bridge.context()).replace(/<\//g, '<\\/')
    const boot = bootstrap(this.nonce).replace(JSON.stringify('__CONTEXT__'), context)
    const css = this.source.css ? `<style>${this.source.css}</style>` : ''
    const js = this.source.js ? `<script>${this.source.js.replace(/<\/script/gi, '<\\/script')}</script>` : ''
    return `<!doctype html><html><head><meta charset="utf-8">${css}<script>${boot}</script></head><body>${this.source.html}${js}</body></html>`
  }

  /** Mount into `container` (visible) or into the body hidden (headless executor). */
  mount(container: HTMLElement | null): Promise<void> {
    if (this.disposed) return Promise.reject(new Error('扩展已卸载'))
    if (this.frame) return this.readyPromise
    const frame = document.createElement('iframe')
    frame.className = 'aiws-html-frame'
    frame.setAttribute('title', 'HTML 扩展')
    if (!container) { frame.style.position = 'fixed'; frame.style.width = '1px'; frame.style.height = '1px'; frame.style.opacity = '0'; frame.style.pointerEvents = 'none'; frame.setAttribute('aria-hidden', 'true') }
    window.addEventListener('message', this.listener)
    frame.srcdoc = this.document()
    ;(container ?? document.body).appendChild(frame)
    this.frame = frame
    this.readyTimer = window.setTimeout(() => this.failAll(new Error('扩展在 8 秒内没有报告就绪（无响应）')), READY_TIMEOUT_MS)
    return this.readyPromise
  }

  get ready(): Promise<void> { return this.readyPromise }

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

  /** Ask the extension to handle `name` (e.g. `analyze`, `execute`, `render`). */
  request(name: string, payload: Json, timeoutMs = CALL_TIMEOUT_MS): Promise<Json> {
    if (this.disposed) return Promise.reject(new Error('扩展已卸载'))
    return this.readyPromise.then(() => new Promise<Json>((resolve, reject) => {
      if (this.disposed) { reject(new Error('扩展已卸载')); return }
      const id = ++this.seq
      const timer = window.setTimeout(() => this.failAll(new Error(`扩展处理 ${name} 超时（${Math.round(timeoutMs / 1000)} 秒无响应）`)), timeoutMs)
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
      case 'read': case 'query': case 'submit': case 'upload': case 'snapshot': case 'input': case 'outline': case 'write': case 'batch': case 'watch': {
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
    const need = <T>(f: T | undefined): T => { if (!f) throw new Error(`这个宿主不提供 ${type}`); return f }
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
      case 'watch': {
        const name = String(p.name ?? '')
        const off = need(this.bridge.watch)(name, () => this.post('event', null, { name } as Json))
        if (!off) throw new Error(`没有名为 ${name} 的绑定`)
        this.unwatch.push(off)
        return null
      }
      default:
        throw new Error(`unknown bridge call ${type}`)
    }
  }

  dispose() {
    if (this.disposed) return
    this.disposed = true
    window.removeEventListener('message', this.listener)
    for (const off of this.unwatch.splice(0)) off()
    this.rejectWaiting(new Error('扩展已卸载'))
    this.frame?.remove()
    this.frame = null
  }
}
