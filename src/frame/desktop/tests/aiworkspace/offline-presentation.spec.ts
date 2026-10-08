/* Presentation offline (第三期规划 §8.4, §15 P3-18) and the show lock seen by an offline replica (§8.1, P3-12): with
 * the network really cut, the guide and a local read-only show work in the window that holds the replica, while the
 * prompter link and operable shows say they are unavailable; a replica's edits made during someone's show stay
 * queued and go out once the show ends. Production build behind a TCP relay (offline-fixtures.ts). */

import { readFileSync } from 'node:fs'
import { resolve } from 'node:path'
import { blockCenter } from './fixtures'
import { expect, prepareOffline, saveSummary, test, type Api } from './offline-fixtures'

const ALICE = 'tok-alice'
const FIXTURE = resolve('../aiworkspace/fixtures/presentation/commits.json')

async function showWorkspace(api: Api, title: string) {
  const ws = await api.rpc(ALICE, 'ws.create', { title }) as { workspace_id: string; epoch: string }
  const fixture = JSON.parse(readFileSync(FIXTURE, 'utf8')) as { commits: { operations: Record<string, unknown>[] }[] }
  for (const [index, commit] of fixture.commits.entries()) {
    const result = await api.commit(ALICE, ws, commit.operations, `presentation/${index + 1}`)
    expect(result.status, JSON.stringify(result)).toBe('accepted')
  }
  return ws
}

test('P3-18 offline: the guide and a local read-only show work; the prompter link and operable shows are unavailable', async ({ page, context, api, net }) => {
  const ws = await showWorkspace(api, `p3-18 ${Date.now()}`)
  await prepareOffline(page, net, ALICE, ws.workspace_id)
  await net.down()
  await context.setOffline(true)
  await expect(page.getByTestId('aiws-conn')).toHaveAttribute('data-status', 'offline', { timeout: 40_000 })
  // the guide runs from the replica
  await page.getByTestId('aiws-guide-start').click()
  await expect(page.getByTestId('aiws-guide')).toHaveAttribute('data-step', 'g1')
  await page.keyboard.press('ArrowRight')
  await expect(page.getByTestId('aiws-guide')).toHaveAttribute('data-step', 'g2')
  await page.keyboard.press('Escape')
  await expect(page.getByTestId('aiws-guide')).toHaveCount(0)
  // a show: local, read-only, without a lock, a prompter or a clone — and it says so
  await page.getByTestId('aiws-main-menu').click()
  await page.getByTestId('aiws-top-play').click()
  await expect(page.getByTestId('aiws-start-show')).toContainText('后台当前不可达')
  await page.getByTestId('aiws-start-show-go').click()
  await expect(page.getByTestId('aiws-show')).toBeVisible({ timeout: 30_000 })
  await expect(page.getByTestId('aiws-show-status')).toHaveText('本地放映')
  await expect(page.getByTestId('aiws-show-link')).toBeDisabled()
  await expect(page.getByTestId('aiws-show')).not.toHaveAttribute('data-live', 'true')
  await page.keyboard.press('ArrowRight')
  await expect(page.getByTestId('aiws-show')).toHaveAttribute('data-step', 's2')
  await page.keyboard.press('Escape')
  await expect(page.getByTestId('aiws-show')).toHaveCount(0)
  await context.setOffline(false)
  await net.up()
})

test('P3-12 a replica\'s edits made during a show wait in the queue and are sent once the show ends', async ({ page, api, net }) => {
  const ws = await showWorkspace(api, `p3-12r ${Date.now()}`)
  await prepareOffline(page, net, ALICE, ws.workspace_id)
  // someone presents the workspace (here: alice from another device)
  const show = await api.rpc(ALICE, 'show.start', { workspace_id: ws.workspace_id, path_id: 'path-intro', live: false })
  expect(show.locked).toBe(true)
  const head = await api.headSeq(ALICE, ws.workspace_id)
  // an edit in the replica window: saved on this device, refused by the service while the show runs, kept queued
  const at = await blockCenter(page, 'shape-cover')
  await page.mouse.click(at.x, at.y)
  await page.keyboard.press('ArrowRight')
  await expect.poll(async () => (await saveSummary(page)).pending).toBe(1)
  await expect(page.getByTestId('aiws-show-locked')).toBeVisible({ timeout: 40_000 })
  await page.waitForTimeout(1500)
  expect((await saveSummary(page)).pending).toBe(1)
  expect((await saveSummary(page)).attention).toBe(0)
  expect(await api.headSeq(ALICE, ws.workspace_id)).toBe(head)
  // the show ends: the queued edit goes out, exactly once
  await api.rpc(ALICE, 'show.end', { workspace_id: ws.workspace_id, show_id: show.show_id })
  await expect.poll(async () => (await saveSummary(page)).pending, { timeout: 45_000 }).toBe(0)
  expect(await api.headSeq(ALICE, ws.workspace_id)).toBe(head + 1)
  await expect(page.getByTestId('aiws-show-locked')).toHaveCount(0)
})
