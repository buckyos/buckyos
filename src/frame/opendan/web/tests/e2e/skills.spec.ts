import { createHash } from 'node:crypto'
import { fileURLToPath } from 'node:url'
import { gzipSync } from 'node:zlib'
import { readFileSync } from 'node:fs'
import { expect, test, type Page } from '@playwright/test'
import { fakeZone, picker, pickerParams, pickLocal, pickZone } from './picker'

// A skill PIKG built by `buckyos pikg pack`: `cmd_skills/hello/SKILL.md` and `plan_skills/hello_plan/SKILL.md`.
const SAMPLE_PIKG = fileURLToPath(new URL('../../../../../../test/skills_pikg_sample/dapp_dist/hello-skills-0.1.0.pikg', import.meta.url))

const tarFile = (path: string, text: string) => {
  const body = Buffer.from(text)
  const header = Buffer.alloc(512)
  header.write(path, 0)
  header.write('0000644', 100)
  header.write(body.length.toString(8).padStart(11, '0'), 124)
  header.write('0', 156)
  header.write('ustar', 257)
  return Buffer.concat([header, body, Buffer.alloc((512 - (body.length % 512)) % 512)])
}

const zipStored = (files: [string, Buffer][]) => {
  const locals: Buffer[] = []
  const central: Buffer[] = []
  let offset = 0
  for (const [name, data] of files) {
    const local = Buffer.alloc(30)
    local.writeUInt32LE(0x04034b50, 0)
    local.writeUInt32LE(data.length, 18)
    local.writeUInt32LE(data.length, 22)
    local.writeUInt16LE(name.length, 26)
    const entry = Buffer.alloc(46)
    entry.writeUInt32LE(0x02014b50, 0)
    entry.writeUInt32LE(data.length, 20)
    entry.writeUInt32LE(data.length, 24)
    entry.writeUInt16LE(name.length, 28)
    entry.writeUInt32LE(offset, 42)
    locals.push(local, Buffer.from(name), data)
    central.push(entry, Buffer.from(name))
    offset += 30 + name.length + data.length
  }
  const directory = Buffer.concat(central)
  const end = Buffer.alloc(22)
  end.writeUInt32LE(0x06054b50, 0)
  end.writeUInt16LE(files.length, 8)
  end.writeUInt16LE(files.length, 10)
  end.writeUInt32LE(directory.length, 12)
  end.writeUInt32LE(offset, 16)
  return Buffer.concat([...locals, directory, end])
}

