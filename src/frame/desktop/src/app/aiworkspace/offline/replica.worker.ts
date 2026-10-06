/* Replica Worker (design §6): one per held Workspace replica.
 *
 *   SQLite WASM (official build) on the `opfs-sahpool` VFS — no COOP/COEP, no SharedArrayBuffer;
 *   the WASM `Replica` (aiworkspace-core) is the engine, the database is its durable form.
 *
 * The replica database holds the confirmed-layer tables exactly as `replica.bootstrap` built them,
 * plus the client tables below. The working view is never stored: it is the confirmed layer with
 * the pending submissions replayed by the engine (first-phase document §11-12).
 *
 * Rules this file enforces:
 *   - a local edit is answered `saved_locally` only after the transaction that inserted its
 *     `pending_submissions` row has committed; if that fails the engine is rolled back too;
 *   - rows changed by `apply_remote`, `confirmed_seq` and the removal / state changes of pending
 *     rows are written in ONE transaction; if it fails the engine is rebuilt from the database,
 *     so memory never runs ahead of what is durable.
 * All operations run one at a time, in arrival order. */

import sqlite3InitModule, { type Database, type SAHPoolUtil, type Sqlite3Static, type SqlValue } from '@sqlite.org/sqlite-wasm'
import sqliteWasmUrl from '@sqlite.org/sqlite-wasm/sqlite3.wasm?url'
import initCore, { Replica } from '../wasm/aiworkspace_wasm.js'
import coreWasmUrl from '../wasm/aiworkspace_wasm_bg.wasm?url'
import type { AnnotationContent, CommitEvent, CommitRequest, EntityEnvelope, Json, ListAnnotationsParams, QueryPage, QueryParams, Selector, WorkspaceInfo } from '../api/types'
import {
  CLIENT_SCHEMA,
  type AssetRow, type InitResult, type LocalExport, type LocalRefused, type LocalSaved, type PendingMeta, type PendingRow, type PendingState,
  type ReadEntry, type ReplicaApi, type ReplicaMeta, type RichTextDraftRow, type WorkerFailure, type WorkerRequest, type WorkerResponse,
} from './protocol'

const DB_NAME = '/replica.sqlite'
const CONFIRMED_TABLES = ['entities', 'tree_edges', 'table_fields', 'table_records', 'richtext_states', 'refs', 'assets'] as const
const REF_KEY = ['src_entity_id', 'src_selector', 'kind', 'dst_workspace_id', 'dst_entity_id', 'dst_object_id', 'dst_query_json']

/* `pending_submissions` follows design §6.2 without `preimage_json` (the working view is rebuilt by
 * replay, there are no preimages) and with `meta_json` (what the UI shows for the row after a restart). */
const CLIENT_DDL = `
CREATE TABLE IF NOT EXISTS pending_submissions (
  local_order   INTEGER PRIMARY KEY AUTOINCREMENT,
  idem_key      TEXT NOT NULL UNIQUE,
  request_json  TEXT NOT NULL,
  state         TEXT NOT NULL,
  result_json   TEXT,
  meta_json     TEXT,
  created_at    TEXT NOT NULL
);
CREATE TABLE IF NOT EXISTS drafts (
  draft_id TEXT PRIMARY KEY, entity_id TEXT NOT NULL, kind TEXT NOT NULL,
  base_json TEXT NOT NULL, content BLOB NOT NULL, updated_at TEXT NOT NULL
);
CREATE TABLE IF NOT EXISTS replica_meta (key TEXT PRIMARY KEY, value TEXT NOT NULL);
`

class StorageError extends Error {
  readonly storageKind: 'quota' | 'transaction'
  constructor(storageKind: 'quota' | 'transaction', message: string) {
    super(message)
    this.name = 'StorageError'
    this.storageKind = storageKind
  }
}

class EngineError extends Error {
  readonly service: NonNullable<WorkerFailure['service']>
  constructor(service: NonNullable<WorkerFailure['service']>) {
    super(`${service.code}${service.detail ? `: ${service.detail}` : ''}`)
    this.name = 'EngineError'
    this.service = service
  }
}

