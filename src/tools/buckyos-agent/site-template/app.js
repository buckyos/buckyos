import { ago, has, setLanguage, t, today } from './i18n.js'

const SITE = '/_data/site.json'
export const EVENTS = '/_data/events.json'
export const READING = '/_data/reading.json'
const feedUrl = (id) => `/_data/feed/${id}.json`
export const articleUrl = (id) => `/_data/a/${id}.json`

const ROUTE = /^(?:([ase])=([a-z0-9_-]{1,64}))?$/
const FRESH_MS = 60000
// The session lasts 15 minutes, and a reading record sent on leaving cannot be sent again after a 401.
const KEEP_MS = 600000
const ICONS = {
  back: 'M15 5l-7 7 7 7',
  fold: 'M6 9l6 6 6-6',
  next: 'M9 5l7 7-7 7',
  out: 'M8 7h9v9M17 7L7 17',
  gone: 'M4 5h16v14H4zM4 15l4.5-4.5L13 15l2.5-2.5L20 17M3 3l18 18',
}
const TONE = { ok: 'good', failed: 'bad', needs_login: 'warn', needs_upgrade: 'info' }

const embedded = window.parent !== window
const store = new Map()
const flights = new Set()
const again = new Set()
const routes = []
const columns = { wanted: new Set(), folds: new Map() }

export const app = { site: null, text: 't' }

let root = null
let view = null
let reader = null
let seen = {}
let siteText = ''
let ready = false
let queued = false
let renewing = null
let renewedAt = 0
let keeping = 0
let signedOut = false
let warming = null
let refreshedAt = 0
let saving = 0
let lastY = 0
let away = false

function recall(area, key) {
  try { return window[area].getItem(`site.${key}`) } catch { return null }
}

function keep(area, key, value) {
  try { window[area].setItem(`site.${key}`, value) } catch { /* the page works without storage */ }
}

export const str = (value) => (typeof value === 'string' ? value : '')
export const arr = (value) => (Array.isArray(value) ? value : [])

export function h(tag, props, ...kids) {
  const el = document.createElement(tag)
  for (const [name, value] of Object.entries(props ?? {})) {
    if (value == null || value === false) continue
    if (name.startsWith('on')) el.addEventListener(name.slice(2), value)
    else el.setAttribute(name, value === true ? '' : value)
  }
  el.append(...kids.flat(2).filter((kid) => kid != null && kid !== false && kid !== ''))
  return el
}

export function icon(name) {
  const svg = document.createElementNS('http://www.w3.org/2000/svg', 'svg')
  const path = document.createElementNS('http://www.w3.org/2000/svg', 'path')
  svg.setAttribute('viewBox', '0 0 24 24')
  svg.setAttribute('class', 'icon')
  svg.setAttribute('aria-hidden', 'true')
  path.setAttribute('d', ICONS[name])
  svg.append(path)
  return svg
}

export function web(url) {
  try {
    const parsed = new URL(url)
    return parsed.protocol === 'http:' || parsed.protocol === 'https:' ? parsed.href : null
  } catch {
    return null
  }
}

export function host(url) {
  try { return new URL(url).hostname.replace(/^www\./, '') } catch { return '' }
}

export const picture = (key) => (/^_data\/img\/[0-9a-f]{64}\.[a-z0-9]{1,5}$/.test(str(key)) ? `/${key}` : null)

const href = (route) => (route ? `?${route}` : location.pathname)

export const inside = (route, props, ...kids) => h('a', { ...props, href: href(route) }, ...kids)

export function outside(url, props, ...kids) {
  const target = web(url)
  return target && h('a', { ...props, href: target, target: '_blank', rel: 'noopener noreferrer', 'data-open': true }, ...kids)
}

function post(message) {
  if (embedded) window.parent.postMessage(message, '*')
}

// ── Data: what is in the browser's cache shows first, the network's answer replaces it ──

function renew() {
  renewing ??= fetch('/sso_refresh', { method: 'POST' }).catch(() => {}).finally(() => {
    renewing = null
    renewedAt = Date.now()
    keepSession()
  })
  return renewing
}

