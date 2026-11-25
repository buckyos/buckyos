import { ago, moment, t } from './i18n.js'
import {
  EVENTS, READING, app, arr, articleUrl, bar, failure, find, h, host, icon, inside, lost, memo, outside, picture, readAt, say, see,
  setText, skeleton, sourceOf, stories, str, thread, use, web,
} from './app.js'

const TEXT_BLOCKS = { p: 'p', h2: 'h2', h3: 'h3', quote: 'blockquote', code: 'pre' }
const SOURCE = /^(?:_supplement|[a-z0-9](?:[a-z0-9-]{0,46}[a-z0-9])?)$/

// One visit to an article: the time it was on screen and how far it was read, sent when the page is hidden or left
// (Personal Site Protocol §3.6). A visit that goes on after being hidden sends what was added since, under the same
// `opened_at`.
function track(article, source) {
  // the host takes nothing but whole records: one with another kind of id would only be refused
  if (app.site.reading?.enabled === false || !/^[0-9a-f]{16}$/.test(article) || !SOURCE.test(source)) return null
  const opened = Date.now()
  let since = document.visibilityState === 'visible' ? performance.now() : null
  let total = 0
  let sent = 0
  let far = 0
  const measure = () => {
    const box = document.querySelector('.post')?.getBoundingClientRect()
    if (box?.height > 0) far = Math.max(far, Math.min(100, Math.round(((window.innerHeight - box.top) / box.height) * 100)))
  }
  const flush = () => {
    if (since != null) {
      measure()
      total += performance.now() - since
      since = null
    }
    const dwell = Math.round(total - sent)
    if (total < 2000 || dwell < 1) return
    sent = total
    const record = { article, source, opened_at: opened, dwell_ms: dwell, max_scroll: far }
    try {
      navigator.sendBeacon('/_read', new Blob([JSON.stringify([record])], { type: 'application/json' }))
    } catch { /* a record that cannot be sent is dropped */ }
  }
  const onVisibility = () => {
    if (document.visibilityState === 'hidden') flush()
    else since ??= performance.now()
  }
  document.addEventListener('visibilitychange', onVisibility)
  window.addEventListener('pagehide', flush)
  window.addEventListener('scroll', measure, { passive: true })
  return () => {
    flush()
    document.removeEventListener('visibilitychange', onVisibility)
    window.removeEventListener('pagehide', flush)
    window.removeEventListener('scroll', measure)
  }
}

function head(item) {
  return h('header', { class: 'head' }, h('h1', { dir: 'auto' }, str(item.title)), str(item.summary) && h('p', { class: 'lead', dir: 'auto' }, item.summary))
}

function related(id) {
  return stories().filter((story) => story.parts.length > 1 && story.parts.some((part) => part.id === id)).map((story) => [
    thread(story.event),
    h('ul', { class: 'lines' }, story.parts.filter((part) => part.id !== id).map((part) => {
      const title = part.title || str(find(part.id)?.title)
      return h('li', null, inside(`a=${part.id}`, { class: readAt(part.id) ? 'line read' : 'line' },
        h('span', { dir: 'auto' }, title || part.adds || t('story'), title && part.adds && h('span', { class: 'adds' }, part.adds)),
        part.extra && h('span', { class: 'meta' }, host(part.url))))
    })),
  ])
}

