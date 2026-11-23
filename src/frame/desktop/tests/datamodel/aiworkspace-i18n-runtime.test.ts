/* AI Workspace text at run time (src/app/aiworkspace/i18n.ts): a language's table loads on first use, text stays in the
 * language shown before until it is here, plural forms follow the language's CLDR rules, a missing entry reads English.
 *
 *   deno test --no-check --allow-read --allow-env --unstable-sloppy-imports tests/datamodel/aiworkspace-i18n-runtime.test.ts */

import { aiwsLocaleReady, aiwsText, aiwsTextVersion, loadAiwsLocale, sdkLocale, setAiwsLocale, subscribeAiwsText } from '../../src/app/aiworkspace/i18n.ts'

function equal(a: unknown, b: unknown, what = '') { if (a !== b) throw new Error(`${what}${JSON.stringify(a)} !== ${JSON.stringify(b)}`) }

const PAGES = 'aiworkspace.blocks.media.pages'

Deno.test('English and Simplified Chinese are there from the start, with their plural rules', () => {
  setAiwsLocale('en')
  equal(aiwsLocaleReady('en') && aiwsLocaleReady('zh-CN'), true)
  equal(aiwsText().t('aiworkspace.calendar.reminder.hours', { count: 1 }), '1 hour before')
  equal(aiwsText().t('aiworkspace.calendar.reminder.hours', { count: 3 }), '3 hours before')
  equal(aiwsText('zh-CN').t('aiworkspace.calendar.reminder.hours', { count: 1 }), '提前 1 小时')
})

Deno.test('Arabic loads its own strings and number formats', async () => {
  setAiwsLocale('en')
  equal(aiwsText().intl, 'en')
  setAiwsLocale('ar')
  await loadAiwsLocale('ar')
  const text = aiwsText()
  equal(text.intl, 'ar')
  equal(text.t('aiworkspace.common.cancel'), 'إلغاء')
  equal(text.t(PAGES, { count: 1 }), 'صفحة واحدة')
  equal(text.t(PAGES, { count: 2 }), 'صفحتان')
  equal(text.t(PAGES, { count: 7 }), '7 صفحات')
  setAiwsLocale(null)
})

Deno.test('another language keeps the shown text until its table arrives, then tells its listeners', async () => {
  setAiwsLocale('en')
  aiwsText() // English is the language shown
  setAiwsLocale('ru')
  equal(aiwsLocaleReady('ru'), false)
  const before = aiwsTextVersion()
  let told = 0
  const stop = subscribeAiwsText(() => { told += 1 })
  equal(aiwsText().locale, 'en', 'while loading: ')
  await loadAiwsLocale('ru')
  stop()
  equal(told, 1)
  equal(aiwsTextVersion(), before + 1)
  const ru = aiwsText()
  equal(ru.locale, 'ru')
  equal(ru.intl, 'ru')
  // one / few / many (the plain key)
  equal(ru.t(PAGES, { count: 1 }), '1 страница')
  equal(ru.t(PAGES, { count: 3 }), '3 страницы')
  equal(ru.t(PAGES, { count: 5 }), '5 страниц')
  equal(ru.t(PAGES, { count: 21 }), '21 страница')
  equal(ru.template(PAGES, 22), '{{count}} страницы')
  // a key the table lacks reads English
  equal(ru.t('aiworkspace.no.such.key'), 'aiworkspace.no.such.key')
})

Deno.test('French singular covers 0 and 1; Japanese has one form', async () => {
  await Promise.all([loadAiwsLocale('fr'), loadAiwsLocale('ja')])
  equal(aiwsText('fr').t(PAGES, { count: 0 }), '0 page')
  equal(aiwsText('fr').t(PAGES, { count: 2 }), '2 pages')
  const ja = aiwsText('ja')
  equal(ja.t(PAGES, { count: 1 }), ja.t(PAGES, { count: 7 }).replace('7', '1'))
  equal(ja.list(['a', 'b']).includes('a') && ja.list(['a', 'b']) !== 'a, b', true, 'the Japanese list separator: ')
})

Deno.test('the SDK helpers take the product languages', () => {
  equal(sdkLocale('zh-CN'), 'zh')
  equal(sdkLocale('zh-TW'), 'zh-TW')
  equal(sdkLocale('ru'), 'ru')
  equal(sdkLocale('ar'), 'ar')
})
