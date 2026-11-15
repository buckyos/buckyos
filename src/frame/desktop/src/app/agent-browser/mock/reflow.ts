/* Reflow in the mock runtime: the user's library kept in this page (the same code reads and writes it as on a
 * Zone, through reflow/library.ts), `browser.reflow_*` of the mock Agent Service over it, and the pages the frame
 * shows. A page is a document of this origin built from the library (`srcdoc`): it posts the `reflow.*` messages of
 * the protocol, and the frame takes them because the mock source names this origin as the pages'. The Agent's side
 * (a fallback page, a failing refresh) is driven from `window.__agentBrowserMock.reflow`. */

import { fetchCurrentAccount } from '../../../api/account'
import { getMockEntityDid } from '../../messagehub/mock/data'
import { agentBrowserExtensions } from '../extensions'
import {
  FALLBACK_SCHEMA, MANIFEST_SCHEMA, RESULT_SCHEMA, ReflowError, libraryState, normalizeUrl, packageMatches, reflowAddress, viewOf,
  type Library, type LibraryStore, type Manifest, type MatchRule, type ReflowAddress, type ReflowState,
} from '../reflow/library'
import type { ReflowSource } from '../reflow/source'

const BUCKET = 'user/control-panel/mock/reflow'
const REFRESH_MS = 350
const LATENCY_MS = 20
/** Who made what the library starts with: the MessageHub mock's own Agent. */
const MAKER = getMockEntityDid('agent-coder')
const SEEDED_AT = Date.UTC(2026, 10, 14, 8, 30)

const encoder = new TextEncoder()
const decoder = new TextDecoder()
const objects = new Map<string, { bytes: Uint8Array<ArrayBuffer>; contentType: string }>()

function put(key: string, body: Uint8Array | string, contentType = 'application/octet-stream') {
  objects.set(key, { bytes: typeof body === 'string' ? encoder.encode(body) : new Uint8Array(body), contentType })
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
    return new Response(object.bytes, { headers: { 'Content-Type': object.contentType } })
  },
  async putBytes(_bucketId, key, body, options) {
    await seeded
    put(key, body, options?.contentType)
    return {}
  },
  async delete(_bucketId, key) {
    await seeded
    return { deleted: objects.delete(key) }
  },
}

const library: Library = { store, bucketId: BUCKET, userId: 'mock' }

// ── What the library starts with ──

/** The page of a mock package: it draws `results/<page>.json` and offers every message of the protocol. */
const PACKAGE_PAGE = `<!doctype html>
<html>
<head>
<meta charset="utf-8">
<meta name="viewport" content="width=device-width, initial-scale=1">
<title>Reflow</title>
<style>
  body { margin: 0 auto; max-width: 40rem; padding: 1rem; font: 17px/1.7 system-ui, sans-serif; }
  a, button { display: block; margin: 0.6rem 0; font: inherit; }
</style>
</head>
<body>
<h1 id="title"></h1>
<p id="made"></p>
<p id="comments"></p>
<a id="next" href="#">Next story</a>
<button id="more" type="button">Load comments</button>
<button id="again" type="button">Make again</button>
<button id="original" type="button">Original page</button>
<script>
const view = window.__reflowView;
const query = new URLSearchParams(view.query);
const tell = (message) => parent.postMessage(message, '*');
let draws = 0;
let data = {};
function draw() {
  const result = JSON.parse(parent.__agentBrowserMock.reflow.read(view.site + '/results/' + query.get('page') + '.json'));
  data = result.data;
  document.getElementById('title').textContent = data.title;
  document.getElementById('made').textContent = String(result.made_at);
  document.getElementById('comments').textContent = (data.clicked || []).join(' ');
  document.body.dataset.draws = String(++draws);
}
addEventListener('message', (event) => {
  if (event.source === parent && event.data && event.data.type === 'reflow.updated') draw();
});
document.getElementById('next').onclick = (event) => { event.preventDefault(); tell({ type: 'reflow.open', url: data.next }); };
document.getElementById('more').onclick = () => tell({ type: 'reflow.action', selector: '#load-comments' });
document.getElementById('again').onclick = () => tell({ type: 'reflow.refresh' });
document.getElementById('original').onclick = () => tell({ type: 'reflow.source' });
draw();
tell({ type: 'reflow.ready' });
</script>
</body>
</html>
`