/** The WASM facade throws `Error(JSON of the structured error)`. */
function engineError(error: unknown): Error {
  const message = error instanceof Error ? error.message : String(error)
  try {
    const parsed = JSON.parse(message) as { code?: string; detail?: string; sub_code?: string; data?: Record<string, unknown>; retryable?: boolean }
    if (parsed && typeof parsed.code === 'string') return new EngineError({ code: parsed.code, detail: parsed.detail, sub_code: parsed.sub_code, data: parsed.data, retryable: parsed.retryable })
  } catch { /* not a structured error */ }
  return error instanceof Error ? error : new Error(message)
}

function engine<T>(run: () => T): T {
  try { return run() } catch (error) { throw engineError(error) }
}

function storageError(error: unknown): StorageError {
  if (error instanceof StorageError) return error
  const message = error instanceof Error ? error.message : String(error)
  const quota = /quota|SQLITE_FULL|disk is full/i.test(message) || (error instanceof DOMException && error.name === 'QuotaExceededError')
  return new StorageError(quota ? 'quota' : 'transaction', message)
}

function toBase64(bytes: Uint8Array): string {
  let binary = ''
  for (let i = 0; i < bytes.length; i += 0x8000) binary += String.fromCharCode(...bytes.subarray(i, i + 0x8000))
  return btoa(binary)
}

function fromBase64(text: string): Uint8Array {
  const binary = atob(text)
  const out = new Uint8Array(binary.length)
  for (let i = 0; i < binary.length; i++) out[i] = binary.charCodeAt(i)
  return out
}

type Row = Record<string, SqlValue>

/** Worker-only OPFS API (not in the DOM lib this project compiles against). */
interface SyncAccessHandle {
  truncate(size: number): void
  write(data: Uint8Array, options?: { at: number }): number
  flush(): void
  close(): void
}

let workspaceId = ''
let testHooks = false
let sqlite3: Sqlite3Static | null = null
let pool: SAHPoolUtil | null = null
let db: Database | null = null
let replica: Replica | null = null
let meta: ReplicaMeta | null = null
/** Display data of pending rows (the engine does not carry it). */
const rowMeta = new Map<string, { meta: PendingMeta | null; created_at: string }>()
const failNext = { kind: 'error' as 'quota' | 'error', count: 0 }

function database(): Database {
  if (!db) throw new Error('the replica database is not open')
  return db
}

function core(): Replica {
  if (!replica) throw new Error('the replica is not open')
  return replica
}

function select(sql: string, bind?: SqlValue[]): Row[] {
  return database().exec({ sql, bind, rowMode: 'object', returnValue: 'resultRows' }) as Row[]
}

function run(sql: string, bind?: SqlValue[]) {
  database().exec({ sql, bind })
}

/** BEGIN … COMMIT. Resolves only after COMMIT returned; any failure rolls back and surfaces as StorageError. */
function transaction(work: () => void) {
  try {
    database().transaction(() => {
      work()
      if (failNext.count > 0) {
        failNext.count -= 1
        throw new StorageError(failNext.kind === 'quota' ? 'quota' : 'transaction', failNext.kind === 'quota' ? 'injected: QuotaExceededError (SQLITE_FULL)' : 'injected: transaction failed')
      }
    })
  } catch (error) {
    throw storageError(error)
  }
}

function metaValue(key: string): string | null {
  const rows = select('SELECT value FROM replica_meta WHERE key = ?', [key])
  return rows.length > 0 ? String(rows[0].value) : null
}

function setMetaValue(key: string, value: string) {
  run('INSERT INTO replica_meta(key, value) VALUES (?, ?) ON CONFLICT(key) DO UPDATE SET value = excluded.value', [key, value])
}