function keepSession() {
  clearTimeout(keeping)
  if (document.visibilityState !== 'visible') return
  const wait = renewedAt + KEEP_MS - Date.now()
  if (wait > 0) keeping = setTimeout(keepSession, wait)
  else renew()
}

async function request(url, init) {
  let response = await fetch(url, init)
  if (response.status === 401) {
    await renew()
    response = await fetch(url, init)
  }
  if (response.status !== 401) {
    signedOut = false
    return response
  }
  if (!embedded) {
    location.reload()
    return new Promise(() => {})
  }
  // Inside the App the frame is the App's to sign in again: the page says so once and keeps what it shows.
  if (!signedOut) post({ type: 'site.session' })
  signedOut = true
  throw new Error(`${url}: 401`)
}

async function read(url, init) {
  const response = await request(url, init)
  const text = await response.text()
  if (response.status === 404) return { state: 'missing', text: '' }
  if (!response.ok) throw new Error(`${url}: ${response.status}`)
  return { state: 'ok', text, data: JSON.parse(text) }
}

// Whether the last answer for this address came out of the browser's cache without asking the host. Not knowing counts as yes.
const unasked = (url) => !performance.getEntriesByName?.(new URL(url, location.href).href).at(-1)?.transferSize

function put(url, next) {
  const was = store.get(url)
  store.set(url, { ...next, at: Date.now() })
  if (!was || was.text !== next.text || was.state !== next.state || !was.late !== !next.late) invalidate()
}

async function load(url, first) {
  try {
    if (first) {
      const kept = await read(url, { cache: 'force-cache' })
      const live = !unasked(url)
      put(url, { ...kept, live })
      if (live) return
    }
    put(url, { ...await read(url), live: true })
  } catch {
    put(url, { ...(store.get(url) ?? { state: 'error', text: '' }), live: false, late: true })
  }
}

function sync(url, force) {
  const had = store.get(url)
  if (flights.has(url)) {
    if (force) again.add(url)
    return
  }
  if (!force && (had?.live || had?.late) && Date.now() - had.at < FRESH_MS) return
  flights.add(url)
  load(url, !had).finally(() => {
    flights.delete(url)
    if (again.delete(url)) sync(url, true)
    invalidate()
  })
}

export function use(url) {
  view.urls.add(url)
  sync(url)
  return store.get(url)
}

async function refresh() {
  if (!view) return
  refreshedAt = Date.now()
  if (reader === false) reader = null
  for (const url of view.urls) sync(url, true)
  invalidate()
  let next
  try { next = await read(SITE) } catch { return }
  if (next.state !== 'ok' || next.text === siteText || !next.data) return
  if (next.data.template?.version !== app.site.template?.version) {
    location.reload()
    return
  }
  siteText = next.text
  if (next.data.language !== app.site.language) await speak(next.data.language)
  adopt(next.data)
  invalidate()
}

// ── What the owner has read: the site's summary, and what this device opened since ──

export function see(id) {
  seen[id] = Date.now()
  const ids = Object.keys(seen)
  if (ids.length > 400) for (const old of ids.sort((a, b) => seen[a] - seen[b]).slice(0, 100)) delete seen[old]
  keep('localStorage', 'seen', JSON.stringify(seen))
}

export const readAt = (id) => Math.max(store.get(READING)?.data?.articles?.[id]?.last_opened_at ?? 0, seen[id] ?? 0)

export function setText(mode) {
  app.text = mode
  keep('sessionStorage', 'text', mode)
  invalidate()
}

// ── Site ──

function adopt(site) {
  app.site = {
    ...site,
    title: str(site.title),
    sources: arr(site.sources).filter((source) => ROUTE.test(`s=${source?.id}`)),
    columns: arr(site.columns).filter((column) => column && typeof column === 'object'),
    upgrades: arr(site.upgrades),
  }
}

export const sourceOf = (id) => app.site.sources.find((source) => source.id === id)
const upgradeOf = (id) => app.site.upgrades.find((upgrade) => upgrade?.state === 'open' && upgrade.source === id)
const tagsOf = (item) => arr(item.tags).filter((tag) => typeof tag === 'string')

