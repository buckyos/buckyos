/* A public session on a real Zone (第四期规划 P0, P4-15/16/18 on the Zone's own deployment): the room lives on the
 * source workspace's own host (`aiws-<id>.<zone>`), its channel is a WebSocket through the Zone gateway (wss, not the
 * long-poll fallback), the presenter's voice goes through the OOD's media port to an audience member and is
 * recorded, and a second account joins with its own identity through the audience link. Chromium's fake microphone
 * stands for the presenter's. */

import { readFileSync } from 'node:fs'
import { resolve } from 'node:path'
import type { Operation, WorkspaceSummary } from '../../src/app/aiworkspace/api/types'
import { expect, openWorkspace, screenshot, test, workspaceOrigin, type Actor } from './aiworkspace.fixtures'

test.use({ viewport: { width: 1400, height: 900 }, launchOptions: { args: ['--ignore-certificate-errors', '--use-fake-ui-for-media-stream', '--use-fake-device-for-media-stream'] }, permissions: ['microphone'] })

const FIXTURE = resolve('../aiworkspace/fixtures/presentation/commits.json')

async function courseware(actor: Actor) {
  const created = await actor.client.ok<WorkspaceSummary>('ws.create', { title: actor.client.prefix })
  actor.client.owned.add(created.workspace_id)
  const ws = { workspace_id: created.workspace_id, epoch: created.epoch }
  const fixture = JSON.parse(readFileSync(FIXTURE, 'utf8')) as { commits: { operations: Operation[] }[] }
  for (const commit of fixture.commits) expect((await actor.client.commit(ws, commit.operations)).status).toBe('accepted')
  return ws
}

test('[session] a room on the workspace host: wss channel through the gateway, voice through the OOD, an account joins by link', async ({ owner, peer }, info) => {
  test.setTimeout(240_000)
  const ws = await courseware(owner)
  const created = await owner.client.ok<{ session: { session_id: string; workspace_id: string } }>('session.create', { workspace_id: ws.workspace_id, path_id: 'path-intro', title: 'DV 公开课' })
  const sid = created.session.session_id
  const at = (await owner.client.ok<{ invite: { token: string } }>('session.invite', { session_id: sid, kind: 'at' })).invite.token
  const got = await owner.client.ok<{ voice_service: { available: boolean; reason?: string; hosts?: string[] } }>('session.get', { session_id: sid })
  await info.attach('voice-service', { body: Buffer.from(JSON.stringify(got.voice_service)), contentType: 'application/json' })
  expect(got.voice_service.available, JSON.stringify(got.voice_service)).toBe(true)

  // the presenter signs into the workspace's own origin, then opens the room there (same host as the source, §1)
  const origin = workspaceOrigin(owner, ws)
  await openWorkspace(owner, ws)
  await owner.page.goto(`${origin}/w?session=${sid}`)
  await expect(owner.page.getByTestId('aiws-session-presenter')).toHaveAttribute('data-state', 'prepared', { timeout: 30_000 })
  await expect(owner.page.getByTestId('aiws-session-presenter')).toHaveAttribute('data-channel', 'open', { timeout: 30_000 })
  await owner.page.getByTestId('aiws-session-start').click()
  await owner.page.getByTestId('aiws-session-voice').click()
  await expect(owner.page.getByTestId('aiws-session-voice')).toHaveAttribute('aria-pressed', 'true', { timeout: 20_000 })

  // the second account opens the audience link, signs in through the Zone and joins with its own identity
  await peer.page.goto(`${origin}/w?session=${sid}#k=${at}`)
  await expect(peer.page.getByTestId('aiws-session-gate')).toBeVisible({ timeout: 30_000 })
  await peer.page.getByTestId('aiws-session-signin').click()
  await expect(peer.page.getByTestId('aiws-session-gate')).toBeVisible({ timeout: 60_000 })
  await peer.page.getByTestId('aiws-session-join').click()
  await expect(peer.page.getByTestId('aiws-session-audience')).toHaveAttribute('data-channel', 'open', { timeout: 30_000 })
  await expect(peer.page.getByTestId('aiws-session-listen')).toBeVisible({ timeout: 30_000 })
  const joined = await owner.client.ok<{ participants: { principal: string; level: string }[] }>('session.get', { session_id: sid })
  expect(joined.participants.some((p) => p.principal === peer.userId && p.level === 'verified'), JSON.stringify(joined.participants)).toBe(true)

  // a step, followed through the gateway
  await owner.page.getByTestId('aiws-session-next').click()
  await expect(peer.page.getByTestId('aiws-session-stage')).toHaveAttribute('data-step', 's2', { timeout: 15_000 })
  // ten seconds of voice make a recorded segment
  await owner.page.waitForTimeout(12_000)
  await owner.page.getByTestId('aiws-session-voice').click()
  let segments: { data: { media_type: string; t_rel_start: number; t_rel_end: number } }[] = []
  for (let i = 0; i < 40 && segments.length === 0; i++) {
    await owner.page.waitForTimeout(500)
    const events = await owner.client.ok<{ events: { type: string; data: never }[] }>('session.events', { session_id: sid, after: 0, limit: 2000 })
    segments = events.events.filter((e) => e.type === 'voice.segment')
  }
  await screenshot(owner.page, info, 'session-presenter')
  await screenshot(peer.page, info, 'session-audience')
  expect(segments.length).toBeGreaterThan(0)
  expect(segments[0].data.media_type).toBe('audio/ogg')
  // the end and the save; the room is the replay now
  owner.page.once('dialog', (d) => { void d.accept() })
  await owner.page.getByTestId('aiws-session-end').click()
  await expect(owner.page.getByTestId('aiws-session-replay')).toBeVisible({ timeout: 30_000 })
  await owner.page.getByTestId('aiws-session-save').click()
  await expect(owner.page.getByTestId('aiws-session-replay-state')).toHaveText(/已保存|Saved/, { timeout: 30_000 })
  await owner.client.ok('session.delete', { session_id: sid })
})
