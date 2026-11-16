/* A data source a sync program fills, on a real Zone (AI Native 余下任务 B3-1, B8-2): the definition comes first and is
 * pending (no valid data, SQL refuses it), the program's `aiws.sync.v1` file in the owner's home is read as the owner on
 * a refresh and becomes a data version with its business time; a failed program keeps the last good version and says
 * why; the same file again changes nothing (no duplicate records); a clone keeps the data and definition. The program
 * itself is stood in for by this test writing its file (where it runs is not chosen yet, B1-1). */

import { mkdirSync, renameSync, rmSync, writeFileSync } from 'node:fs'
import { join } from 'node:path'
import type { Operation, WorkspaceSummary } from '../../src/app/aiworkspace/api/types'
import { expect, test, type Actor } from './aiworkspace.fixtures'

type Workspace = Pick<WorkspaceSummary, 'workspace_id' | 'epoch'>
// eslint-disable-next-line @typescript-eslint/no-explicit-any
type Json = any

const ROOT = process.env.BUCKYOS_ROOT ?? '/opt/buckyos'

/** The program's output, written whole (a temporary file renamed over it). */
function writeOutput(host: string, body: Json) {
  writeFileSync(`${host}.tmp`, JSON.stringify(body))
  renameSync(`${host}.tmp`, host)
}

async function refresh(owner: Actor, ws: Workspace, key: string): Promise<Json> {
  const started = await owner.client.ok<Json>('proc.start', { workspace_id: ws.workspace_id, program: 'source.refresh@1', params: { entity_id: 'quotes' }, idempotency_key: key })
  let run: Json
  await expect.poll(async () => { run = await owner.client.ok<Json>('proc.get', { workspace_id: ws.workspace_id, run_id: started.run_id }); return run.state }, { timeout: 60_000 }).toMatch(/succeeded|failed/)
  return run
}

async function meta(owner: Actor, ws: Workspace, id = 'quotes'): Promise<Json> {
  return owner.client.ok<Json>('doc.read', { workspace_id: ws.workspace_id, entity_id: id, as: 'meta' })
}

test('[sources] a sync program\'s file in the owner\'s home: pending until the first sync, a data version with its business time, a failure keeps the last version, a repeat changes nothing, a clone keeps it', async ({ owner }) => {
  const created = await owner.client.ok<WorkspaceSummary>('ws.create', { title: owner.client.prefix })
  owner.client.owned.add(created.workspace_id)
  const ws: Workspace = { workspace_id: created.workspace_id, epoch: created.epoch }
  const folder = `aiws-dv/${owner.client.prefix}`
  const hostDir = join(ROOT, 'data/home', owner.userId, folder)
  mkdirSync(hostDir, { recursive: true })
  const host = join(hostDir, 'quotes.json')
  try {
    const r = await owner.client.commit(ws, [{ op: 'entity.create', entity_id: 'quotes', type_id: 'buckyos.table-source', parent_id: 'data', order_key: 'q', name: 'Quotes',
      payload: { fields: [{ field_id: 'symbol', name: 'Symbol', type: 'text' }, { field_id: 'close', name: 'Close', type: 'number' }], title_field_id: 'symbol',
        source: { kind: 'zone', path: `/home/${owner.userId}/${folder}/quotes.json`, def: { meaning: 'Closing prices of the watch list', primary_key: ['symbol'], freshness: { max_age_s: 86400 } } } } } as Operation])
    expect(r.status, JSON.stringify(r)).toBe('accepted')

    // the definition exists, the data does not: pending, never an empty table
    let m = await meta(owner, ws)
    expect(m.data?.state, JSON.stringify(m).slice(0, 1500)).toBe('pending')
    const sql = await owner.client.call<Json>('doc.query', { workspace_id: ws.workspace_id, sql: 'SELECT * FROM "/data/Quotes"' }).catch((e: unknown) => e)
    expect(JSON.stringify(sql)).toContain('DATA_PENDING')

    // the program wrote its first output: the refresh reads it as the owner
    writeOutput(host, { format: 'aiws.sync.v1', status: 'ok', program: { name: 'quotes', version: '1' }, business_time: '2026-11-14T21:00:00Z', produced_at: '2026-11-15T08:00:00Z',
      records: [{ symbol: 'AAPL', close: 190.5 }, { symbol: 'MSFT', close: 410.25 }] })
    const first = await refresh(owner, ws, `${owner.client.prefix}-1`)
    expect(first.state, JSON.stringify(first)).toBe('succeeded')
    m = await meta(owner, ws)
    expect(m.data.state).toBe('valid')
    expect(m.data.version).toMatchObject({ business_time: '2026-11-14T21:00:00Z', program: { name: 'quotes', version: '1' }, records: 2 })
    expect(m.data.attempt.status).toBe('succeeded')

    // the program failed: the refresh fails with its reason, the last good version stays
    writeOutput(host, { format: 'aiws.sync.v1', status: 'failed', error: 'the market API answered 503', program: { name: 'quotes', version: '1' }, produced_at: '2026-11-15T09:00:00Z' })
    const failed = await refresh(owner, ws, `${owner.client.prefix}-2`)
    expect(failed.state).toBe('failed')
    expect(JSON.stringify(failed)).toContain('503')
    m = await meta(owner, ws)
    expect(m.data.state).toBe('valid')
    expect(m.data.version.records).toBe(2)
    expect(m.data.attempt.status).toBe('failed')

    // the same data again: nothing new, no duplicate records
    const head = (await owner.client.ok<Json>('ws.get_info', { workspace_id: ws.workspace_id })).head_seq
    writeOutput(host, { format: 'aiws.sync.v1', status: 'ok', program: { name: 'quotes', version: '1' }, business_time: '2026-11-14T21:00:00Z', produced_at: '2026-11-15T10:00:00Z',
      records: [{ symbol: 'AAPL', close: 190.5 }, { symbol: 'MSFT', close: 410.25 }] })
    const again = await refresh(owner, ws, `${owner.client.prefix}-3`)
    expect(again.state, JSON.stringify(again)).toBe('succeeded')
    expect((await owner.client.ok<Json>('ws.get_info', { workspace_id: ws.workspace_id })).head_seq).toBe(head)
    const rows = await owner.client.ok<Json>('doc.query', { workspace_id: ws.workspace_id, sql: 'SELECT "Symbol", "Close" FROM "/data/Quotes" ORDER BY 1' })
    expect(rows.rows).toEqual([['AAPL', 190.5], ['MSFT', 410.25]])

    // a clone keeps the data and the definition; its last attempt is its own
    const clone = await owner.client.ok<WorkspaceSummary>('ws.clone', { workspace_id: ws.workspace_id })
    owner.client.owned.add(clone.workspace_id)
    const cm = await meta(owner, { workspace_id: clone.workspace_id, epoch: clone.epoch })
    expect(cm.data.state).toBe('valid')
    expect(cm.data.def.primary_key).toEqual(['symbol'])
    expect(cm.data.attempt ?? null).toBeNull()
  } finally {
    rmSync(hostDir, { recursive: true, force: true })
  }
})
