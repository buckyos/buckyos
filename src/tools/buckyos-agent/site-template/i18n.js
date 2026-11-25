export const en = {
  back: 'Back',
  synced: 'Synced {time}',
  updating: 'Updating…',
  offline: 'Could not update. This is what was read earlier.',
  retry: 'Try again',
  failed: 'This could not be loaded.',
  none: 'No articles yet.',
  followups: 'Follow-ups',
  story: 'Story',
  readEarlier: 'Read {time}',
  material: 'More material',
  translation: 'Translation',
  original: 'Original',
  viewOriginal: 'View the original page',
  comments: 'Comments',
  related: 'Related links',
  noImage: 'Image not available',
  revised: 'Revised {time}',
  updated: 'Updated {time}',
  'st.ok': 'Up to date',
  'st.preparing': 'Being set up',
  'st.failed': 'Could not be set up',
  'st.needs_login': 'Sign-in needed',
  'st.paused': 'Paused',
  'st.needs_upgrade': 'Needs an upgrade',
  'err.transient': 'Unreachable for now, will try again',
  'err.auth': 'The site asks to sign in',
  'err.program': 'The site changed, waiting for a fix',
  'err.config': 'A setting needs attention',
  signIn: 'Sign in',
  upgrade: 'Upgrade request',
  lastSync: 'Last synced {time}',
  neverSynced: 'Not synced yet',
  pending: '{n} waiting to be published',
  'why.translate_budget': 'today’s translation budget is used up',
  'why.translate_config': 'no translation model is available',
  'why.translate_failed': 'the translation did not work, it is tried again at the next sync',
  'why.time': 'the last sync ran out of time',
  'why.images': 'their images are still being fetched',
  'empty.title': 'Nothing to read yet',
  'empty.text': 'Add the sites you read. Their articles are gathered here, translated, every day.',
  'empty.wait': 'Your sources are being set up. The first articles arrive soon.',
  'empty.add': 'Add a source',
  'empty.where': 'Sources are added in the Agent Browser of BuckyOS.',
  'gone.article': 'This article is no longer kept.',
  'gone.source': 'There is no such source.',
  'gone.story': 'There is no such story.',
}

const FILES = ['zh-CN', 'zh-TW', 'ja', 'ko', 'fr', 'de', 'es', 'ru', 'ar']
const TRADITIONAL = /^zh-(tw|hk|mo|hant)\b/

let words = en
let locale = 'en'
let formats = {}

function fileOf(tag) {
  const lower = String(tag ?? '').replace(/_/g, '-').toLowerCase()
  if (lower === 'zh' || lower.startsWith('zh-')) return TRADITIONAL.test(lower) ? 'zh-TW' : 'zh-CN'
  return FILES.find((name) => name === lower.split('-')[0]) ?? 'en'
}

export async function setLanguage(tag) {
  const file = fileOf(tag)
  let own = {}
  if (file !== 'en') {
    try { own = (await import(`./lang/${file}.js`)).default } catch { /* English stands in */ }
  }
  words = { ...en, ...own }
  try { locale = Intl.getCanonicalLocales(tag)[0] ?? file } catch { locale = file }
  formats = {}
  return { lang: locale, dir: file === 'ar' ? 'rtl' : 'ltr' }
}

export const has = (key) => Object.hasOwn(words, key)

export function t(key, values) {
  return (words[key] ?? key).replace(/\{(\w+)\}/g, (_, name) => values?.[name] ?? '')
}

const dates = (name, options) => (formats[name] ??= new Intl.DateTimeFormat(locale, options))

export function day(ms) {
  const sameYear = new Date(ms).getFullYear() === new Date().getFullYear()
  return sameYear ? dates('day', { month: 'short', day: 'numeric' }).format(ms) : dates('date', { year: 'numeric', month: 'short', day: 'numeric' }).format(ms)
}

export const moment = (ms) => dates('moment', { dateStyle: 'medium', timeStyle: 'short' }).format(ms)

export const today = () => dates('today', { weekday: 'long', month: 'long', day: 'numeric' }).format(Date.now())

export function ago(ms) {
  const seconds = (ms - Date.now()) / 1000
  const past = Math.abs(seconds)
  const relative = (formats.ago ??= new Intl.RelativeTimeFormat(locale, { numeric: 'auto', style: 'narrow' }))
  if (past < 45) return relative.format(0, 'second')
  if (past < 3600) return relative.format(Math.trunc(seconds / 60) || -1, 'minute')
  if (past < 86400) return relative.format(Math.trunc(seconds / 3600), 'hour')
  if (past < 7 * 86400) return relative.format(Math.trunc(seconds / 86400), 'day')
  return day(ms)
}