function condition(source, feed) {
  if (source.state === 'paused') return 'paused'
  if (upgradeOf(source.id)) return 'needs_upgrade'
  if (source.state === 'preparing' || source.state === 'failed') return source.state
  if (!feed || feed.state === 'error') return null
  return feed.state === 'missing' ? 'preparing' : str(feed.data?.status?.state) || 'preparing'
}

export function find(id) {
  for (const source of app.site.sources) {
    const item = arr(store.get(feedUrl(source.id))?.data?.items).find((one) => one?.id === id)
    if (item) return { ...item, source: source.id }
  }
  return null
}

const when = (one) => (Number.isFinite(one?.at) ? one.at : null)

export function stories() {
  return arr(store.get(EVENTS)?.data?.events).filter((event) => ROUTE.test(`e=${event?.id}`)).map((event) => ({
    event,
    parts: [
      ...arr(event.articles).map((one) => ({ id: one?.id, adds: str(one?.adds), title: str(one?.title), at: when(one), source: one?.source })),
      ...arr(event.supplements).map((one) => ({ id: one?.article, adds: str(one?.adds), title: str(one?.title), at: when(one), extra: true, url: str(one?.source_url) })),
    ].filter((part) => ROUTE.test(`a=${part.id}`)),
  }))
}

function news() {
  const out = []
  for (const story of stories()) {
    if (story.parts.length < 2) continue
    let anchor = null
    for (const part of story.parts) {
      part.read = readAt(part.id)
      if (part.read > (anchor?.read ?? 0)) anchor = part
    }
    const unread = story.parts.filter((part) => !part.read)
    if (anchor && unread.length && story.event.updated_at > Math.max(anchor.read, seen[story.event.id] ?? 0)) out.push({ ...story, anchor, latest: unread.at(-1) })
  }
  return out.sort((a, b) => b.event.updated_at - a.event.updated_at).slice(0, 5)
}

// ── Pieces ──

export function memo(key, mark, make) {
  const had = view.nodes.get(key)
  const node = had?.mark === mark ? had : { mark, el: make() }
  view.next.set(key, node)
  return node.el
}

export function bar(up, label, ...tools) {
  return h('nav', { class: away ? 'bar away' : 'bar' },
    h('button', { class: 'tool', type: 'button', 'aria-label': t('back'), onclick: () => back(up) }, icon('back')),
    label && inside(up, { class: 'bar-up' }, label),
    h('span', { class: 'gap' }),
    tools)
}

export function skeleton() {
  return h('div', { class: 'wait', 'aria-hidden': 'true' }, [70, 100, 85, 40].map((width) => h('span', { style: `width:${width}%` })))
}

export function failure() {
  return h('div', { class: 'empty' }, h('p', null, t('failed')), h('button', { class: 'btn', type: 'button', onclick: refresh }, t('retry')))
}

export function lost(key) {
  return h('div', { class: 'empty' }, h('p', null, t(key)), inside('', { class: 'btn' }, app.site.title))
}

export function thread(event, adds) {
  return inside(`e=${event.id}`, { class: 'thread', 'data-k': `e/${event.id}` },
    h('span', { class: 'tag' }, t('story')), h('b', { dir: 'auto' }, str(event.title)), adds && h('span', { class: 'adds', dir: 'auto' }, adds))
}

function row(item, scope, sourceId) {
  const read = readAt(item.id) > 0
  const at = item.published_at ?? item.fetched_at
  const meta = [sourceId && sourceOf(sourceId)?.title, at && ago(at), ...tagsOf(item)].filter(Boolean).join(' · ')
  const image = picture(item.image)
  const key = `${scope}/${item.id}`
  return memo(key, JSON.stringify([item.title, item.summary, image, meta, read]), () =>
    h('article', { class: read ? 'row read' : 'row', 'data-k': key },
      h('div', { class: 'row-text' },
        h('h3', { dir: 'auto' }, inside(`a=${item.id}`, { class: 'row-link' }, str(item.title))),
        str(item.summary) && h('p', { class: 'row-sum', dir: 'auto' }, item.summary),
        h('p', { class: 'meta' }, meta)),
      image && h('img', { class: 'thumb', src: image, alt: '', loading: 'lazy', decoding: 'async', width: 96, height: 72 })))
}

