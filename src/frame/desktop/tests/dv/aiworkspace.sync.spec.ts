/* A sync task of the owner's Agent on a real Zone (AI Native 余下任务 §8.2): installed on the Agent (`sync.put`), it runs a
 * Deno program with the declared hosts only and no environment or files; a declared credential missing is an `auth`
 * failure; the owner enters it (never shown again, never in the task's records); a data source bound to the task
 * refreshes through the Agent (`sync.run` by the AIWS service) and becomes a data version of the program; a disabled
 * task makes the refresh `SYNC_UNAVAILABLE` with the last version kept. The Agent is `AIWS_DV_AGENT` (one of the signed-in
 * user's Agents). */

import { readdirSync, readFileSync } from 'node:fs'
import { join } from 'node:path'
import type { Operation, WorkspaceSummary } from '../../src/app/aiworkspace/api/types'
import { expect, test, type Actor } from './aiworkspace.fixtures'

type Workspace = Pick<WorkspaceSummary, 'workspace_id' | 'epoch'>
// eslint-disable-next-line @typescript-eslint/no-explicit-any
type Json = any

const ROOT = process.env.BUCKYOS_ROOT ?? '/opt/buckyos'
const AGENT = process.env.AIWS_DV_AGENT ?? 'browsy.test.buckyos.io'
const SECRET = `dv-secret-${Date.now()}`

const PROGRAM = `
const input = JSON.parse(await new Response(Deno.stdin.readable).text())
const probe = async (f) => { try { return String(await f()) } catch (e) { return (e && e.name) || String(e) } }
const records = [
  { check: 'env', result: await probe(() => Deno.env.get('BUCKYOS_APP_TOKEN') ?? 'none') },
  { check: 'file', result: await probe(() => Deno.readTextFile('/opt/buckyos/etc/node_identity.json').then(() => 'read')) },
  { check: 'other_host', result: await probe(() => fetch('https://example.org/').then((r) => r.status)) },
  { check: 'allowed_host', result: await probe(() => fetch('https://example.com/').then((r) => r.status)) },
  { check: 'secret', result: input.secrets && input.secrets.API_KEY ? 'given' : 'missing' },
]
console.log(JSON.stringify({ status: 'ok', business_time: '2026-11-16T00:00:00Z', records }))
`

async function agent<T = Json>(owner: Actor, method: string, params: Json): Promise<T> {
  return owner.client.service<T>(AGENT, method, params)
}

async function refresh(owner: Actor, ws: Workspace, key: string): Promise<Json> {
  const started = await owner.client.ok<Json>('proc.start', { workspace_id: ws.workspace_id, program: 'source.refresh@1', params: { entity_id: 'probe' }, idempotency_key: key })
  let run: Json
  await expect.poll(async () => { run = await owner.client.ok<Json>('proc.get', { workspace_id: ws.workspace_id, run_id: started.run_id }); return run.state }, { timeout: 240_000, intervals: [3_000] }).toMatch(/succeeded|failed/)
  return run
}

test('[sync] the owner\'s Agent runs a sync task with its declared hosts only; its credential stays out of everything; a data source refreshes through it', async ({ owner }) => {
  test.setTimeout(900_000)
  const task = `dv${Date.now().toString(36)}`
  const owned = owner.userId
  const dir = join(ROOT, 'data/home', owned, 'agents', AGENT, 'sync', task)
  try {
    const put = await agent(owner, 'sync.put', { task_id: task, title: 'DV probe', program: PROGRAM, hosts: ['example.com'], schedule: { every_secs: 900 }, secrets: ['API_KEY'] })
    expect(put.output.path).toBe(`/home/${owned}/agents/${AGENT}/sync/${task}/out.json`)
    // the credential is declared, not given: an auth failure, the program does not start
    const noKey = await agent(owner, 'sync.run', { task_id: task, wait_ms: 120_000 })
    expect(noKey.run, JSON.stringify(noKey)).toMatchObject({ status: 'failed', kind: 'auth' })
    await agent(owner, 'sync.set_secret', { task_id: task, name: 'API_KEY', value: SECRET })
    expect((await agent(owner, 'sync.get', { task_id: task })).secrets).toEqual([{ name: 'API_KEY', set: true }])

    // a data source bound to the task refreshes through the Agent
    const created = await owner.client.ok<WorkspaceSummary>('ws.create', { title: owner.client.prefix })
    owner.client.owned.add(created.workspace_id)
    const ws: Workspace = { workspace_id: created.workspace_id, epoch: created.epoch }
    const r = await owner.client.commit(ws, [{ op: 'entity.create', entity_id: 'probe', type_id: 'buckyos.table-source', parent_id: 'data', order_key: 'p', name: 'Probe',
      payload: { fields: [{ field_id: 'check', name: 'Check', type: 'text' }, { field_id: 'result', name: 'Result', type: 'text' }], title_field_id: 'check',
        source: { kind: 'zone', path: put.output.path, sync: { agent_id: AGENT, task_id: task }, def: { meaning: 'What a sync program may reach', primary_key: ['check'] } } } } as Operation])
    expect(r.status, JSON.stringify(r)).toBe('accepted')
    const done = await refresh(owner, ws, `${task}-1`)
    expect(done.state, JSON.stringify(done)).toBe('succeeded')
    const meta = await owner.client.ok<Json>('doc.read', { workspace_id: ws.workspace_id, entity_id: 'probe', as: 'meta' })
    expect(meta.data).toMatchObject({ state: 'valid', sync: { agent_id: AGENT, task_id: task } })
    expect(meta.data.version.program).toMatchObject({ name: task })
    const rows = await owner.client.ok<Json>('doc.query', { workspace_id: ws.workspace_id, sql: 'SELECT "Check", "Result" FROM "/data/Probe" ORDER BY 1' })
    const result = Object.fromEntries((rows.rows as [string, string][]).map(([k, v]) => [k, v]))
    // no environment, no files, no host that was not declared; the declared one is reached; the credential arrived
    expect(result.env, JSON.stringify(result)).toBe('NotCapable')
    expect(result.file).toBe('NotCapable')
    expect(result.other_host).toBe('NotCapable')
    expect(result.allowed_host).toBe('200')
    expect(result.secret).toBe('given')
    // the credential's value is in none of the task's files
    for (const f of readdirSync(dir)) expect(readFileSync(join(dir, f), 'utf8').includes(SECRET), f).toBe(false)

    // a disabled task: the refresh says why, the last version stays
    await agent(owner, 'sync.set_enabled', { task_id: task, enabled: false })
    const off = await refresh(owner, ws, `${task}-2`)
    expect(off.state).toBe('failed')
    expect(JSON.stringify(off)).toContain('SYNC_UNAVAILABLE')
    expect((await owner.client.ok<Json>('doc.read', { workspace_id: ws.workspace_id, entity_id: 'probe', as: 'meta' })).data.state).toBe('valid')
  } finally {
    await agent(owner, 'sync.delete', { task_id: task }).catch(() => undefined)
  }
})