function readMeta(): ReplicaMeta | null {
  const tables = select("SELECT name FROM sqlite_master WHERE type = 'table' AND name = 'replica_meta'")
  if (tables.length === 0) return null
  const epoch = metaValue('epoch')
  const principal = metaValue('principal')
  const info = metaValue('info')
  if (!epoch || !principal || !info) return null
  return {
    workspace_id: workspaceId,
    principal,
    epoch,
    confirmed_seq: Number(metaValue('confirmed_seq') ?? '0'),
    prepared_at: metaValue('prepared_at') ?? '',
    info: JSON.parse(info) as WorkspaceInfo,
    client_schema: Number(metaValue('client_schema') ?? '0'),
  }
}

function confirmedTables(): Record<string, Row[]> {
  const out: Record<string, Row[]> = {}
  for (const table of CONFIRMED_TABLES) {
    out[table] = select(`SELECT * FROM ${table}`).map((row) => {
      for (const [column, value] of Object.entries(row)) {
        if (value instanceof Uint8Array) row[column] = toBase64(value)
        else if (typeof value === 'bigint') row[column] = Number(value)
      }
      return row
    })
  }
  return out
}

function storedPending(): { idempotency_key: string; request: CommitRequest; state: PendingState; result: Json | null }[] {
  rowMeta.clear()
  return select('SELECT idem_key, request_json, state, result_json, meta_json, created_at FROM pending_submissions ORDER BY local_order').map((row) => {
    const key = String(row.idem_key)
    rowMeta.set(key, { meta: row.meta_json ? JSON.parse(String(row.meta_json)) as PendingMeta : null, created_at: String(row.created_at) })
    return {
      idempotency_key: key,
      request: JSON.parse(String(row.request_json)) as CommitRequest,
      state: String(row.state) as PendingState,
      result: row.result_json ? JSON.parse(String(row.result_json)) as Json : null,
    }
  })
}

/** Database rows → engine. The only way the engine comes into existence. */
function loadEngine() {
  const current = readMeta()
  if (!current) throw new Error('this Workspace has no offline replica on this device')
  if (current.client_schema !== CLIENT_SCHEMA) throw new Error(`the offline replica was written by another version (client schema ${current.client_schema}); prepare it again`)
  replica?.free()
  replica = null
  // a fresh peer id per load: it only labels CRDT operations the planner creates locally
  const peer = Math.floor(Math.random() * 2 ** 48) + 1
  const next = engine(() => new Replica(current.principal, workspaceId, current.epoch, JSON.stringify(confirmedTables()), current.confirmed_seq, peer))
  engine(() => next.restore_pending(JSON.stringify(storedPending())))
  replica = next
  meta = current
}

function pendingRows(): PendingRow[] {
  const rows = JSON.parse(core().pending()) as { idempotency_key: string; request: CommitRequest; state: PendingState; result: Json | null; applied: boolean }[]
  return rows.map((row) => ({ ...row, result: row.result ?? null, meta: rowMeta.get(row.idempotency_key)?.meta ?? null, created_at: rowMeta.get(row.idempotency_key)?.created_at ?? '' }))
}

/** Make the pending table equal to the engine's list (inside the caller's transaction). */
function writePendingStates(rows: PendingRow[]) {
  const keys = new Set(rows.map((row) => row.idempotency_key))
  for (const stored of select('SELECT idem_key FROM pending_submissions')) {
    if (!keys.has(String(stored.idem_key))) run('DELETE FROM pending_submissions WHERE idem_key = ?', [String(stored.idem_key)])
  }
  for (const row of rows) {
    run('UPDATE pending_submissions SET state = ?, result_json = ? WHERE idem_key = ?', [row.state, row.result === null ? null : JSON.stringify(row.result), row.idempotency_key])
  }
}

function forgetRemovedMeta(rows: PendingRow[]) {
  const keys = new Set(rows.map((row) => row.idempotency_key))
  for (const key of [...rowMeta.keys()]) if (!keys.has(key)) rowMeta.delete(key)
}