function ask(upgrade) {
  return h('section', { class: 'card' }, h('p', { class: 'tag' }, t('upgrade')), h('h2', { dir: 'auto' }, str(upgrade.title)), h('p', { dir: 'auto' }, str(upgrade.summary)))
}

function notes(source, feed, onHome) {
  const state = condition(source, feed)
  const status = feed?.data?.status
  const line = (tone, text, action) => h('p', { class: 'note' }, h('span', { class: `dot ${tone ?? ''}` }),
    onHome ? inside(`s=${source.id}`, { class: 'note-text' }, h('b', null, source.title), text) : h('span', { class: 'note-text' }, text), action)
  const out = []
  if (state && (state !== 'ok' || !onHome)) {
    let text = has(`st.${state}`) ? t(`st.${state}`) : state
    let action = null
    if (state === 'failed') text = source.state === 'failed' ? [text, str(source.note)].filter(Boolean).join(' · ') : t(has(`err.${status?.error?.kind}`) ? `err.${status.error.kind}` : 'err.program')
    if (state === 'needs_upgrade') text = `${text} · ${str(upgradeOf(source.id).title)}`
    if (state === 'needs_login') action = outside(status?.login_url ?? source.home_url, { class: 'act' }, t('signIn'))
    if (!onHome) text = `${text} · ${status?.checked_at ? t('lastSync', { time: ago(status.checked_at) }) : t('neverSynced')}`
    out.push(line(Object.hasOwn(TONE, state) ? TONE[state] : '', text, action))
  }
  if (status?.pending?.count > 0) {
    const why = `why.${status.pending.reason}`
    out.push(line('', [t('pending', { n: status.pending.count }), has(why) ? t(why) : str(status.pending.message)].filter(Boolean).join(' · ')))
  }
  if (!onHome && status?.error?.message && state !== 'paused') out.push(h('p', { class: 'meta detail' }, status.error.message))
  return out
}

// ── Home ──

function sections() {
  const { site } = app
  const placed = new Set()
  const out = []
  for (const column of site.columns) {
    const sources = arr(column.sources).map((id) => sourceOf(id)).filter(Boolean)
    for (const source of sources) placed.add(source.id)
    if (sources.length) out.push({ id: `c-${column.id}`, title: str(column.title), sources, folded: arr(site.template?.params?.collapsed).includes(column.id) })
  }
  for (const source of site.sources) if (!placed.has(source.id)) out.push({ id: `s-${source.id}`, title: source.title, sources: [source] })
  return out
}

const timeOf = (item) => item.published_at ?? item.fetched_at ?? 0

function lines(section, params) {
  const hidden = new Set(arr(params.hide_topics))
  const per = Math.min(100, Math.max(1, Math.trunc(params.per_column) || 20))
  const items = section.sources.flatMap((source) => arr(store.get(feedUrl(source.id))?.data?.items)
    .filter((item) => ROUTE.test(`a=${item?.id}`) && !(tagsOf(item).length && tagsOf(item).every((tag) => hidden.has(tag))))
    .map((item) => ({ ...item, source: source.id })))
    .sort((a, b) => timeOf(b) - timeOf(a))
  if (params.home_order === 'source') {
    const share = Math.max(1, Math.ceil(per / section.sources.length))
    return section.sources.flatMap((source) => {
      const own = items.filter((item) => item.source === source.id).slice(0, share)
      return own.length ? [inside(`s=${source.id}`, { class: 'group' }, source.title), ...own.map((item) => row(item, section.id))] : []
    })
  }
  if (params.home_order !== 'events') return items.slice(0, per).map((item) => row(item, section.id, item.source))
  const told = stories().filter((story) => story.parts.length > 1)
  const out = []
  const used = new Set()
  for (const item of items) {
    if (used.size >= per) break
    if (used.has(item.id)) continue
    const story = told.find((one) => one.parts.some((part) => part.id === item.id))
    const group = story ? items.filter((other) => !used.has(other.id) && story.parts.some((part) => part.id === other.id)) : [item]
    for (const member of group.slice(0, per - used.size)) {
      used.add(member.id)
      out.push(row(member, section.id, member.source))
    }
    if (story) out.push(thread(story.event))
  }
  return out
}