function fallbackPage(title: string, links: string[]): string {
  const escape = (text: string) => text.replaceAll('&', '&amp;').replaceAll('<', '&lt;').replaceAll('>', '&gt;').replaceAll('"', '&quot;')
  return `<!doctype html>
<html>
<head>
<meta charset="utf-8">
<meta name="viewport" content="width=device-width, initial-scale=1">
<title>${escape(title)}</title>
<style>body { margin: 0 auto; max-width: 40rem; padding: 1rem; font: 17px/1.7 system-ui, sans-serif; }</style>
</head>
<body>
<h1 id="title">${escape(title)}</h1>
${links.map((link) => `<p><a href="${escape(link)}">${escape(link)}</a></p>`).join('\n')}
<script>
document.addEventListener('click', (event) => {
  const link = event.target.closest('a[href]');
  if (!link) return;
  event.preventDefault();
  parent.postMessage({ type: 'reflow.open', url: link.href }, '*');
});
parent.postMessage({ type: 'reflow.ready' }, '*');
</script>
</body>
</html>
`
}

function resultData(address: ReflowAddress, clicked: string[] = []) {
  const number = Number(/(\d+)$/.exec(address.path)?.[1] ?? 0)
  return { title: `Story ${address.path}`, next: `https://${address.host}/story/${number + 1}`, clicked }
}

function putResult(address: ReflowAddress, madeAt: number, clicked: string[] = []) {
  put(`${address.site}/results/${address.page}.json`, JSON.stringify({ schema: RESULT_SCHEMA, source_url: address.url, made_at: madeAt, made_by: MAKER, data: resultData(address, clicked) }), 'application/json')
}

function putFallback(address: ReflowAddress, madeAt: number, title: string, links: string[] = []) {
  put(`${address.site}/fallback/${address.page}.html`, fallbackPage(title, links), 'text/html; charset=utf-8')
  put(`${address.site}/fallback/${address.page}.json`, JSON.stringify({ schema: FALLBACK_SCHEMA, source_url: address.url, made_at: madeAt, made_by: MAKER }), 'application/json')
}

async function seed() {
  const manifest: Manifest = {
    schema: MANIFEST_SCHEMA,
    site: 'news-example-com',
    match: [{ host: 'news.example.com', path: '/*' }],
    preference: 'Headline and text only, one column',
    author: { kind: 'agent', id: MAKER },
    updated_at: SEEDED_AT - 3_600_000,
    refresh: { script: 'refresh.mjs', timeout_s: 60 },
  }
  put('news-example-com/manifest.json', JSON.stringify(manifest, null, 2), 'application/json')
  put('news-example-com/refresh.mjs', "export default async ({ page }) => ({ title: await page.title() })\n", 'text/javascript; charset=utf-8')
  put('news-example-com/index.html', PACKAGE_PAGE, 'text/html; charset=utf-8')
  put('news-example-com/style.css', 'h1 { font-size: 1.4rem; }\n', 'text/css; charset=utf-8')
  putResult(await reflowAddress('https://news.example.com/story/1'), SEEDED_AT)
  putFallback(await reflowAddress('https://old.example.org/about'), SEEDED_AT, 'About the old site', ['https://old.example.org/contact'])
}

const seeded = seed()

// ── `browser.reflow_*` ──

class MockReflowError extends Error {
  readonly code: string
  constructor(code: string, message = code) {
    super(`RPC call error: Failed due to reason: ${JSON.stringify({ kind: code === 'invalid_argument' ? code : 'other', code, message })}`)
    this.code = code
  }
}

type Job = ReflowState['refresh']
const jobs = new Map<string, Job>()
const failures = new Map<string, string>()