export function article(view) {
  const entry = use(articleUrl(view.id))
  use(EVENTS)
  use(READING)
  if (entry?.state !== 'ok' || !entry.data) {
    const known = find(view.id)
    if (known) say({ title: known.title, url: web(known.url), source: known.source })
    else if (entry) say({ title: app.site.title })
    const up = known ? [`s=${known.source}`, sourceOf(known.source)?.title] : ['', app.site.title]
    if (entry?.state === 'missing') return [bar(...up), lost('gone.article'), known && h('footer', { class: 'foot' }, outside(known.url, { class: 'btn' }, t('viewOriginal'), icon('out')))]
    return [bar(...up), known && head(known), entry ? failure() : skeleton()]
  }

  const a = entry.data
  const from = sourceOf(a.source)
  const story = a.supplement && stories().find((one) => one.event.id === a.supplement.event)
  const blocks = arr(a.blocks)
  const comments = arr(a.comments)
  const tags = arr(a.tags).filter((tag) => typeof tag === 'string')
  const two = [a.title, a.summary, ...blocks.map((block) => block?.text)].some((text) => text?.o && text.o !== text.t)
  const mode = two && app.text === 'o' ? 'o' : 't'
  const lang = mode === 'o' || !two ? str(a.lang) || null : app.site.language
  const pick = (text) => str(text?.[mode]) || str(text?.t) || str(text?.o)
  const tx = (tag, text, props) => h(tag, { lang, dir: 'auto', ...props }, pick(text))

  if (!view.stop) {
    see(view.id)
    view.stop = track(view.id, str(a.source)) ?? (() => {})
  }
  say({ title: str(a.title?.t) || str(a.title?.o), url: web(a.url), source: from?.id })

  const body = []
  let list = null
  blocks.forEach((block, i) => {
    const key = `b${i}`
    if (block?.type !== 'li') list = null
    if (block?.type === 'li') {
      if (!list) body.push(list = h('ul', { 'data-k': key, lang, dir: 'auto' }))
      list.append(h('li', null, pick(block.text)))
    } else if (block?.type === 'img') {
      const src = !block.missing && picture(block.src)
      const caption = pick(block.alt)
      const image = src && memo(`img/${src}`, '', () => h('img', { src, loading: 'lazy', decoding: 'async', width: block.w > 0 && block.w, height: block.h > 0 && block.h }))
      if (image) image.alt = caption
      body.push(h('figure', { 'data-k': key },
        image || h('div', { class: 'gone' }, icon('gone'), h('span', null, t('noImage'))),
        caption && h('figcaption', { lang, dir: 'auto' }, caption)))
    } else if (block?.type === 'table') {
      const rows = arr(block.rows).filter(Array.isArray)
      body.push(h('div', { class: 'scroll', 'data-k': key, tabindex: 0 }, h('table', null, h('tbody', null,
        rows.map((cells, r) => h('tr', null, cells.map((cell) => tx(r === 0 && rows.length > 1 ? 'th' : 'td', cell, { class: pick(cell).length > 24 && 'long' }))))))))
    } else if (Object.hasOwn(TEXT_BLOCKS, block?.type)) {
      body.push(tx(TEXT_BLOCKS[block.type], block.text, { 'data-k': key }))
    }
  })

  const links = arr(a.links).map((link) => {
    const label = h('span', { lang, dir: 'auto' }, pick(link?.text) || host(link?.url))
    if (/^[0-9a-f]{16}$/.test(str(link?.article))) return inside(`a=${link.article}`, { class: 'line' }, label)
    return outside(link?.url, { class: 'line' }, label, h('span', { class: 'meta' }, host(link?.url)), icon('out'))
  }).filter(Boolean)
  const more = related(view.id)
  const by = [str(a.byline) || from?.title || host(a.url), a.published_at && moment(a.published_at), a.rev > 1 && a.fetched_at && t('revised', { time: ago(a.fetched_at) })]

  return [
    bar(...(from ? [`s=${from.id}`, from.title] : story ? [`e=${story.event.id}`, story.event.title] : ['', app.site.title]),
      two && h('div', { class: 'seg', role: 'group' }, ['t', 'o'].map((one) =>
        h('button', { type: 'button', 'aria-pressed': String(mode === one), onclick: () => setText(one) }, t(one === 't' ? 'translation' : 'original')))),
      outside(a.url, { class: 'tool', 'aria-label': t('viewOriginal'), title: t('viewOriginal') }, icon('out'))),
    h('article', { class: 'post' },
      h('header', { class: 'head' },
        a.supplement && h('p', { class: 'tag' }, t('material')),
        tags.length > 0 && h('p', { class: 'meta' }, tags.join(' · ')),
        tx('h1', a.title),
        h('p', { class: 'meta' }, by.filter(Boolean).join(' · ')),
        a.supplement && str(a.supplement.adds) && h('p', { class: 'adds' }, a.supplement.adds),
        str(a.summary?.[mode]) && tx('p', a.summary, { class: 'lead' })),
      h('div', { class: 'body' }, body)),
    more.length > 0 && h('section', { class: 'sec' }, h('h2', { class: 'sec-h plain' }, t('followups')), more),
    links.length > 0 && h('section', { class: 'sec' }, h('h2', { class: 'sec-h plain' }, t('related')), h('ul', { class: 'lines' }, links.map((link) => h('li', null, link)))),
    comments.length > 0 && h('section', { class: 'sec' }, h('h2', { class: 'sec-h plain' }, `${t('comments')} · ${comments.length}`),
      comments.map((comment, i) => h('div', { class: 'cmt', 'data-k': `c${i}` },
        h('p', { class: 'meta' }, h('b', null, str(comment?.author)), comment?.at && ` · ${ago(comment.at)}`),
        tx('p', comment?.text)))),
    h('footer', { class: 'foot' }, outside(a.url, { class: 'btn' }, t('viewOriginal'), icon('out'))),
  ]
}

export function story(view) {
  const entry = use(EVENTS)
  use(READING)
  const found = stories().find((one) => one.event.id === view.id)
  if (!found) {
    if (entry) say({ title: app.site.title })
    return [bar('', app.site.title), !entry ? skeleton() : entry.state === 'error' ? failure() : lost('gone.story')]
  }
  const { event } = found
  if (!view.stop) {
    see(event.id)
    view.stop = () => {}
  }
  say({ title: str(event.title) })
  const reports = found.parts.filter((part) => !part.extra)
    .map((part, i) => ({ ...part, i }))
    .sort((x, y) => (x.at ?? Infinity) - (y.at ?? Infinity) || x.i - y.i)
  const extras = found.parts.filter((part) => part.extra)
  const step = (part) => h('li', { class: readAt(part.id) ? 'step read' : 'step', 'data-k': part.id },
    h('p', { class: 'meta' }, [part.extra ? host(part.url) : sourceOf(part.source)?.title, part.at && moment(part.at)].filter(Boolean).join(' · ')),
    inside(`a=${part.id}`, { class: 'step-t', dir: 'auto' }, part.title || part.adds || t('story')),
    part.title && part.adds && h('p', { class: 'adds', dir: 'auto' }, part.adds))
  return [
    bar('', app.site.title),
    h('header', { class: 'head' },
      h('p', { class: 'tag' }, t('story')),
      h('h1', { dir: 'auto' }, str(event.title)),
      event.updated_at > 0 && h('p', { class: 'meta' }, t('updated', { time: ago(event.updated_at) })),
      str(event.summary) && h('p', { class: 'lead', dir: 'auto' }, event.summary)),
    reports.length > 0 && h('ol', { class: 'steps' }, reports.map(step)),
    extras.length > 0 && h('section', { class: 'sec' }, h('h2', { class: 'sec-h plain' }, t('material')), h('ol', { class: 'steps' }, extras.map(step))),
  ]
}