function fold(id, folded) {
  columns.folds.set(id, folded)
  invalidate()
}

function home() {
  const { site } = app
  const params = site.template?.params ?? {}
  const shown = sections()
  say({ title: site.title })
  if (!shown.length) return [masthead(), vacant('empty.text')]
  use(EVENTS)
  use(READING)
  const bare = site.sources.every((source) => {
    const feed = store.get(feedUrl(source.id))
    return feed && feed.state !== 'error' && !arr(feed.data?.items).length
  })
  const parts = shown.map((section) => {
    const folded = columns.folds.get(section.id) ?? !!section.folded
    const urls = section.sources.map((source) => feedUrl(source.id))
    if (!folded && urls.every((url) => store.has(url))) columns.wanted.add(section.id)
    const wanted = !folded && columns.wanted.has(section.id)
    const feeds = wanted ? urls.map(use) : []
    const done = wanted && feeds.every(Boolean)
    const rows = done ? lines(section, params) : []
    return h('section', { class: 'sec', 'data-s': section.id, 'data-done': done, 'data-folded': folded },
      h('h2', { class: 'sec-h' },
        h('button', { class: 'fold', type: 'button', 'aria-expanded': String(!folded), onclick: () => fold(section.id, !folded) }, h('span', null, section.title), icon('fold'))),
      !folded && section.sources.map((source, i) => notes(source, wanted ? feeds[i] : undefined, true)),
      !folded && (!done ? skeleton()
        : feeds.every((feed) => feed.state === 'error') ? failure()
          : rows.length ? rows
            : !bare && h('p', { class: 'meta none' }, t('none'))),
      done && rows.length > 0 && params.home_order !== 'source' && h('p', { class: 'more' }, section.sources.map((source) => inside(`s=${source.id}`, { class: 'act' }, source.title, icon('next')))))
  })
  const fresh = news().map(({ event, anchor, latest }) => {
    const title = anchor.title || str(find(anchor.id)?.title)
    return [
      title && h('article', { class: 'row read', 'data-k': `news/${anchor.id}` }, h('div', { class: 'row-text' },
        h('h3', { dir: 'auto' }, inside(`a=${anchor.id}`, { class: 'row-link' }, title)),
        h('p', { class: 'meta' }, t('readEarlier', { time: ago(anchor.read) })))),
      thread(event, latest.adds),
    ]
  })
  return [
    masthead(),
    site.upgrades.filter((upgrade) => upgrade?.state === 'open' && !sourceOf(upgrade.source)).map(ask),
    bare && vacant('empty.wait'),
    fresh.length > 0 && h('section', { class: 'sec' }, h('h2', { class: 'sec-h plain' }, t('followups')), fresh),
    parts,
  ]
}

function masthead() {
  const checked = Math.max(0, ...app.site.sources.map((source) => store.get(feedUrl(source.id))?.data?.status?.checked_at ?? 0))
  const busy = [...view.urls].some((url) => flights.has(url) && store.has(url))
  const late = [...view.urls].some((url) => store.get(url)?.late)
  return [
    h('header', { class: 'mast' },
      h('h1', null, app.site.title),
      h('p', { class: 'meta' }, [today(), busy ? t('updating') : checked ? t('synced', { time: ago(checked) }) : ''].filter(Boolean).join(' · '))),
    late && !busy && h('p', { class: 'note' }, h('span', { class: 'dot warn' }), h('span', { class: 'note-text' }, t('offline')),
      h('button', { class: 'act', type: 'button', onclick: refresh }, t('retry'))),
  ]
}

function vacant(text) {
  return h('div', { class: 'empty' }, h('h2', null, t('empty.title')), h('p', null, t(text)),
    embedded ? h('button', { class: 'btn main', type: 'button', onclick: () => post({ type: 'site.add' }) }, t('empty.add')) : h('p', { class: 'meta' }, t('empty.where')))
}

