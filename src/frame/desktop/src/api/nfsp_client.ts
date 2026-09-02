/**
 * NFSP v0 client for nfs_server (buckyos/src/frame/nfs_server).
 *
 * Speaks the control plane (`POST /nfs/v1/{method}`), the data plane
 * (`GET /nfs/v1/read/{node_id}`), the minimal tus upload area
 * (`PATCH|HEAD /nfs/v1/uploads/{fb}`) and the watch SSE stream
 * (`GET /nfs/v1/watch`).
 *
 * Protocol references:
 * - cyfs-ndn/doc/NamedFileSystem_Protocol_v0.md (NFSP v0)
 * - buckyos/product/bucky_file/nfs_server.md
 * - buckyos/src/frame/nfs_server/README.md (v1 降级契约)
 *
 * Client contract highlights (nfs_server README §4.2):
 * - `revision` is an opaque equality token: only compare, never order.
 * - Write ops require `seq` (exactly-once); the client auto-assigns one per
 *   write and replays with the SAME seq on network retry.
 * - Leases are advisory versus server-local bypass writers: commit may fail
 *   with TARGET_MISMATCH{reason:"bypass_modified"} — surface, don't retry.
 * - watch is lossy: any `resync` event means "re-list what you care about".
 *
 * Uses only fetch/WHATWG streams, so it runs in the browser and in Node ≥ 18.
 */

// ---------------------------------------------------------------------------
// Wire types
// ---------------------------------------------------------------------------

export const NFSP_VERSION = 'nfsp/0'

/** `{"type":"live",...}` or `{"type":"object",...}` (NFSP §3.1.2). */
export type LiveRef = { type: 'live'; node_id: string; gen?: number }
export type ObjectRef = { type: 'object'; obj_id: string; inner_path?: string }
export type WireRef = LiveRef | ObjectRef

export const liveRef = (nodeId: string, gen = 0): LiveRef => ({
  type: 'live',
  node_id: nodeId,
  gen,
})

/** The `at` locator: ref > uri > path (NFSP §3.4). */
export interface Locator {
  realm?: string
  path?: string
  uri?: string
  ref?: WireRef
}

/** Sugar: accept a dfs path string, a `xxx://` uri, a WireRef or a Locator. */
export type LocatorLike = string | WireRef | Locator

export const toLocator = (at: LocatorLike): Locator => {
  if (typeof at === 'string') {
    if (at.includes('://')) return { uri: at }
    return { realm: 'dfs', path: at }
  }
  if ('type' in at) return { ref: at }
  return at
}

export type WantGroup = 'base' | 'ident' | 'access' | 'meta'

export type NodeKind = 'dir' | 'file' | 'symlink' | 'view' | 'collection' | 'group'

export interface Capabilities {
  list: boolean
  read: boolean
  accepts_content: boolean
  accepts_references: boolean
  remove_semantics: 'destroy' | 'unlink' | 'none'
  ordered: boolean
}

/** resolve/stat result (fields beyond `base` appear per the `want` mask). */
export interface NodeInfo {
  kind: NodeKind
  state: string
  ref: WireRef
  capabilities: Capabilities
  revision?: string
  locations?: unknown[]
  // base
  name?: string
  size?: number
  mtime?: number
  ctime?: number
  flags?: string[]
  // ident
  node_id?: string
  gen?: number
  etag?: string
  obj_id?: string
  // access
  access_urls?: { kind: string; url: string }[]
  // meta
  meta_summary?: Record<string, number>
  // view / collection extras
  view_id?: string
  collection_id?: string
  title?: string
  origin?: string
  stale?: boolean
}

export type EntryBinding = 'native' | 'reference' | 'member' | 'derived'

