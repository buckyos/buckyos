/* The user's reflow bucket in the mock runtime, kept in this page (the same code reads and writes it as on a Zone,
 * through reflow/library.ts and site/data.ts): the pages tidied one by one with `browser.reflow_state` of the mock
 * Agent Service over them, the personal site's data (mock/site.ts), and the pages the frame shows. A page is a
 * document of this origin built from the bucket (`srcdoc`): it posts the messages of the protocols, and the frame
 * takes them because the mock source names this origin as the pages'. The Agent's side (a fallback page, what it
 * writes into the site) is driven from `window.__agentBrowserMock.reflow` and `.site`. */

import { fetchCurrentAccount } from '../../../api/account'
import { getMockEntityDid } from '../../messagehub/mock/data'
import { agentBrowserExtensions } from '../extensions'
import { ReflowError, libraryState, normalizeUrl, reflowAddress, rulesMatch, type Library, type LibraryStore, type MatchRule, type ReflowAddress, type ReflowState } from '../reflow/library'
import type { ReflowSource } from '../reflow/source'
import { MOCK_SITE_PAGE, mockSeed } from './site'

const BUCKET = 'user/control-panel/mock/reflow'
const LATENCY_MS = 20
/** Who made what the bucket starts with: the MessageHub mock's own Agent. */
const MAKER = getMockEntityDid('agent-coder')
const SEEDED_AT = Date.UTC(2026, 10, 14, 8, 30)

const encoder = new TextEncoder()
const decoder = new TextDecoder()
const objects = new Map<string, { bytes: Uint8Array<ArrayBuffer>; contentType: string; etag: string }>()
let writes = 0

const contentTypeOf = (key: string) => (key.endsWith('.html') ? 'text/html; charset=utf-8' : key.endsWith('.json') ? 'application/json' : 'application/octet-stream')

function put(key: string, body: Uint8Array | string) {
  objects.set(key, { bytes: typeof body === 'string' ? encoder.encode(body) : new Uint8Array(body), contentType: contentTypeOf(key), etag: `"mock-${++writes}"` })
}

function read(key: string): string | null {
  const object = objects.get(key)
  return object ? decoder.decode(object.bytes) : null
}

const store: LibraryStore = {
  async listObjects(_bucketId, { prefix = '', delimiter, start_after }) {
    await seeded
    const items: { key: string; size: number }[] = []
    const prefixes = new Set<string>()
    for (const key of [...objects.keys()].sort()) {
      if (!key.startsWith(prefix) || (start_after && key <= start_after)) continue
      const rest = key.slice(prefix.length)
      const cut = delimiter ? rest.indexOf(delimiter) : -1
      if (delimiter && cut >= 0) prefixes.add(prefix + rest.slice(0, cut + delimiter.length))
      else items.push({ key, size: objects.get(key)!.bytes.byteLength })
    }
    return { items, common_prefixes: [...prefixes] }
  },
  async fetchObject(_bucketId, key) {
    await seeded
    const object = objects.get(key)
    if (!object) throw Object.assign(new Error(`no such key: ${key}`), { code: 'no_such_key' })
    return new Response(object.bytes, { headers: { 'Content-Type': object.contentType, ETag: object.etag } })
  },
  async putBytes(_bucketId, key, body, options) {
    await seeded
    if (options?.if_match !== undefined && objects.get(key)?.etag !== options.if_match) throw Object.assign(new Error('the current etag differs'), { code: 'precondition_failed' })
    put(key, body)
    return {}
  },
  async delete(_bucketId, key) {
    await seeded
    return { deleted: objects.delete(key) }
  },
}

const library: Library = { store, bucketId: BUCKET, userId: 'mock' }

// ── What the bucket starts with ──

/** A tidied page: its title, its links (`reflow.open`) and the way back to the original (`reflow.source`). */
function fallbackPage(title: string, links: string[]): string {
  const escape = (text: string) => text.replaceAll('&', '&amp;').replaceAll('<', '&lt;').replaceAll('>', '&gt;').replaceAll('"', '&quot;')
  return `<!doctype html>
<html>
<head>
<meta charset="utf-8">
<meta name="viewport" content="width=device-width, initial-scale=1">
<title>${escape(title)}</title>
<style>
  body { margin: 0 auto; max-width: 40rem; padding: 1rem; font: 17px/1.7 system-ui, sans-serif; }
  button { font: inherit; }
</style>
</head>
<body>
<h1 id="title">${escape(title)}</h1>
${links.map((link) => `<p><a href="${escape(link)}">${escape(link)}</a></p>`).join('\n')}
<button id="original" type="button">Original page</button>
<script>
document.addEventListener('click', (event) => {
  const link = event.target.closest('a[href]');
  if (!link) return;
  event.preventDefault();
  parent.postMessage({ type: 'reflow.open', url: link.href }, '*');
});
document.getElementById('original').onclick = () => parent.postMessage({ type: 'reflow.source' }, '*');
parent.postMessage({ type: 'reflow.ready' }, '*');
</script>
</body>
</html>
`
}

