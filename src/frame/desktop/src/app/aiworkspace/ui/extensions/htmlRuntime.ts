/* HTML extension runtime (phase two §10.4, D16): an HTML Block definition runs same-origin in an
 * iframe — the iframe is for style and crash isolation and mount management, not a security
 * boundary (the Owner is the trust root). The host offers a small, versioned JS API (`window.aiws`):
 * read bound data, query tables, submit candidate operations (which go through WorkspaceStore and
 * therefore through permissions, locks, undo and save states), upload assets, report a static
 * snapshot, and — for wish executors — answer analyze / execute requests.
 *
 * Errors, crashes and silence are localised: a call that does not answer in time rejects, and the
 * Block falls back to its static snapshot. */

import type { Json, Operation } from '../../api/types'

export const HTML_API_VERSION = 1
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
}

export interface HtmlSource { html: string; css?: string; js?: string; api_version?: number }

type Pending = { resolve: (value: Json) => void; reject: (error: Error) => void; timer: number }

/** The script that becomes `window.aiws` inside the iframe. Kept in a string so a definition ships as data. */
function bootstrap(nonce: string): string {
  return `
(function(){
  var seq = 0, waiting = {}, handlers = {};
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
    if (m.type === 'request') {
      var h = handlers[m.name];
      if (!h) { send('response', { error: 'no handler for ' + m.name }, m.id); return; }
      Promise.resolve().then(function(){ return h(m.payload); }).then(function(r){ send('response', { value: r }, m.id); }, function(e){ send('response', { error: String(e && e.message || e) }, m.id); });
    }
  });
  window.aiws = {
    version: ${HTML_API_VERSION},
    context: ${JSON.stringify('__CONTEXT__')},
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
    on: function(name, handler){ handlers[name] = handler; }
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
  /** Called when the extension reports an uncaught error. */
  onCrash: ((message: string) => void) | null = null

  constructor(source: HtmlSource, bridge: HostBridge) {
    if ((source.api_version ?? 1) !== HTML_API_VERSION) throw new Error(`不支持 HTML API 版本 ${source.api_version}`)
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
      case 'read': case 'query': case 'submit': case 'upload': case 'snapshot': {
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
      default:
        throw new Error(`unknown bridge call ${type}`)
    }
  }

  dispose() {
    if (this.disposed) return
    this.disposed = true
    window.removeEventListener('message', this.listener)
    this.rejectWaiting(new Error('扩展已卸载'))
    this.frame?.remove()
    this.frame = null
  }
}