/** Compact target of a listing entry (not a full NodeInfo). */
export interface EntryTarget {
  ref: WireRef
  kind: NodeKind
  /** Attribute groups per the `want` mask: base → size/mtime/flags, ident → etag/obj_id, access → access_urls. */
  attrs?: {
    size?: number
    mtime?: number
    flags?: string[]
    etag?: string
    obj_id?: string
    access_urls?: { kind: string; url: string }[]
  } & Record<string, unknown>
  /** `stale` when a referenced target no longer resolves. */
  target_state?: string
}

export interface Entry {
  name: string
  binding: EntryBinding
  entry_ref?: string
  target: EntryTarget
  canonical_path?: string
  context?: { count?: number; provenance?: Record<string, unknown> } & Record<string, unknown>
}

export interface Listing {
  container: NodeInfo
  entries: Entry[]
  truncated?: boolean
  next_cursor?: string
  revision_changed?: boolean
  watch_token?: string
  /** Same-name virtual bindings shadowed by native entries (`native_shadow`). */
  conflicts?: { name: string; reason: string; entry_ref?: string; target?: EntryTarget }[]
}

export interface ListOptions {
  cursor?: string
  limit?: number
  order?: 'name' | 'mtime' | 'size' | 'manual'
  filter?: { kind?: NodeKind[]; name_glob?: string }
}

export interface HelloResult {
  version: string
  session: string
  features: string[]
  limits: { max_batch: number; max_list: number; replay_window: number; attr_ttl_ms: number }
  realms: { id: string; writable: boolean }[]
}

export type BatchOp =
  | { m: 'walk'; args: { name?: string; names?: string[]; entry_ref?: string } }
  | { m: 'stat' | 'resolve'; args?: { name?: string }; want?: WantGroup[] }
  | { m: 'list'; args?: ListOptions; want?: WantGroup[] }

export interface BatchResult {
  completed: number
  results: ({ ok: true; result: unknown } | { ok: false; error: NfspErrorBody })[]
}

export interface OpenWriteResult {
  fb_handle: string
  upload_url: string
  lease: { lease_id: string; seq: number; ttl_ms: number }
  target: { path: string; exists: boolean }
}

export interface CommitResult {
  ref: WireRef
  entry_ref: string
  revision: string
  obj: { sha256: string; size: number }
}

export interface MetaRecord {
  ns: string
  key: string
  value: unknown
  source?: unknown
  confidence?: number
  anchor?: string
  visibility?: string
}

export interface SearchHit {
  match_source: string
  canonical_path: string
  explain?: Record<string, unknown>
  [k: string]: unknown
}

export interface SearchResult {
  hits: SearchHit[]
  partial: boolean
  sources: { mode: string; state: string; took_ms?: number; reason?: string }[]
  next_cursor?: string
}

export interface GrantResult {
  cap_id: string
  token: string
  subtree: string
  ops: string[]
  expires_at?: number
}

export type CollectionPatchOp =
  | {
      add_ref: {
        target_ref: WireRef
        name?: string
        position?: number
        parent_entry_ref?: string
      }
    }
  | { remove_entry: { entry_ref: string } }
  | { move_entries: { entry_refs: string[]; to_index: number } }
  | { create_group: { name: string; position?: number } }
  | { rename_group: { entry_ref: string; name: string } }

export interface WatchEvent {
  /** `resync` | `container_changed` | `meta_changed` (extensible). */
  event: string
  /** Parsed JSON payload of the SSE `data:` line. */
  data: Record<string, unknown>
  /** SSE `id:` (the server_rev), when present. */
  id?: string
}

// ---------------------------------------------------------------------------
// Errors
// ---------------------------------------------------------------------------

/** Structured NFSP error body (§8 plus implementation extensions). */
export interface NfspErrorBody {
  code: string
  message: string
  [k: string]: unknown
}

export class NfspError extends Error {
  readonly code: string
  readonly httpStatus: number
  /** Extra structured fields (reason, holder_session, obj_id, expected, ...). */
  readonly details: Record<string, unknown>