// A skill PIKG that still brings a command of its own.
const toolsPikg = () => {
  const payload = gzipSync(Buffer.concat([
    tarFile('cmd_skills/hello/SKILL.md', '---\nname: hello\ndescription: Say hello.\n---\n\nRun `hello`.\n'),
    tarFile('tools/hello/bin/hello', '#!/bin/sh\necho hello\n'),
    Buffer.alloc(1024),
  ]))
  const app = { name: 'hello-tools', version: '0.1.0', app_type: 'agent_skills', pkg_list: { agent_skills: { pkg_objid: 'pkg:text' } } }
  const meta = {
    package_objects: { 'pkg:text': { size: payload.length } },
    content_index: { text: { sub_pkg_name: 'agent_skills', path: 'agent_skills.tar.gz', digest: `sha256:${createHash('sha256').update(payload).digest('hex')}` } },
  }
  return {
    name: 'hello-tools-0.1.0.pikg',
    mimeType: 'application/zip',
    buffer: zipStored([['APPDOC.json', Buffer.from(JSON.stringify(app))], ['PACKAGE_META.json', Buffer.from(JSON.stringify(meta))], ['agent_skills.tar.gz', payload]]),
  }
}

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

  // A skill is text: nothing of its own is installed on a runtime, so there is nothing to install or re-check.
  await expect(page.getByTestId('skill-availability-yt_fetch')).toContainText('Commands complete')
  await expect(page.getByTestId('skill-install-yt_fetch')).toHaveCount(0)
  await expect(page.getByRole('button', { name: 'Install commands here' })).toHaveCount(0)
  await expect(page.getByRole('button', { name: 'Re-check' })).toHaveCount(0)

  await page.getByTestId('skills-runtime-select').selectOption('host')
  await expect(page.getByTestId('skill-availability-doc_convert')).toContainText('needs the extended environment')
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
  await fakeZone(page)
  const errors = await open(page)
  const choose = page.getByTestId('skills-pikg-choose')
  await choose.click()
  expect(await pickerParams(page)).toMatchObject({ mode: 'open', allowLocal: true, accept: [{ ext: ['.pikg', '.zip'] }, { mime: 'application/zip' }] })
  await pickLocal(page, { name: 'broken.pikg', mimeType: 'application/zip', buffer: Buffer.from('not a zip at all, just text') })
  await expect(page.getByTestId('skills-install').getByRole('alert')).toContainText('not a skill package')

  // A package that brings a command is passed on as it is: the service's refusal is what the owner reads.
  await choose.click()
  await pickLocal(page, toolsPikg())
  const dialog = page.getByTestId('skill-plan-dialog')
  await expect(dialog.getByRole('alert')).toContainText('`tools/hello/bin/hello`: a skill is text only')
  await expect(dialog.getByTestId('plan-confirm')).toHaveCount(0)
  await dialog.getByRole('button', { name: 'Cancel' }).click()

  await choose.click()
  await pickLocal(page, SAMPLE_PIKG)
  await expect(dialog).toContainText('Install hello-skills 0.1.0?')
  await expect(dialog.getByTestId('plan-skills')).toContainText('work:hello')
  await expect(dialog.getByTestId('plan-skills')).toContainText('plan:hello_plan')
  await expect(dialog.getByTestId('plan-skills')).not.toContainText('Commands it brings')
  await expect(dialog.getByTestId('plan-risk')).toHaveText('Low')
  // Installing copies text: there is no runtime to choose.
  await expect(dialog.locator('select')).toHaveCount(0)
  await expect(dialog).not.toContainText('Where its commands are installed')
  await expect(dialog.getByTestId('plan-text-only')).toContainText('checks the text and copies it')
  await dialog.getByTestId('plan-confirm').click()
  await expect(page.getByTestId('skills-notice')).toHaveText('Installed: hello, hello_plan')
  const hello = page.getByTestId('skill-work-hello')
  await expect(hello).toContainText('Installed')
  await expect(hello).toContainText('hello-skills 0.1.0')

  // The same package again replaces the skill, and says so.
  await choose.click()
  await pickLocal(page, SAMPLE_PIKG)
  await expect(dialog.getByTestId('plan-replaces').first()).toContainText('replaces the installed skill')
  await expect(dialog.getByTestId('plan-risk')).toHaveText('Medium')
  await expect(dialog.getByTestId('plan-confirm')).toHaveText('Install and replace')
  await dialog.getByTestId('plan-confirm').click()
  await expect(dialog).toHaveCount(0)
  expect(errors).toEqual([])
})


test('a .pikg in the Zone is chosen in the system picker and read with the page session', async ({ page }) => {
  await fakeZone(page, { '/home/devtest/Downloads/hello-skills-0.1.0.pikg': readFileSync(SAMPLE_PIKG) })
  const errors = await open(page)
  await page.getByTestId('skills-pikg-choose').click()
  await pickZone(page, '/home/devtest/Downloads/hello-skills-0.1.0.pikg')
  const dialog = page.getByTestId('skill-plan-dialog')
  await expect(dialog).toContainText('Install hello-skills 0.1.0?')
  await expect(dialog.getByTestId('plan-skills')).toContainText('work:hello')
  await dialog.getByRole('button', { name: 'Cancel' }).click()
  await expect(page.locator('[data-buckyos-sysdlg]')).toHaveCount(0)

  // Cancelling the picker changes nothing.
  await page.getByTestId('skills-pikg-choose').click()
  await picker(page).getByRole('button', { name: 'Answer cancel' }).click()
  await expect(page.locator('[data-buckyos-sysdlg]')).toHaveCount(0)
  await expect(page.getByTestId('skills-pikg-choose')).toBeEnabled()
  await expect(dialog).toHaveCount(0)
  expect(errors).toEqual([])
})