function more() {
  if (view?.kind !== 'home') return
  for (const el of root.querySelectorAll('[data-s]:not([data-done], [data-folded])')) {
    // a place to return to further down needs the columns above it first
    if (!columns.wanted.has(el.dataset.s) && (view.y || el.getBoundingClientRect().top < window.innerHeight * 1.5)) {
      columns.wanted.add(el.dataset.s)
      invalidate()
    }
    return
  }
  view.y = 0
}

// ── Source ──

function source() {
  const one = sourceOf(view.id)
  if (!one) {
    say({ title: app.site.title })
    return [bar('', app.site.title), lost('gone.source')]
  }
  say({ title: one.title, source: one.id })
  const feed = use(feedUrl(one.id))
  use(READING)
  const upgrade = upgradeOf(one.id)
  const items = arr(feed?.data?.items).filter((item) => ROUTE.test(`a=${item?.id}`))
  return [
    bar('', app.site.title),
    h('header', { class: 'head' }, h('h1', null, str(one.title)), outside(one.home_url, { class: 'act ext' }, host(one.home_url), icon('out'))),
    notes(one, feed, false),
    upgrade && ask(upgrade),
    h('section', { class: 'list' }, !feed ? skeleton()
      : feed.state === 'error' ? failure()
        : items.length ? items.map((item) => row(item, 's'))
          : h('p', { class: 'meta none' }, t('none'))),
  ]
}

// ── Article and story pages live in article.js: the home needs none of it ──

function warm() {
  // A module that failed to load may stay failed under its address: the second try, after the session was renewed if
  // that was the reason, asks under another one.
  warming ??= import('./article.js')
    .catch(async () => {
      await request(new URL('./article.js', import.meta.url))
      return import(`./article.js?again=${Date.now()}`)
    })
    .then((module) => { reader = module }, () => { reader = false; warming = null })
  return warming
}

function reading() {
  if (reader) return view.kind === 'a' ? reader.article(view) : reader.story(view)
  if (reader === false) return [bar('', app.site.title), failure()]
  warm().finally(invalidate)
  return [bar('', app.site.title), skeleton()]
}

const pages = { home, s: source, a: reading, e: reading }

// ── Drawing: a page is drawn again from what is loaded, and what the owner was looking at stays where it was ──

function invalidate() {
  if (queued || !view) return
  queued = true
  queueMicrotask(() => {
    queued = false
    draw()
  })
}

function draw() {
  // Measured before the new tree is built: building it takes the pieces that stay out of the page.
  let held = null
  if (view.drawn && window.scrollY > 8) {
    for (const el of root.querySelectorAll('[data-k]')) {
      const box = el.getBoundingClientRect()
      if (box.bottom > 0) {
        held = { key: el.dataset.k, top: box.top }
        break
      }
    }
  }
  let tree
  view.next = new Map()
  try {
    tree = pages[view.kind]()
  } catch (error) {
    console.error(error)
    tree = [bar('', app.site.title), failure()]
  }
  root.replaceChildren(...[tree].flat(4).filter((node) => node != null && node !== false && node !== ''))
  view.nodes = view.next
  view.drawn = true
  const same = held && [...root.querySelectorAll('[data-k]')].find((el) => el.dataset.k === held.key)
  if (same) window.scrollBy(0, same.getBoundingClientRect().top - held.top)
  if (view.y && document.documentElement.scrollHeight - window.innerHeight >= view.y) {
    window.scrollTo(0, view.y)
    view.y = 0
  }
  // the page moved itself: the top bar slides only when the owner scrolls
  lastY = window.scrollY
  more()
  if (!ready) {
    ready = true
    post({ type: 'site.ready' })
  }
}

export function say({ title, url, source: from }) {
  if (view.said) return
  view.said = true
  document.title = title && title !== app.site.title ? `${title} · ${app.site.title}` : app.site.title
  const { route } = view
  queueMicrotask(() => post({ type: 'site.location', route, title: str(title), ...(url && { url }), ...(from && { source: from }) }))
}