  constructor(body: NfspErrorBody, httpStatus: number) {
    super(`${body.code}: ${body.message}`)
    this.name = 'NfspError'
    this.code = body.code
    this.httpStatus = httpStatus
    const { code: _c, message: _m, ...rest } = body
    this.details = rest
  }
}

// ---------------------------------------------------------------------------
// Client
// ---------------------------------------------------------------------------

export interface NfspClientOptions {
  /** e.g. `http://127.0.0.1:3260` — no trailing slash needed. */
  baseUrl: string
  /** Override fetch (tests, custom auth wrappers). Defaults to global fetch. */
  fetch?: typeof fetch
  /** tus PATCH chunk size; server guidance is 4–16 MB. Default 8 MB. */
  uploadChunkSize?: number
}

interface EnvelopeExtras {
  at?: Locator
  want?: WantGroup[]
  args?: Record<string, unknown>
}

export class NfspClient {
  readonly baseUrl: string
  private readonly fetchFn: typeof fetch
  private readonly chunkSize: number
  private session: string | null = null
  private seq = 0
  private helloResult: HelloResult | null = null

  constructor(opts: NfspClientOptions) {
    this.baseUrl = opts.baseUrl.replace(/\/+$/, '')
    // Bind to globalThis: an unbound fetch throws "Illegal invocation".
    this.fetchFn = opts.fetch ?? ((...a) => globalThis.fetch(...a))
    this.chunkSize = opts.uploadChunkSize ?? 8 * 1024 * 1024
  }

  get sessionId(): string | null {
    return this.session
  }

  /** Features advertised by the server in hello (empty before hello). */
  get features(): string[] {
    return this.helloResult?.features ?? []
  }

  get limits(): HelloResult['limits'] | null {
    return this.helloResult?.limits ?? null
  }

  // ---------- session ----------

  async hello(clientFeatures?: string[]): Promise<HelloResult> {
    const args: Record<string, unknown> = { versions: [NFSP_VERSION] }
    if (clientFeatures) args.features = clientFeatures
    const r = (await this.post('hello', { args })) as HelloResult
    this.session = r.session
    this.seq = 0
    this.helloResult = r
    return r
  }

  async bye(): Promise<void> {
    await this.call('bye', {})
    this.session = null
    this.helloResult = null
  }

  // ---------- resolve / stat / list / batch ----------

  async resolve(at: LocatorLike, want?: WantGroup[]): Promise<NodeInfo> {
    return (await this.call('resolve', { at: toLocator(at), want })) as NodeInfo
  }

  /** stat with an optional child `name` step (server-side walk). */
  async stat(at: LocatorLike, opts?: { name?: string; want?: WantGroup[] }): Promise<NodeInfo> {
    return (await this.call('stat', {
      at: toLocator(at),
      want: opts?.want,
      args: opts?.name ? { name: opts.name } : {},
    })) as NodeInfo
  }

  async list(at: LocatorLike, opts?: ListOptions, want?: WantGroup[]): Promise<Listing> {
    return (await this.call('list', {
      at: toLocator(at),
      want,
      args: (opts ?? {}) as Record<string, unknown>,
    })) as Listing
  }

  async batch(
    start: LocatorLike,
    ops: BatchOp[],
    onError: 'abort' | 'continue' = 'abort',
  ): Promise<BatchResult> {
    return (await this.call('batch', {
      args: { start: toLocator(start), ops, on_error: onError },
    })) as BatchResult
  }

  // ---------- structure writes ----------

  /**
   * Ref form: create one child under `parent` (optional CAS via
   * `expectedRevision`). Path form: pass a dfs path string and no name —
   * behaves like `mkdir -p` (idempotent, no CAS).
   */
  async mkdir(
    parent: LocatorLike,
    name?: string,
    opts?: { expectedRevision?: string },
  ): Promise<{ ref: WireRef; existed: boolean; revision?: string }> {
    const args: Record<string, unknown> = {}
    if (name !== undefined) args.name = name
    if (opts?.expectedRevision) args.expected_revision = opts.expectedRevision
    return (await this.write('mkdir', { at: toLocator(parent), args })) as {
      ref: WireRef
      existed: boolean
      revision?: string
    }
  }

