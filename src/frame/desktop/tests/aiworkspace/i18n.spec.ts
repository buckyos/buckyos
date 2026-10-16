/* The English UI (the Desktop's default language): AI Workspace shows no Chinese — the list, the workspace chrome,
 * menus, panels, dialogs, the insert catalog, data sources, presentation editing, and the demo content and Mock wish
 * output it creates in English. The rest of the suite runs in zh-CN (fixtures.ts); the source-level guard is
 * tests/datamodel/aiworkspace-i18n.test.ts. */

import type { Page } from '@playwright/test'
import { backToList, expect, LOCALE_STORAGE_KEY, openApp, openSide, openStatus, setCanvasMode, test } from './fixtures'

const ALICE = 'tok-alice'
const CJK = /[\u3000-\u303f\u3400-\u4dbf\u4e00-\u9fff\uff00-\uffef]/

/** Content shown as is: workspace titles on the list (other specs name theirs in Chinese) and group titles — a
 * wish's result group is named by the backend, whose text is not localised yet (README "Language"). */
const USER_CONTENT = 'script, style, .aiws-card-title, .aiws-group-title'

/** Chinese text anywhere on the page: text nodes, accessible names, titles, placeholders, same-origin frames. */
async function chineseOnPage(page: Page): Promise<string[]> {
  const found: string[] = []
  for (const frame of page.frames()) {
    const texts = await frame.evaluate(([source, skip]: string[]) => {
      const cjk = new RegExp(source)
      const out = new Set<string>()
      const walker = document.createTreeWalker(document.body, NodeFilter.SHOW_TEXT)
      for (let node = walker.nextNode(); node; node = walker.nextNode()) {
        const text = node.textContent ?? ''
        if (cjk.test(text) && node.parentElement?.closest(skip) == null) out.add(text.trim().slice(0, 80))
      }
      for (const el of document.querySelectorAll('[aria-label], [title], [placeholder], [alt], [aria-description]')) {
        for (const name of ['aria-label', 'title', 'placeholder', 'alt', 'aria-description']) {
          const value = el.getAttribute(name)
          if (value && cjk.test(value)) out.add(`${name}=${value.slice(0, 80)}`)
        }
      }
      return [...out]
    }, [CJK.source, USER_CONTENT]).catch(() => [] as string[])
    found.push(...texts)
  }
  return found
}

async function expectEnglish(page: Page, where: string) {
  expect(await chineseOnPage(page), `Chinese text on screen: ${where}`).toEqual([])
}

/** The Desktop's language setting, changed live while the workspace stays open: minimise the AI Workspace
 * window, Settings → Appearance → Language, close Settings, bring the workspace back. */
async function switchLanguage(page: Page, locale: 'en' | 'zh-CN') {
  const workspace = page.getByTestId('window-aiworkspace')
  await workspace.getByRole('button', { name: /^(Minimize|最小化)$/ }).click()
  await page.getByTestId('desktop-app-settings').click()
  const settings = page.getByTestId('window-settings')
  await settings.getByRole('button', { name: /^(Appearance|外观)$/, exact: true }).click()
  await settings.getByTestId('settings-language').selectOption(locale)
  await settings.getByRole('button', { name: /^(Close|关闭)$/ }).first().click()
  await expect(settings).toHaveCount(0)
  await page.getByTestId('desktop-app-aiworkspace').click()
  await expect(page.getByTestId('aiws-workspace')).toBeVisible()
}

test.beforeEach(async ({ page }) => {
  // after the suite's zh-CN storage state: this spec is about the Desktop's default language
  await page.addInitScript((key) => window.localStorage.setItem(key, 'en'), LOCALE_STORAGE_KEY)
})