// ── Routes ──

const parse = (route) => {
  const match = ROUTE.exec(route)
  return match && { route, kind: match[1] ?? 'home', id: match[2] }
}

function current() {
  const query = new URLSearchParams(location.search)
  for (const name of ['a', 's', 'e']) {
    const to = parse(`${name}=${query.get(name)}`)
    if (query.has(name) && to) return to
  }
  return parse('')
}

function enter(to, y) {
  clearTimeout(saving)
  view?.stop?.()
  view = { ...to, urls: new Set(), nodes: new Map(), y, said: false }
  routes[history.state?.n ?? 0] = to.route
  away = false
  lastY = y
  draw()
  if (!y) window.scrollTo(0, 0)
}

function go(route, replace) {
  const to = parse(route)
  if (!to || !view || to.route === view.route) return
  const n = history.state?.n ?? 0
  if (replace) {
    history.replaceState({ n }, '', href(to.route))
  } else {
    history.replaceState({ n, y: window.scrollY }, '')
    history.pushState({ n: n + 1 }, '', href(to.route))
    routes.length = n + 1
  }
  enter(to, 0)
}

function back(up) {
  if (history.state?.n > 0) history.back()
  else go(up, true)
}

function onClick(event) {
  const link = event.target.closest?.('a[href]')
  if (!link || event.defaultPrevented || event.button || event.metaKey || event.ctrlKey || event.shiftKey || event.altKey) return
  if (link.hasAttribute('data-open')) {
    if (!embedded) return
    event.preventDefault()
    post({ type: 'site.open', url: link.href })
  } else if (link.origin === location.origin && link.pathname === location.pathname) {
    event.preventDefault()
    go(link.search.slice(1))
  }
}

function onMessage(event) {
  const message = event.data
  if (!embedded || event.source !== window.parent || !message || typeof message !== 'object') return
  if (message.type === 'site.updated') refresh()
  if (message.type === 'site.theme' && (message.theme === 'light' || message.theme === 'dark')) document.documentElement.dataset.theme = message.theme
  if (message.type !== 'site.navigate' || !parse(message.route)) return
  const n = history.state?.n ?? 0
  // The App's own way back names the page before this one: going back in history keeps where the owner was on it.
  if (n > 0 && routes[n - 1] === message.route) history.back()
  else go(message.route)
}

function onScroll() {
  clearTimeout(saving)
  saving = setTimeout(() => history.replaceState({ ...history.state, y: window.scrollY }, ''), 200)
  const y = window.scrollY
  if (Math.abs(y - lastY) >= 6) {
    away = y > lastY && y > 80
    lastY = y
    root.querySelector('.bar')?.classList.toggle('away', away)
  }
  more()
}

async function speak(language) {
  const { lang, dir } = await setLanguage(language)
  document.documentElement.lang = lang
  document.documentElement.dir = dir
}

export default async function start({ site, mount }) {
  adopt(site)
  app.text = recall('sessionStorage', 'text') === 'o' ? 'o' : 't'
  try { seen = JSON.parse(recall('localStorage', 'seen')) } catch { /* nothing kept */ }
  if (!seen || typeof seen !== 'object') seen = {}
  root = mount
  root.className = 'site'
  await speak(site.language)
  history.scrollRestoration = 'manual'
  if (history.state?.n == null) history.replaceState({ n: 0 }, '')
  root.addEventListener('click', onClick)
  window.addEventListener('message', onMessage)
  window.addEventListener('popstate', () => enter(current(), history.state?.y ?? 0))
  window.addEventListener('scroll', onScroll, { passive: true })
  window.addEventListener('pointerdown', warm, { once: true, passive: true })
  performance.addEventListener?.('resourcetimingbufferfull', () => performance.clearResourceTimings())
  document.addEventListener('visibilitychange', () => {
    keepSession()
    if (document.visibilityState === 'visible' && Date.now() - refreshedAt > FRESH_MS) refresh()
  })
  refreshedAt = renewedAt = Date.now()
  keepSession()
  enter(current(), history.state?.y ?? 0)
}