  async move(
    from: { parentRef: WireRef; name: string },
    to: { parentRef: WireRef; name: string },
    opts?: { expectedFromRevision?: string; expectedToRevision?: string },
  ): Promise<{ from_revision: string; to_revision: string }> {
    return (await this.write('move', {
      args: {
        from: { parent_ref: from.parentRef, name: from.name },
        to: { parent_ref: to.parentRef, name: to.name },
        expected_from_revision: opts?.expectedFromRevision,
        expected_to_revision: opts?.expectedToRevision,
      },
    })) as { from_revision: string; to_revision: string }
  }

  /** Destroys a native entry. Reference entries must use `unlink` instead. */
  async delete(
    parent: LocatorLike,
    name: string,
    opts?: { recursive?: boolean; expectedRevision?: string },
  ): Promise<{ revision: string }> {
    return (await this.write('delete', {
      at: toLocator(parent),
      args: {
        name,
        recursive: opts?.recursive,
        expected_revision: opts?.expectedRevision,
      },
    })) as { revision: string }
  }

  async bindRef(
    parentRef: WireRef,
    name: string,
    targetRef: WireRef,
    opts?: { expectedRevision?: string },
  ): Promise<{ entry_ref: string; revision: string }> {
    return (await this.write('bind_ref', {
      args: {
        parent_ref: parentRef,
        name,
        target_ref: targetRef,
        expected_revision: opts?.expectedRevision,
      },
    })) as { entry_ref: string; revision: string }
  }

  /** Removes a reference entry (`be_*`) only; the target is never touched. */
  async unlink(
    entryRef: string,
    opts?: { expectedRevision?: string },
  ): Promise<{ revision: string }> {
    return (await this.write('unlink', {
      args: { entry_ref: entryRef, expected_revision: opts?.expectedRevision },
    })) as { revision: string }
  }

  // ---------- content writes (open_write / tus / commit_file / probe) ----------

  async openWrite(
    target: { parentRef: WireRef; name: string; size?: number } | { ref: WireRef },
  ): Promise<OpenWriteResult> {
    const args: Record<string, unknown> =
      'ref' in target
        ? { ref: target.ref }
        : { parent_ref: target.parentRef, name: target.name, size: target.size }
    return (await this.write('open_write', { args })) as OpenWriteResult
  }

  /** Current upload offset (tus HEAD) — resume point after interruption. */
  async uploadOffset(fbHandle: string): Promise<number> {
    const resp = await this.fetchFn(`${this.baseUrl}/nfs/v1/uploads/${fbHandle}`, {
      method: 'HEAD',
    })
    if (!resp.ok) throw await this.httpError(resp)
    return Number(resp.headers.get('Upload-Offset') ?? '0')
  }

  /** One tus PATCH. Returns the new offset. */
  async uploadChunk(fbHandle: string, offset: number, chunk: Uint8Array): Promise<number> {
    const resp = await this.fetchFn(`${this.baseUrl}/nfs/v1/uploads/${fbHandle}`, {
      method: 'PATCH',
      headers: {
        'Upload-Offset': String(offset),
        'Content-Type': 'application/offset+octet-stream',
      },
      body: chunk as unknown as BodyInit,
    })
    if (resp.status !== 204) throw await this.httpError(resp)
    return Number(resp.headers.get('Upload-Offset') ?? String(offset + chunk.byteLength))
  }

