import { expect, test } from '@playwright/test'
import { readFileSync } from 'node:fs'
import { dictionaries } from '../../../src/i18n/dictionaries'
import { supportedLocales } from '../../../src/models/ui'

const appearance = {
  en: 'Appearance', 'zh-CN': '外观', 'zh-TW': '外觀', ja: '外観', ko: '모양',
  fr: 'Apparence', de: 'Darstellung', es: 'Apariencia', ru: 'Внешний вид', ar: 'المظهر',
}

const placeholders = (text: string) => [...text.matchAll(/\{\{(\w+)\}\}/g)].map(match => match[1]).sort()

for (const locale of supportedLocales) {
  const table: Record<string, string> = locale === 'en' || locale === 'zh-CN'
    ? dictionaries[locale]
    : JSON.parse(readFileSync(new URL(`../../../src/i18n/locales/${locale}.json`, import.meta.url), 'utf8'))

  test(`Desktop dictionary covers the English keys and placeholders in ${locale}`, () => {
    const invalid = Object.entries(dictionaries.en).filter(([key, english]) => {
      const translated = table[key]
      return typeof translated !== 'string' || (english.trim() !== '' && !translated.trim()) || placeholders(english).join() !== placeholders(translated).join()
    })
    expect(invalid.map(([key]) => key)).toEqual([])
  })

  if (locale === 'en') continue
  test(`Settings applies ${locale} to the desktop, search and other apps, and restores it after reload`, async ({ page }) => {
    await page.setViewportSize({ width: 1280, height: 860 })
    await page.goto('/?scenario=normal')
    await page.getByTestId('desktop-app-settings').click()
    const settings = page.getByTestId('window-settings')
    await settings.getByRole('button', { name: 'Appearance', exact: true }).click()
    await settings.getByTestId('settings-language').selectOption(locale)
    await expect(page.locator('html')).toHaveAttribute('lang', locale)
    await expect(page.locator('html')).toHaveAttribute('dir', locale === 'ar' ? 'rtl' : 'ltr')
    await expect(settings.getByRole('button', { name: appearance[locale], exact: true })).toBeVisible()
    await expect(settings.getByText(table['settings.appearance.themeLanguage'], { exact: true })).toBeVisible()
    const search = settings.getByRole('searchbox', { name: table['settings.searchPlaceholder'] })
    await search.fill(table['settings.general.softwareInfo'])
    await settings.getByRole('button').filter({ hasText: table['settings.general.softwareInfo'] }).first().click()
    await expect(settings.getByTestId('settings-software-info')).toContainText(table['settings.general.softwareInfo'])
    await page.reload()
    await expect(page.getByTestId('desktop-app-settings')).toContainText(table['apps.settings'])
    await page.getByTestId('desktop-app-files').click()
    await expect(page.getByTestId('window-files')).toContainText(table['filebrowser.column.name'])
  })
}

test('a failed language pack keeps the selection through preference changes and recovers when online', async ({ page }) => {
  await page.goto('/?scenario=normal')
  await page.getByTestId('desktop-app-settings').click()
  const settings = page.getByTestId('window-settings')
  await settings.getByRole('button', { name: 'Appearance', exact: true }).click()
  const french = /\/locales\/fr\.json(?:\?|$)/
  await page.route(french, route => route.fulfill({ status: 503 }))
  const failed = page.waitForEvent('console', message => message.text().includes('Could not load the fr language pack'))
  await settings.getByTestId('settings-language').selectOption('fr')
  await failed
  await expect(page.locator('html')).toHaveAttribute('lang', 'en')
  await settings.getByTestId('settings-theme').selectOption('dark')
  await expect(settings.getByTestId('settings-language')).toHaveValue('fr')
  expect(await page.evaluate(() => localStorage.getItem('buckyos.prototype.locale.v1'))).toBe('fr')

  await page.unroute(french)
  await page.evaluate(() => window.dispatchEvent(new Event('online')))
  await expect(settings.getByRole('button', { name: 'Apparence', exact: true })).toBeVisible()
  await expect(page.locator('html')).toHaveAttribute('lang', 'fr')
})

test('a late language pack cannot override a more recent selection', async ({ page }) => {
  await page.goto('/?scenario=normal')
  await page.getByTestId('desktop-app-settings').click()
  const settings = page.getByTestId('window-settings')
  await settings.getByRole('button', { name: 'Appearance', exact: true }).click()
  const french = /\/locales\/fr\.json(?:\?|$)/
  let resume!: () => void
  const gate = new Promise<void>(resolve => { resume = resolve })
  await page.route(french, async route => { await gate; await route.continue() })
  const pending = page.waitForRequest(french)
  await settings.getByTestId('settings-language').selectOption('fr')
  await pending
  await settings.getByTestId('settings-language').selectOption('ar')
  await expect(settings.getByRole('button', { name: 'المظهر', exact: true })).toBeVisible()
  const finished = page.waitForResponse(french)
  resume()
  await (await finished).finished()
  await expect(page.locator('html')).toHaveAttribute('lang', 'ar')
  await expect(page.locator('html')).toHaveAttribute('dir', 'rtl')
})

test('a standalone workspace uses the launch language and keeps other URL parameters', async ({ page, context }) => {
  await page.goto('/aiws.html?lang=fr-CA&id=surface-1')
  await expect(page.locator('html')).toHaveAttribute('lang', 'fr')
  await expect(page.getByRole('alert')).toContainText('Cette adresse ne désigne aucun espace de travail')
  await expect(page).toHaveURL(/aiws\.html\?id=surface-1$/)

  const desktop = await context.newPage()
  await desktop.goto('/?scenario=normal')
  await desktop.getByTestId('desktop-app-settings').click()
  const settings = desktop.getByTestId('window-settings')
  await settings.getByRole('button', { name: /^(Apparence|Appearance)$/ }).click()
  await settings.getByTestId('settings-language').selectOption('ar')
  await expect(page.locator('html')).toHaveAttribute('lang', 'ar')
  await expect(page.locator('html')).toHaveAttribute('dir', 'rtl')
  await expect(page.getByRole('alert')).toHaveText(/[\u0600-\u06ff]/)
})