function putFallback(address: ReflowAddress, madeAt: number, title: string, links: string[] = []) {
  put(`${address.site}/fallback/${address.page}.html`, fallbackPage(title, links))
  put(`${address.site}/fallback/${address.page}.json`, JSON.stringify({ schema: 'buckyos.reflow.fallback/1', source_url: address.url, made_at: madeAt, made_by: MAKER }))
}

async function seed() {
  putFallback(await reflowAddress('https://news.example.com/story/1'), SEEDED_AT, 'Story one', ['https://news.example.com/story/2'])
  putFallback(await reflowAddress('https://old.example.org/about'), SEEDED_AT, 'About the old site', ['https://old.example.org/contact'])
  for (const [key, text] of Object.entries(mockSeed.objects ?? {})) put(key, text)
}

const seeded = seed()

// ── `browser.reflow_state` ──

class MockReflowError extends Error {
  readonly code: string
  constructor(code: string, message = code) {
    super(`RPC call error: Failed due to reason: ${JSON.stringify({ kind: code, code, message })}`)
    this.code = code
  }
}

/** The Agent's service does not answer `browser.reflow_state` (the Agent is stopped): what the specs switch. */
let silent = false

/** `browser.reflow_state` of the mock Agent Service: read without a browser, also with the module turned off (protocol §5.3). */
export async function mockReflowState(params: Record<string, unknown>): Promise<ReflowState> {
  await new Promise((resolve) => window.setTimeout(resolve, LATENCY_MS))
  await seeded
  if (silent) throw Error('RPC call error: 502')
  let address: ReflowAddress
  try {
    address = await reflowAddress(typeof params.url === 'string' ? params.url : '')
  } catch (error) {
    throw new MockReflowError('invalid_argument', error instanceof ReflowError ? error.message : String(error))
  }
  return libraryState(library, address)
}

// ── The bucket and its pages, for the App ──

export const mockReflowSource: ReflowSource = {
  async library() {
    await seeded
    return { ...library, userId: (await fetchCurrentAccount())?.user_id ?? library.userId }
  },
  origin: window.location.origin,
  async openPage(view) {
    await new Promise((resolve) => window.setTimeout(resolve, LATENCY_MS))
    await seeded
    const [path, query = ''] = view.split('?')
    // The site's entry is the template's; the mock serves its stand-in.
    const html = path === '' ? MOCK_SITE_PAGE : read(path)
    if (html === null) return { kind: 'failed' }
    const context = `<script>window.__reflowView = ${JSON.stringify({ query, opened: ++opened })}</script>`
    return { kind: 'ready', srcdoc: html.replace('<head>', `<head>${context}`) }
  },
}

/** How many pages the frame was signed in for (each load of the frame is one). */
let opened = 0

/** The Agent's side of the bucket and what the specs check it with. */
export const mockReflowControl = {
  seededAt: SEEDED_AT,
  maker: MAKER,
  /** What the conversation side offers the App (`askAgent`): a spec replaces it to see what the Agent is asked. */
  extensions: agentBrowserExtensions,
  read,
  /** Whoever writes the bucket besides the App (the Agent, a job of its Goal): `null` deletes. */
  put(key: string, text: string | null) {
    if (text === null) objects.delete(key)
    else put(key, text)
  },
  keys: (prefix = '') => [...objects.keys()].filter((key) => key.startsWith(prefix)).sort(),
  opened: () => opened,
  /** The Agent's service stops answering what the bucket has, or answers again. */
  silence(on: boolean) {
    silent = on
  },
  /** The Agent keeps a fallback page for `url` (`reflow result put <url> page.html`). */
  async agentFallback(url: string, title = 'Fallback page') {
    await seeded
    putFallback(await reflowAddress(url), Date.now(), title)
  },
  /** The rules of §5.1 and the match rules as this App computes them, for the shared vectors. */
  address: (url: string) => reflowAddress(url),
  matches: (rules: MatchRule[], url: string) => rulesMatch(rules, normalizeUrl(url)),
}