  /** Uploads a whole buffer in chunks, resuming from the server's offset. */
  async uploadContent(
    fbHandle: string,
    content: Uint8Array,
    onProgress?: (sent: number, total: number) => void,
  ): Promise<void> {
    let offset = await this.uploadOffset(fbHandle)
    while (offset < content.byteLength) {
      const chunk = content.subarray(offset, Math.min(offset + this.chunkSize, content.byteLength))
      offset = await this.uploadChunk(fbHandle, offset, chunk)
      onProgress?.(offset, content.byteLength)
    }
  }

  async commitFile(
    parent: LocatorLike,
    name: string,
    source: { fbHandle: string; leaseId?: string } | { hash: string },
    opts?: { overwrite?: boolean; expectedRevision?: string },
  ): Promise<CommitResult> {
    const args: Record<string, unknown> = {
      name,
      overwrite: opts?.overwrite,
      expected_revision: opts?.expectedRevision,
    }
    if ('fbHandle' in source) {
      args.fb_handle = source.fbHandle
      if (source.leaseId) args.lease_id = source.leaseId
    } else {
      args.hash = source.hash
    }
    return (await this.write('commit_file', { at: toLocator(parent), args })) as CommitResult
  }

  /** Which of these digests must be uploaded (§ probe / 秒传). */
  async probe(digests: { hash: string; size?: number }[]): Promise<{ missing: { hash: string; size?: number }[] }> {
    return (await this.call('probe', { args: { digests } })) as {
      missing: { hash: string; size?: number }[]
    }
  }

  /**
   * Convenience: open_write + chunked tus upload + commit_file.
   * Overwrites an existing file only when `opts.overwrite` is set.
   */
  async uploadFile(
    parentRef: WireRef,
    name: string,
    content: Uint8Array,
    opts?: { overwrite?: boolean; onProgress?: (sent: number, total: number) => void },
  ): Promise<CommitResult> {
    const ow = await this.openWrite({ parentRef, name, size: content.byteLength })
    await this.uploadContent(ow.fb_handle, content, opts?.onProgress)
    return this.commitFile({ ref: parentRef }, name, {
      fbHandle: ow.fb_handle,
      leaseId: ow.lease.lease_id,
    }, { overwrite: opts?.overwrite })
  }

  // ---------- data plane read ----------

  /** URL for `GET /nfs/v1/read/{node_id}` (usable in <img>, <a download>, …). */
  readUrl(nodeId: string, opts?: { download?: boolean; name?: string }): string {
    const params = new URLSearchParams()
    if (opts?.download) params.set('download', '1')
    if (opts?.name) params.set('name', opts.name)
    const qs = params.size > 0 ? `?${params.toString()}` : ''
    return `${this.baseUrl}/nfs/v1/read/${encodeURIComponent(nodeId)}${qs}`
  }

  /** Reads file content; `range` is inclusive byte positions. */
  async readFile(
    nodeId: string,
    opts?: { range?: { start: number; end?: number }; ifNoneMatch?: string },
  ): Promise<Response> {
    const headers: Record<string, string> = {}
    if (opts?.range) {
      headers.Range = `bytes=${opts.range.start}-${opts.range.end ?? ''}`
    }
    if (opts?.ifNoneMatch) headers['If-None-Match'] = opts.ifNoneMatch
    const resp = await this.fetchFn(this.readUrl(nodeId), { headers })
    if (!resp.ok && resp.status !== 304) throw await this.httpError(resp)
    return resp
  }

  // ---------- meta ----------

  async getMeta(target: LocatorLike, ns?: string[]): Promise<{ records: MetaRecord[] }> {
    const loc = toLocator(target)
    const args: Record<string, unknown> = { ns }
    if (loc.ref) args.ref = loc.ref
    return (await this.call('get_meta', { at: loc, args })) as { records: MetaRecord[] }
  }

  /** v1: only `ns: "user"` records are writable. */
  async setMeta(
    target: LocatorLike,
    records: { ns: string; key: string; value: unknown; visibility?: string }[],
  ): Promise<{ updated: number }> {
    const loc = toLocator(target)
    const args: Record<string, unknown> = { records }
    if (loc.ref) args.ref = loc.ref
    return (await this.write('set_meta', { at: loc, args })) as { updated: number }
  }

