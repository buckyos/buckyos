import { fileURLToPath } from 'node:url'
import { expect, test, type Page } from '@playwright/test'

// A skill PIKG built by `buckyos pikg pack`: `cmd_skills/hello/SKILL.md` and `tools/hello/bin/hello`.
const SAMPLE_PIKG = fileURLToPath(new URL('../../../../../../test/skills_pikg_sample/dapp_dist/hello-skills-0.1.0.pikg', import.meta.url))

const open = async (page: Page, hash = '#/skills') => {
  const errors: string[] = []
  page.on('pageerror', (e) => errors.push(e.message))
  await page.goto(`/?data=mock&lang=en${hash}`)
  await expect(page.getByTestId('data-source')).toHaveText('data: mock')
  return errors
}

test('home counts the skills and links to the page', async ({ page }) => {
  const errors = await open(page, '')
  const card = page.getByTestId('skills-card')
  await expect(card.getByTestId('skills-card-summary')).toHaveText('7 available / 1 disabled')
  await card.getByRole('link').click()
  await expect(page.getByTestId('skills-page')).toBeVisible()
  await expect(page.getByTestId('skills-summary')).toHaveText('7 available / 1 disabled')
  expect(errors).toEqual([])
})

test('skills are grouped by kind with the availability of the selected runtime', async ({ page }) => {
  const errors = await open(page)
  await expect(page.getByTestId('skills-work').locator('tbody tr')).toHaveCount(7)
  await expect(page.getByTestId('skills-plan').locator('tbody tr')).toHaveCount(1)
  await expect(page.getByTestId('skills-goal')).toHaveCount(0)
  await expect(page.getByTestId('skills-runtime-select')).toHaveValue('private')

  const old = page.getByTestId('skill-work-old_invoice_flow')
  await expect(old).toContainText('Disabled')
  await expect(old.getByTestId('skill-disabled-reason')).toContainText('the accounting tool changed')
  await expect(page.getByTestId('skill-work-share')).toContainText('ui,work')
  await expect(page.getByTestId('skill-work-share')).toContainText('1 references')

  // The three levels, on the private runtime.
  await expect(page.getByTestId('skill-availability-win_registry')).toContainText('Not available')
  await expect(page.getByTestId('skill-availability-win_registry')).toContainText('works on windows only')
  const convert = page.getByTestId('skill-availability-doc_convert')
  await expect(convert).toContainText('Some steps lack commands')
  await expect(convert.getByTestId('skill-missing')).toHaveText('Missing: pandoc')
  await expect(convert.getByTestId('skill-recommended')).toContainText('ext-docs 0.3.1 (brings pandoc)')
  await expect(convert).toContainText('Requires: the extended environment')
  const audio = page.getByTestId('skill-availability-aigc_audio')
  await expect(audio).toContainText('Commands complete')
  await expect(audio).toContainText('owner’s own installation')
  await expect(page.getByTestId('skill-availability-ui_core')).toContainText('The manual names no commands.')

  // A skill that installs commands of its own: where they stand, and the two actions.
  const fetch = page.getByTestId('skill-install-yt_fetch')
  await expect(fetch).toContainText('stale')
  await expect(fetch).toContainText('volatile')
  await expect(fetch).toContainText('yt-dlp: not found')
  await fetch.getByRole('button', { name: 'Install commands here' }).click()
  await expect(fetch).toContainText('ok')
  await expect(fetch).not.toContainText('stale')

  await page.getByTestId('skills-runtime-select').selectOption('host')
  await expect(page.getByTestId('skill-availability-doc_convert')).toContainText('needs the extended environment')
  await expect(page.getByTestId('skill-install-yt_fetch')).toContainText('not installed on this runtime')
  expect(errors).toEqual([])
})

test('disable with a reason, enable, and remove an installed skill', async ({ page }) => {
  const errors = await open(page)
  const share = page.getByTestId('skill-work-share')
  await share.getByRole('button', { name: 'Disable' }).click()
  const dialog = page.getByRole('dialog')
  await dialog.getByLabel('Reason (optional)').fill('not sharing for now')
  await dialog.getByRole('button', { name: 'Disable' }).click()
  await expect(share.getByTestId('skill-disabled-reason')).toContainText('not sharing for now')
  await expect(page.getByTestId('skills-summary')).toHaveText('6 available / 2 disabled')
  await share.getByRole('button', { name: 'Enable' }).click()
  await expect(share.getByTestId('skill-disabled-reason')).toHaveCount(0)

  // Only installed and imported skills can be removed.
  await expect(page.getByTestId('skill-work-ui_core').getByRole('button', { name: 'Remove' })).toHaveCount(0)
  await page.getByTestId('skill-work-old_invoice_flow').getByRole('button', { name: 'Remove' }).click()
  await expect(page.getByRole('dialog')).toContainText('cannot be undone')
  await page.getByRole('dialog').getByRole('button', { name: 'Remove' }).click()
  await expect(page.getByTestId('skill-work-old_invoice_flow')).toHaveCount(0)

  await page.getByTestId('skills-audit-toggle').click()
  const rows = page.getByTestId('audit-row')
  await expect(rows.first()).toContainText('remove')
  await expect(rows.first()).toContainText('work:old_invoice_flow')
  expect(errors).toEqual([])
})