test('in a Desktop window the text import asks the shell for the picker', async ({ page }) => {
  await fakeZone(page, { '/home/devtest/notes/invoice.md': Buffer.from('# Invoices\n\nFile them monthly.\n') })
  await page.goto('/tests/harness/shell.html?data=mock&lang=en')
  await page.waitForFunction(() => (window as unknown as { frameReady?: boolean }).frameReady === true)
  const frame = page.frame({ url: /bfp=/ })!
  await frame.evaluate(() => { location.hash = '#/skills' })
  const app = page.frameLocator('#app')
  await app.getByTestId('skills-import-file').click()
  await page.waitForFunction(() => (window as unknown as { dialogs: unknown[] }).dialogs.length === 1)
  expect(await page.evaluate(() => (window as unknown as { dialogs: unknown[] }).dialogs[0])).toMatchObject({ path: 'sysdlg/select', params: { mode: 'open', allowLocal: true } })
  // The shell draws the dialog over the window: nothing is framed over the WebUI itself.
  await expect(app.locator('[data-buckyos-sysdlg]')).toHaveCount(0)
  await page.evaluate(() => (window as unknown as { answerDialog: (r: unknown) => void }).answerDialog({ action: 'selected', items: [{ origin: 'zone', kind: 'file', source: { kind: 'cyfs-path', path: 'cyfs:///home/devtest/notes/invoice.md' }, hints: { name: 'invoice.md' } }] }))
  const panel = app.getByTestId('skills-import')
  await expect(panel.getByRole('textbox').first()).toHaveValue('# Invoices\n\nFile them monthly.\n')
  await expect(panel).toContainText('invoice.md')
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
  await expect(dialog.getByTestId('plan-text-only')).toContainText('no script runs and no command is installed')
  await expect(dialog.locator('select')).toHaveCount(0)
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

test('the owner switches the skill crystallization goal on and off', async ({ page }) => {
  const errors = await open(page)
  const panel = page.getByTestId('skills-crystallize')
  const toggle = panel.getByTestId('skills-crystallize-switch')
  await expect(panel).toContainText('at most once a day')
  await expect(panel).toContainText('One look costs about 180,000 tokens (measured 2026-11-10).')
  await expect(toggle).toHaveAttribute('role', 'switch')
  await expect(toggle).not.toBeChecked()
  await expect(panel.getByTestId('skills-crystallize-state')).toHaveText('Off')
  await expect(panel.getByTestId('skills-crystallize-source')).toContainText('the agent package’s default applies (off in Jarvis)')

  await toggle.click()
  await expect(toggle).toBeChecked()
  await expect(panel.getByTestId('skills-crystallize-state')).toHaveText('On')
  await expect(panel.getByTestId('skills-crystallize-source')).toHaveText('Set by the owner.')
  await page.getByTestId('skills-audit-toggle').click()
  await expect(page.getByTestId('audit-row').first()).toContainText('crystallize_on')

  await toggle.click()
  await expect(toggle).not.toBeChecked()
  await expect(panel.getByTestId('skills-crystallize-state')).toHaveText('Off')
  await expect(page.getByTestId('audit-row').first()).toContainText('crystallize_off')
  await page.goto('/?data=mock&lang=zh#/skills')
  await expect(page.getByTestId('skills-crystallize')).toContainText('经验结晶')
  await expect(page.getByTestId('skills-crystallize')).toContainText('18 万 tokens')
  expect(errors).toEqual([])
})
