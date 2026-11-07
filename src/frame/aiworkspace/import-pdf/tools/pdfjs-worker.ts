const root = new URL('../../../desktop/node_modules/pdfjs-dist/', import.meta.url)
const syncResources = new Set(['wasm/qcms_bg.wasm', 'iccs/CGATS001Compat-v2-micro.icc'].map(path => new URL(path, root).href))

class LocalColorResourceRequest {
  responseType = ''
  response: ArrayBuffer | null = null
  private url: string | null = null

  open(method: string, url: string, async: boolean) {
    if (method !== 'GET' || async || !syncResources.has(url)) throw new Error('Unsupported PDF.js color resource request')
    this.url = url
  }

  send() {
    if (!this.url || this.responseType !== 'arraybuffer') throw new Error('Invalid PDF.js color resource request')
    this.response = Deno.readFileSync(new URL(this.url)).buffer
  }
}

Object.defineProperty(globalThis, 'XMLHttpRequest', { value: LocalColorResourceRequest })
const { WorkerMessageHandler } = await import(new URL('legacy/build/pdf.worker.mjs', root).href)
WorkerMessageHandler.initializeFromPort(globalThis)
postMessage('pdfjs-local-worker-ready')