  // ---------- search ----------

  async search(
    q: string,
    opts?: { limit?: number; cursor?: string; scope?: LocatorLike; modes?: string[] },
    want?: WantGroup[],
  ): Promise<SearchResult> {
    return (await this.call('search', {
      want,
      args: {
        q,
        limit: opts?.limit,
        cursor: opts?.cursor,
        scope: opts?.scope !== undefined ? toLocator(opts.scope) : undefined,
        modes: opts?.modes,
      },
    })) as SearchResult
  }

  // ---------- views & collections ----------

  async openView(viewId: string, want?: WantGroup[]): Promise<NodeInfo> {
    return (await this.call('open_view', { want, args: { view_id: viewId } })) as NodeInfo
  }

  async createCollection(
    title: string,
    collectionId?: string,
    want?: WantGroup[],
  ): Promise<NodeInfo> {
    return (await this.write('create_collection', {
      want,
      args: { title, collection_id: collectionId },
    })) as NodeInfo
  }

  async openCollection(collectionId: string, want?: WantGroup[]): Promise<NodeInfo> {
    return (await this.call('open_collection', {
      want,
      args: { collection_id: collectionId },
    })) as NodeInfo
  }

  async collectionPatch(
    ref: WireRef,
    ops: CollectionPatchOp[],
    opts?: { expectedRevision?: string },
  ): Promise<{ revision: string }> {
    return (await this.write('collection_patch', {
      args: { ref, ops, expected_revision: opts?.expectedRevision },
    })) as { revision: string }
  }

  // ---------- grants ----------

  async grant(
    subtree: LocatorLike,
    opts?: { ops?: string[]; ttl?: number; audience?: string; maxUses?: number },
  ): Promise<GrantResult> {
    return (await this.write('grant', {
      args: {
        subtree: toLocator(subtree),
        ops: opts?.ops,
        ttl: opts?.ttl,
        audience: opts?.audience,
        max_uses: opts?.maxUses,
      },
    })) as GrantResult
  }

  async revoke(capId: string): Promise<{ revoked: string }> {
    return (await this.write('revoke', { args: { cap_id: capId } })) as { revoked: string }
  }

  // ---------- watch (SSE) ----------

  /**
   * Opens the watch stream. `tokens` filters to specific containers using the
   * `watch_token` returned by list. The stream is lossy by contract: on any
   * `resync` event, re-list watched containers.
   */
  watch(opts?: { tokens?: string[]; signal?: AbortSignal }): AsyncIterableIterator<WatchEvent> & {
    close: () => void
  } {
    if (!this.session) throw new Error('watch requires an active session; call hello() first')
    const params = new URLSearchParams({ session: this.session })
    if (opts?.tokens?.length) params.set('tokens', opts.tokens.join(','))
    const url = `${this.baseUrl}/nfs/v1/watch?${params.toString()}`
    const controller = new AbortController()
    if (opts?.signal) {
      opts.signal.addEventListener('abort', () => controller.abort(), { once: true })
    }
    const fetchFn = this.fetchFn

    async function* events(): AsyncIterableIterator<WatchEvent> {
      const resp = await fetchFn(url, {
        headers: { Accept: 'text/event-stream' },
        signal: controller.signal,
      })
      if (!resp.ok || !resp.body) {
        throw new Error(`watch failed: HTTP ${resp.status}`)
      }
      const reader = resp.body.getReader()
      const decoder = new TextDecoder()
      let buf = ''
      try {
        for (;;) {
          const { done, value } = await reader.read()
          if (done) return
          buf += decoder.decode(value, { stream: true })
          // SSE frames are separated by a blank line.
          for (;;) {
            const sep = buf.indexOf('\n\n')
            if (sep < 0) break
            const frame = buf.slice(0, sep)
            buf = buf.slice(sep + 2)
            let event = 'message'
            let id: string | undefined
            const dataLines: string[] = []
            for (const line of frame.split('\n')) {
              if (line.startsWith('event:')) event = line.slice(6).trim()
              else if (line.startsWith('data:')) dataLines.push(line.slice(5).trim())
              else if (line.startsWith('id:')) id = line.slice(3).trim()
              // comment lines (`:keep-alive`) are ignored
            }
            if (dataLines.length === 0 && event === 'message') continue
            let data: Record<string, unknown> = {}
            if (dataLines.length > 0) {
              try {
                data = JSON.parse(dataLines.join('\n')) as Record<string, unknown>
              } catch {
                data = { raw: dataLines.join('\n') }
              }
            }
            yield { event, data, id }
          }
        }
      } finally {
        reader.cancel().catch(() => {})
      }
    }

    const it = events() as AsyncIterableIterator<WatchEvent> & { close: () => void }
    it.close = () => controller.abort()
    return it
  }