function upsert(table: string, row: Record<string, Json>) {
  const columns = Object.keys(row)
  const values = columns.map((column): SqlValue => {
    const value = row[column]
    if (table === 'richtext_states' && column === 'snapshot' && typeof value === 'string') return fromBase64(value)
    if (value === null || typeof value === 'string' || typeof value === 'number') return value
    if (typeof value === 'boolean') return value ? 1 : 0
    return JSON.stringify(value)
  })
  run(`INSERT OR REPLACE INTO ${table}(${columns.join(', ')}) VALUES (${columns.map(() => '?').join(', ')})`, values)
}

async function assetDirectory(create: boolean): Promise<FileSystemDirectoryHandle | null> {
  try {
    const root = await navigator.storage.getDirectory()
    const base = await root.getDirectoryHandle('.aiworkspace', { create })
    return await base.getDirectoryHandle(`assets-${workspaceId}`, { create })
  } catch (error) {
    if (!create) return null
    throw error
  }
}

const assetFileName = (objectId: string) => objectId.replace(/[^A-Za-z0-9._-]/g, '_')

async function closeDatabase() {
  replica?.free()
  replica = null
  meta = null
  rowMeta.clear()
  try { db?.close() } catch { /* already closed */ }
  db = null
}

function openDatabase() {
  if (!pool) throw new Error('storage is not initialised')
  db = new pool.OpfsSAHPoolDb(DB_NAME)
  run('PRAGMA foreign_keys = OFF')
}

type Handlers = { [K in keyof ReplicaApi]: (...args: Parameters<ReplicaApi[K]>) => ReturnType<ReplicaApi[K]> | Promise<ReturnType<ReplicaApi[K]>> }