test('the UI set is edited and reset to the default', async ({ page }) => {
  const errors = await open(page)
  const set = page.getByTestId('skills-ui-set')
  await expect(set.getByTestId('skills-ui-source')).toContainText('default')
  await expect(set.getByTestId('ui-skill-ui_core')).toBeVisible()
  await expect(set).toContainText('conversations started from now on')
  await set.getByRole('button', { name: 'Add', exact: true }).click()
  await expect(set.getByTestId('ui-skill-share')).toBeVisible()
  await expect(set.getByTestId('skills-ui-source')).toContainText('set by the owner · 8.4 KB')
  await set.getByRole('button', { name: 'Remove ui_core' }).click()
  await expect(set.getByTestId('ui-skill-ui_core')).toHaveCount(0)
  await set.getByRole('button', { name: 'Reset to the default' }).click()
  await expect(set.getByTestId('ui-skill-ui_core')).toBeVisible()
  await expect(set.getByTestId('ui-skill-share')).toHaveCount(0)
  expect(errors).toEqual([])
})

test('a .pikg file is read in the browser, planned, confirmed and installed', async ({ page }) => {
  const errors = await open(page)
  await page.getByTestId('skills-pikg-input').setInputFiles({ name: 'broken.pikg', mimeType: 'application/zip', buffer: Buffer.from('not a zip at all, just text') })
  await expect(page.getByTestId('skills-install').getByRole('alert')).toContainText('not a skill package')

  await page.getByTestId('skills-pikg-input').setInputFiles(SAMPLE_PIKG)
  const dialog = page.getByTestId('skill-plan-dialog')
  await expect(dialog).toContainText('Install hello-skills 0.1.0?')
  await expect(dialog.getByTestId('plan-skills')).toContainText('work:hello')
  await expect(dialog.getByTestId('plan-skills')).toContainText('plan:hello_plan')
  await expect(dialog.getByTestId('plan-skills')).toContainText('Commands it brings: hello')
  await expect(dialog.getByTestId('plan-risk')).toHaveText('Low')
  await expect(dialog.getByLabel('Where its commands are installed')).toHaveValue('private')
  // The owner's machine grants no installation: it is not offered, with the reason.
  await expect(dialog.getByLabel('Where its commands are installed').locator('option')).toHaveCount(1)
  await expect(dialog).toContainText('grants `none`')
  await dialog.getByTestId('plan-confirm').click()
  await expect(page.getByTestId('skills-notice')).toHaveText('Installed: hello, hello_plan')
  const hello = page.getByTestId('skill-work-hello')
  await expect(hello).toContainText('Installed')
  await expect(hello).toContainText('hello-skills 0.1.0')

  // The same package again replaces the skill, and says so.
  await page.getByTestId('skills-pikg-input').setInputFiles(SAMPLE_PIKG)
  await expect(dialog.getByTestId('plan-replaces').first()).toContainText('replaces the installed skill')
  await expect(dialog.getByTestId('plan-risk')).toHaveText('Medium')
  await expect(dialog.getByTestId('plan-confirm')).toHaveText('Install and replace')
  await dialog.getByTestId('plan-confirm').click()
  await expect(dialog).toHaveCount(0)
  expect(errors).toEqual([])
})

test('drafts are viewed, installed after the plan, or discarded; an import starts a session', async ({ page }) => {
  const errors = await open(page)
  const drafts = page.getByTestId('skills-drafts')
  await expect(drafts.locator('li')).toHaveCount(2)
  await expect(drafts.getByTestId('draft-broken-notes').getByTestId('draft-error')).toHaveText('draft.json is missing')
  await expect(drafts.getByTestId('draft-broken-notes').getByRole('button', { name: 'Install' })).toBeDisabled()

  const minutes = drafts.getByTestId('draft-meeting-minutes')
  await minutes.getByRole('button', { name: 'View' }).click()
  await expect(minutes.getByTestId('draft-risk')).toContainText('Sending the mail cannot be undone')
  await minutes.getByRole('button', { name: 'cmd_skills/meeting_minutes/SKILL.md' }).click()
  await expect(minutes).toContainText('Mail them to the attendees')

  await minutes.getByRole('button', { name: 'Install' }).click()
  const dialog = page.getByTestId('skill-plan-dialog')
  await expect(dialog.getByTestId('plan-findings')).toContainText('imported_text')
  await expect(dialog).toContainText('The package installs no commands')
  await expect(dialog).toContainText('Missing: pandoc')
  await dialog.getByTestId('plan-confirm').click()
  await expect(page.getByTestId('skill-work-meeting_minutes')).toContainText('Imported')
  await expect(drafts.getByTestId('draft-meeting-minutes')).toHaveCount(0)

  await drafts.getByTestId('draft-broken-notes').getByRole('button', { name: 'Discard' }).click()
  await page.getByRole('dialog').getByRole('button', { name: 'Discard' }).click()
  await expect(drafts.locator('li')).toHaveCount(0)

  const form = page.getByTestId('skills-import')
  await form.getByLabel('Text to import').fill('How to rotate the logs\n1. stop the service')
  await form.getByLabel('Name (optional)').fill('log-rotation')
  await form.getByRole('button', { name: 'Import', exact: true }).click()
  await expect(form.getByTestId('skills-import-started').getByRole('link')).toHaveAttribute('href', '#/session/work-b41d09e2')
  await expect(drafts.getByTestId('draft-log-rotation')).toBeVisible()
  expect(errors).toEqual([])
})