test('i18n: in English, the list, a demo created from a template, its chrome, panels and wish output show no Chinese', async ({ page }) => {
  test.setTimeout(240_000)
  await openApp(page, ALICE)
  await expectEnglish(page, 'workspace list')

  // the template dialog, then the quarterly demo created in English
  await page.getByTestId('aiws-new-template').click()
  await page.getByTestId('aiws-template-quarterly').click()
  await expectEnglish(page, 'template dialog')
  await page.getByLabel('Workspace title', { exact: true }).fill(`i18n quarterly ${Date.now()}`)
  await page.getByTestId('aiws-create-template').click()
  await expect(page.getByTestId('aiws-workspace')).toBeVisible({ timeout: 60_000 })
  await expect(page.getByTestId('aiws-canvas-block-blk-wish')).toBeVisible({ timeout: 30_000 })
  await expect(page.getByTestId('aiws-conn')).toHaveAttribute('data-status', 'live')
  await expectEnglish(page, 'quarterly demo canvas')

  // the main menu and its submenus
  await page.getByTestId('aiws-main-menu').click()
  await expectEnglish(page, 'main menu')
  for (const submenu of ['aiws-menu-view', 'aiws-menu-canvas-mode', 'aiws-menu-dev']) {
    await page.getByTestId(submenu).click()
    await expectEnglish(page, `main menu → ${submenu}`)
  }
  await page.keyboard.press('Escape')
  await page.keyboard.press('Escape')

  // the side panel tabs and the status area
  for (const tab of ['inspector', 'relations', 'annotations', 'collab', 'edits'] as const) {
    await openSide(page, tab)
    await expectEnglish(page, `side panel: ${tab}`)
  }
  await page.getByTestId('aiws-side-close').click()
  await openStatus(page)
  await expectEnglish(page, 'status area')
  await page.getByTestId('aiws-status').click()

  // the insert catalog, every tab
  await page.getByTestId('aiws-tool-catalog').click()
  await expect(page.getByTestId('aiws-catalog')).toBeVisible()
  const tabs = page.locator('[data-testid^="aiws-catalog-tab-"]')
  for (let i = 0; i < await tabs.count(); i += 1) {
    await tabs.nth(i).click()
    await expectEnglish(page, `insert catalog tab ${i}`)
  }
  await page.keyboard.press('Escape')

  // the Mock wish: analysis, candidate and applied results are English too
  await page.getByTestId('aiws-canvas-block-blk-wish').dblclick()
  await page.getByTestId('aiws-wish-analyze').click()
  await expect(page.getByTestId('aiws-wish-analysis')).toBeVisible()
  await page.getByTestId('aiws-wish-execute').click()
  await expect(page.getByTestId('aiws-wish-candidate')).toHaveAttribute('data-ready', 'true', { timeout: 30_000 })
  await expectEnglish(page, 'wish candidate')
  await page.getByTestId('aiws-wish-apply').click()
  await expect(page.getByTestId('aiws-wish-message')).toBeVisible()
  await expectEnglish(page, 'wish applied')

  // presentation path editing and view mode
  await setCanvasMode(page, 'presentation_edit')
  await expectEnglish(page, 'presentation path editor')
  await setCanvasMode(page, 'view')
  await expectEnglish(page, 'view mode')
  await setCanvasMode(page, 'edit')

  // the data sources mode
  await page.getByTestId('aiws-top-sources').click()
  await expectEnglish(page, 'data sources')
})

test('i18n: the film demo in English; a live language switch re-renders the open workspace both ways; the choice persists', async ({ page }) => {
  test.setTimeout(180_000)
  await openApp(page, ALICE)
  await page.getByTestId('aiws-new-template').click()
  await page.getByTestId('aiws-template-film').click()
  await page.getByLabel('Workspace title', { exact: true }).fill(`i18n film ${Date.now()}`)
  await page.getByTestId('aiws-create-template').click()
  await expect(page.getByTestId('aiws-canvas-block-blk-wish-1')).toBeVisible({ timeout: 60_000 })
  await expectEnglish(page, 'film demo canvas')

  // a live switch in Settings → Appearance re-renders the open (minimised) workspace, both ways
  await switchLanguage(page, 'zh-CN')
  await expect(page.getByRole('toolbar', { name: '主工具栏' })).toBeVisible()
  await expect(page.getByTestId('aiws-status')).toContainText(CJK)
  await switchLanguage(page, 'en')
  await expect(page.getByRole('toolbar', { name: 'Main toolbar' })).toBeVisible()
  await expectEnglish(page, 'after switching back to English without a reload')

  // a stored choice survives a reload (init scripts run in order: this one after the spec's English)
  await page.addInitScript((key) => window.localStorage.setItem(key, 'zh-CN'), LOCALE_STORAGE_KEY)
  await page.reload()
  await page.getByTestId('desktop-app-aiworkspace').click()
  await backToList(page)
  await expect(page.getByTestId('aiws-new-template')).toContainText(CJK)
  await expect(page.getByTestId('aiws-list')).toContainText(/i18n film/)
})