async function addressOf(url: string): Promise<ReflowAddress> {
  try {
    return await reflowAddress(url)
  } catch (error) {
    throw new MockReflowError('invalid_argument', error instanceof ReflowError ? error.message : String(error))
  }
}

async function stateOf(address: ReflowAddress): Promise<ReflowState> {
  return { ...await libraryState(library, address), refresh: jobs.get(address.url) ?? { status: 'idle', error: null, at_ms: null } }
}

async function refresh(address: ReflowAddress, click?: string): Promise<ReflowState> {
  if (!(await libraryState(library, address)).package) throw new MockReflowError('reflow_no_package', `no package matches ${address.url}`)
  if (jobs.get(address.url)?.status !== 'running') {
    jobs.set(address.url, { status: 'running', error: null, at_ms: Date.now() })
    window.setTimeout(() => {
      const error = failures.get(address.url) ?? (address.path.startsWith('/broken') ? 'refresh.mjs: no element matches `h1`' : null)
      failures.delete(address.url)
      if (error) {
        jobs.set(address.url, { status: 'failed', error, at_ms: Date.now() })
        return
      }
      const before = read(`${address.site}/results/${address.page}.json`)
      const clicked: string[] = before ? (JSON.parse(before) as { data: { clicked: string[] } }).data.clicked : []
      putResult(address, Date.now(), click ? [...clicked, click] : clicked)
      jobs.delete(address.url)
    }, REFRESH_MS)
  }
  return stateOf(address)
}

/** `browser.reflow_state`, `browser.reflow_refresh` and `browser.reflow_action` of the mock Agent Service. The state is read without a browser, also with the module turned off (protocol §5.3); making anything needs one. */
export async function mockReflowCall(method: string, params: Record<string, unknown>, browser: { enabled: boolean; available: boolean }): Promise<ReflowState> {
  await new Promise((resolve) => window.setTimeout(resolve, LATENCY_MS))
  await seeded
  const address = await addressOf(typeof params.url === 'string' ? params.url : '')
  if (method === 'browser.reflow_state') return stateOf(address)
  if (!browser.enabled || !browser.available) throw new MockReflowError('browser_unavailable')
  if (method === 'browser.reflow_refresh') return refresh(address)
  if (typeof params.selector !== 'string' || !params.selector) throw new MockReflowError('invalid_argument', 'selector is missing')
  return refresh(address, params.selector)
}

// ── The library and its pages, for the App ──

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
    const html = read(path.endsWith('/') ? `${path}index.html` : path)
    if (html === null) return { kind: 'failed' }
    const context = `<script>window.__reflowView = ${JSON.stringify({ site: path.slice(0, path.indexOf('/')), query })}</script>`
    return { kind: 'ready', srcdoc: html.includes('<head>') ? html.replace('<head>', `<head>${context}`) : `${context}${html}` }
  },
}

/** The Agent's side of the library and what the specs check it with. */
export const mockReflowControl = {
  seededAt: SEEDED_AT,
  maker: MAKER,
  /** What the conversation side offers the App (`askAgent`): a spec replaces it to see what the Agent is asked. */
  extensions: agentBrowserExtensions,
  read,
  contentType: (key: string) => objects.get(key)?.contentType ?? null,
  keys: (prefix = '') => [...objects.keys()].filter((key) => key.startsWith(prefix)).sort(),
  /** The Agent keeps a fallback page for `url` (`reflow result put <url> page.html`). */
  async agentFallback(url: string, title = 'Fallback page') {
    await seeded
    putFallback(await reflowAddress(url), Date.now(), title)
  },
  /** The next refresh of `url` fails with `error`. */
  failRefresh(url: string, error: string) {
    failures.set(normalizeUrl(url).url, error)
  },
  /** The rules of §5.1–5.2 as this App computes them, for the shared vectors. */
  async address(url: string) {
    const address = await reflowAddress(url)
    return { ...address, view: viewOf(address, { result: true, fallback: false }) }
  },
  matches: (rules: MatchRule[], url: string) => packageMatches(rules, normalizeUrl(url)),
}