  // ---------- envelope plumbing ----------

  /** Read-path call: no seq. Throws NfspError on `ok:false`. */
  async call(method: string, extras: EnvelopeExtras): Promise<unknown> {
    return this.post(method, this.envelope(extras))
  }

  /**
   * Write-path call: assigns the next seq once and reuses it across network
   * retries so the server's replay window guarantees exactly-once.
   */
  async write(method: string, extras: EnvelopeExtras, retries = 2): Promise<unknown> {
    const seq = ++this.seq
    const body = { ...this.envelope(extras), seq }
    let lastErr: unknown
    for (let attempt = 0; attempt <= retries; attempt++) {
      try {
        return await this.post(method, body)
      } catch (e) {
        // Only network-level failures are retried (same seq → replay-safe).
        // NfspError means the server answered: propagate immediately.
        if (e instanceof NfspError) throw e
        lastErr = e
      }
    }
    throw lastErr
  }

  private envelope(extras: EnvelopeExtras): Record<string, unknown> {
    const body: Record<string, unknown> = { args: pruneUndefined(extras.args ?? {}) }
    if (this.session) body.session = this.session
    if (extras.at) body.at = pruneUndefined(extras.at as unknown as Record<string, unknown>)
    if (extras.want) body.want = extras.want
    return body
  }

  private async post(method: string, body: Record<string, unknown>): Promise<unknown> {
    const resp = await this.fetchFn(`${this.baseUrl}/nfs/v1/${method}`, {
      method: 'POST',
      headers: { 'Content-Type': 'application/json' },
      body: JSON.stringify(body),
    })
    let parsed: { ok?: boolean; result?: unknown; error?: NfspErrorBody }
    try {
      parsed = (await resp.json()) as typeof parsed
    } catch {
      throw new NfspError(
        { code: 'INTERNAL', message: `non-JSON response (HTTP ${resp.status})` },
        resp.status,
      )
    }
    if (parsed.ok !== true) {
      throw new NfspError(
        parsed.error ?? { code: 'INTERNAL', message: 'malformed error envelope' },
        resp.status,
      )
    }
    return parsed.result
  }

  private async httpError(resp: Response): Promise<NfspError> {
    try {
      const parsed = (await resp.json()) as { error?: NfspErrorBody }
      if (parsed.error) return new NfspError(parsed.error, resp.status)
    } catch {
      // fall through
    }
    return new NfspError({ code: 'INTERNAL', message: `HTTP ${resp.status}` }, resp.status)
  }
}

/** Drops undefined values so they don't serialize as JSON nulls. */
const pruneUndefined = (obj: Record<string, unknown>): Record<string, unknown> => {
  const out: Record<string, unknown> = {}
  for (const [k, v] of Object.entries(obj)) {
    if (v !== undefined) out[k] = v
  }
  return out
}