const handlers: Handlers = {
  async init(options): Promise<InitResult> {
    workspaceId = options.workspaceId
    testHooks = options.testHooks
    if (!/^[A-Za-z0-9][A-Za-z0-9_-]{0,63}$/.test(workspaceId)) return { ok: false, reason: `非法的工作区标识：${workspaceId}` }
    if (typeof navigator.storage?.getDirectory !== 'function') return { ok: false, reason: '此浏览器没有 OPFS（Origin Private File System）' }
    if (typeof FileSystemFileHandle === 'undefined' || !('createSyncAccessHandle' in FileSystemFileHandle.prototype)) {
      return { ok: false, reason: '此浏览器的 Worker 不支持 OPFS 同步访问句柄（createSyncAccessHandle）' }
    }
    try {
      await initCore({ module_or_path: coreWasmUrl })
      const start = sqlite3InitModule as unknown as (options: { locateFile: () => string; print: (text: string) => void; printErr: (text: string) => void }) => Promise<Sqlite3Static>
      sqlite3 = await start({ locateFile: () => sqliteWasmUrl, print: () => undefined, printErr: (text) => console.warn('[aiworkspace replica]', text) })
      // The access handles of a Worker that was just terminated are released a moment later: retry briefly.
      for (let attempt = 0; ; attempt++) {
        try {
          // `forceReinitIfPreviouslyFailed` exists in the library but not in its typings
          const options = { name: `aiws-${workspaceId}`, directory: `/.aiworkspace/replica-${workspaceId}`, initialCapacity: 6, clearOnInit: false, forceReinitIfPreviouslyFailed: true }
          pool = await sqlite3.installOpfsSAHPoolVfs(options)
          break
        } catch (error) {
          if (attempt >= 8) throw error
          await new Promise((resolve) => setTimeout(resolve, 250))
        }
      }
    } catch (error) {
      return { ok: false, reason: `无法初始化本地数据库（SQLite WASM / opfs-sahpool）：${error instanceof Error ? error.message : String(error)}` }
    }
    let prepared: ReplicaMeta | null = null
    let pendingCount = 0
    if (pool.getFileNames().includes(DB_NAME)) {
      try {
        openDatabase()
        prepared = readMeta()
        if (prepared) pendingCount = Number(select('SELECT COUNT(*) AS n FROM pending_submissions')[0].n)
      } catch (error) {
        await closeDatabase()
        return { ok: false, reason: `本机的离线副本无法打开：${error instanceof Error ? error.message : String(error)}` }
      }
    }
    return { ok: true, prepared, pendingCount, sqlite: sqlite3.version.libVersion, vfs: 'opfs-sahpool' }
  },

  async importReplica(args): Promise<ReplicaMeta> {
    if (!pool) throw new Error('storage is not initialised')
    if (db) {
      const count = readMeta() ? Number(select('SELECT COUNT(*) AS n FROM pending_submissions')[0].n) : 0
      if (count > 0 && !args.discardLocal) throw new EngineError({ code: 'LOCAL_PENDING', detail: `本机副本里还有 ${count} 条未提交的修改；请先导出或处理它们` })
    }
    await closeDatabase()
    try {
      await pool.importDb(DB_NAME, new Uint8Array(args.bytes))
      openDatabase()
      transaction(() => {
        run('DROP TABLE IF EXISTS pending_submissions')
        run('DROP TABLE IF EXISTS drafts')
        run('DROP TABLE IF EXISTS replica_meta')
        database().exec(CLIENT_DDL)
        setMetaValue('epoch', args.epoch)
        setMetaValue('confirmed_seq', String(args.headSeq))
        setMetaValue('principal', args.principal)
        setMetaValue('prepared_at', args.preparedAt)
        setMetaValue('info', JSON.stringify(args.info))
        setMetaValue('client_schema', String(CLIENT_SCHEMA))
      })
    } catch (error) {
      throw storageError(error)
    }
    const written = readMeta()
    if (!written) throw new StorageError('transaction', 'the replica metadata could not be read back')
    return written
  },

  open() {
    if (!db) throw new Error('this Workspace has no offline replica on this device')
    loadEngine()
    return { meta: meta as ReplicaMeta, pending: pendingRows() }
  },

  outline() {
    return (JSON.parse(engine(() => core().outline())) as { entities: EntityEnvelope[] }).entities
  },

  read(entityId: string, selector: Selector | null) {
    const result = JSON.parse(engine(() => core().read(entityId, selector ? JSON.stringify(selector) : undefined))) as EntityEnvelope & { content: unknown; head_seq: number }
    result.head_seq = core().confirmed_seq
    return result
  },

  readMany(targets): ReadEntry[] {
    return targets.map((target) => {
      try {
        return handlers.read(target.entity_id, target.selector ?? null) as ReadEntry
      } catch (error) {
        if (error instanceof EngineError) return { error: { code: error.service.code, detail: error.service.detail } }
        throw error
      }
    })
  },

  listAnnotations(params: ListAnnotationsParams) {
    const { annotations } = JSON.parse(engine(() => core().list_annotations(JSON.stringify(params)))) as { annotations: (EntityEnvelope & { content: AnnotationContent })[] }
    return annotations.map((entry) => ({ ...entry, head_seq: core().confirmed_seq }))
  },

  query(params: QueryParams) {
    return JSON.parse(engine(() => core().query(JSON.stringify(params)))) as QueryPage
  },

  collab(entityId: string) {
    const lineage = core().lineage_id(entityId)
    if (!lineage) throw new EngineError({ code: 'NOT_FOUND', detail: `${entityId} is not a rich text of this replica` })
    const updates = JSON.parse(core().pending_richtext_updates(entityId)) as { update: string }[]
    return { lineage_id: lineage, snapshot: toBase64(engine(() => core().collab_snapshot(entityId))), pending_updates: updates.map((item) => item.update) }
  },

  submit(request: CommitRequest, pendingMeta: PendingMeta | null): LocalSaved | LocalRefused {
    const result = JSON.parse(engine(() => core().submit_local(JSON.stringify(request)))) as { status: string; provisional_seq?: number }
    if (result.status !== 'saved_locally') return result as LocalRefused
    const key = request.idempotency_key
    const createdAt = new Date().toISOString()
    // the engine may have rewritten provisional `expect.rev` into `{ after: key }`: store what it keeps
    const stored = (JSON.parse(core().pending()) as { idempotency_key: string; request: CommitRequest }[]).find((row) => row.idempotency_key === key)
    try {
      transaction(() => {
        run('INSERT INTO pending_submissions(idem_key, request_json, state, result_json, meta_json, created_at) VALUES (?, ?, ?, NULL, ?, ?)',
          [key, JSON.stringify(stored?.request ?? request), 'queued', pendingMeta ? JSON.stringify(pendingMeta) : null, createdAt])
      })
    } catch (error) {
      // not durable → not saved: take it out of the working view again
      core().discard(key)
      throw error
    }
    rowMeta.set(key, { meta: pendingMeta, created_at: createdAt })
    return { status: 'saved_locally', idempotency_key: key, provisional_seq: result.provisional_seq ?? 0, pending: pendingRows() }
  },

  nextToSend() {
    return JSON.parse(engine(() => core().next_to_send())) as CommitRequest | null
  },

  mark(key: string, state: PendingState, result: Json | null) {
    engine(() => core().mark(key, state, result === null ? undefined : JSON.stringify(result)))
    const rows = pendingRows()
    try {
      transaction(() => writePendingStates(rows))
    } catch (error) {
      loadEngine()
      throw error
    }
    return rows
  },

  discard(key: string, unsentOnly: boolean) {
    const before = pendingRows().find((row) => row.idempotency_key === key) ?? null
    if (!before) return { removed: null, pending: pendingRows() }
    if (unsentOnly && (before.state === 'sending' || before.state === 'unknown')) {
      throw new EngineError({ code: 'IN_FLIGHT', detail: '这条修改正在发送或结果未知，后台可能已经接受它，暂时不能撤回' })
    }
    core().discard(key)
    const rows = pendingRows()
    try {
      transaction(() => writePendingStates(rows))
    } catch (error) {
      loadEngine()
      throw error
    }
    forgetRemovedMeta(rows)
    return { removed: before, pending: rows }
  },

  applyRemote(events: CommitEvent[]) {
    let applied: { confirmed_seq: number; settled: string[] }
    try {
      applied = JSON.parse(engine(() => core().apply_remote(JSON.stringify(events)))) as { confirmed_seq: number; settled: string[] }
    } catch (error) {
      // a batch may have been applied in part: memory must not run ahead of the database
      loadEngine()
      throw error
    }
    const rows = pendingRows()
    try {
      const delta = JSON.parse(engine(() => core().take_confirmed_delta())) as Record<string, Record<string, Json>[]>
      transaction(() => {
        for (const gone of delta.refs_deleted ?? []) {
          run(`DELETE FROM refs WHERE ${REF_KEY.map((column) => `${column} = ?`).join(' AND ')}`, REF_KEY.map((column) => String(gone[column] ?? '')))
        }
        for (const table of CONFIRMED_TABLES) for (const row of delta[table] ?? []) upsert(table, row)
        setMetaValue('confirmed_seq', String(applied.confirmed_seq))
        writePendingStates(rows)
      })
    } catch (error) {
      loadEngine()
      throw error
    }
    forgetRemovedMeta(rows)
    if (meta) meta = { ...meta, confirmed_seq: applied.confirmed_seq }
    return { confirmed_seq: applied.confirmed_seq, settled: applied.settled, pending: rows }
  },

  pending() {
    return pendingRows()
  },

  updateInfo(info: WorkspaceInfo) {
    transaction(() => setMetaValue('info', JSON.stringify(info)))
    if (meta) meta = { ...meta, info }
    return null
  },

  saveDraft(draft: RichTextDraftRow) {
    transaction(() => {
      run('INSERT OR REPLACE INTO drafts(draft_id, entity_id, kind, base_json, content, updated_at) VALUES (?, ?, ?, ?, ?, ?)',
        [draft.draft_id, draft.entity_id, draft.kind, JSON.stringify(draft.base), new TextEncoder().encode(JSON.stringify(draft.content)), draft.updated_at])
    })
    return null
  },

  exportLocal(): LocalExport {
    const current = meta ?? readMeta()
    if (!current) throw new Error('this Workspace has no offline replica on this device')
    // read from the database, not from memory: this is what would survive a crash
    const pending = select('SELECT idem_key, request_json, state, result_json, meta_json, created_at FROM pending_submissions ORDER BY local_order').map((row): PendingRow => ({
      idempotency_key: String(row.idem_key),
      request: JSON.parse(String(row.request_json)) as CommitRequest,
      state: String(row.state) as PendingState,
      result: row.result_json ? JSON.parse(String(row.result_json)) as Json : null,
      applied: false,
      meta: row.meta_json ? JSON.parse(String(row.meta_json)) as PendingMeta : null,
      created_at: String(row.created_at),
    }))
    const drafts = select('SELECT draft_id, entity_id, kind, base_json, content, updated_at FROM drafts ORDER BY updated_at').map((row): RichTextDraftRow => ({
      draft_id: String(row.draft_id), entity_id: String(row.entity_id), kind: String(row.kind), base: JSON.parse(String(row.base_json)) as Json,
      content: JSON.parse(new TextDecoder().decode(row.content as Uint8Array)) as Json, updated_at: String(row.updated_at),
    }))
    return { format: 'buckyos.aiworkspace.local-pending/1', exported_at: new Date().toISOString(), meta: current, pending, drafts }
  },

  listAssets(): AssetRow[] {
    return select('SELECT object_id, media_type, size FROM assets').map((row) => ({ object_id: String(row.object_id), media_type: String(row.media_type), size: Number(row.size) }))
  },

  async putAsset(objectId: string, bytes: ArrayBuffer) {
    try {
      const directory = await assetDirectory(true)
      const file = await (directory as FileSystemDirectoryHandle).getFileHandle(assetFileName(objectId), { create: true })
      const handle = await (file as FileSystemFileHandle & { createSyncAccessHandle(): Promise<SyncAccessHandle> }).createSyncAccessHandle()
      try {
        handle.truncate(0)
        handle.write(new Uint8Array(bytes), { at: 0 })
        handle.flush()
      } finally { handle.close() }
    } catch (error) {
      throw storageError(error)
    }
    return null
  },

  async getAsset(objectId: string) {
    const directory = await assetDirectory(false)
    if (!directory) return null
    try {
      const file = await directory.getFileHandle(assetFileName(objectId))
      return await (await file.getFile()).arrayBuffer()
    } catch {
      return null
    }
  },

  async destroy() {
    await closeDatabase()
    if (pool) {
      await pool.wipeFiles()
      await pool.removeVfs()
      pool = null
    }
    try {
      const root = await navigator.storage.getDirectory()
      const base = await root.getDirectoryHandle('.aiworkspace')
      await base.removeEntry(`assets-${workspaceId}`, { recursive: true }).catch(() => undefined)
      await base.removeEntry(`replica-${workspaceId}`, { recursive: true }).catch(() => undefined)
    } catch { /* nothing was stored */ }
    return null
  },

  async close() {
    await closeDatabase()
    // release the pool's access handles so another tab (or a restarted Worker) can take over at once
    try { pool?.pauseVfs() } catch { /* the Worker is about to end anyway */ }
    return null
  },

  testFailTransactions(kind, count) {
    if (!testHooks) throw new Error('test hooks are not enabled')
    failNext.kind = kind
    failNext.count = count
    return null
  },
}

function failure(error: unknown): WorkerFailure {
  if (error instanceof StorageError) return { kind: 'storage', message: error.message, storageKind: error.storageKind }
  if (error instanceof EngineError) return { kind: 'service', message: error.message, service: error.service }
  return { kind: 'internal', message: error instanceof Error ? error.message : String(error) }
}

let chain: Promise<void> = Promise.resolve()

self.onmessage = (event: MessageEvent<WorkerRequest>) => {
  const { id, op, args } = event.data
  chain = chain.then(async () => {
    let response: WorkerResponse
    try {
      const handler = handlers[op] as (...values: unknown[]) => unknown
      response = { id, ok: true, value: await handler(...args) }
    } catch (error) {
      response = { id, ok: false, error: failure(error) }
    }
    self.postMessage(response)
  })
}
