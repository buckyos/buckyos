/**
 * nfs_browser_client —— File Browser WebUI 的带缓存 NFSP 客户端。
 *
 * 分层:`nfsp_client.ts` 是无状态协议客户端(一次调用 = 一次 HTTP 往返);
 * 本文件的 `NfsBrowserClient` 通过组合包装它,实现协议 §6.1 第 2/3 档缓存
 * (Live Container / 属性按 revision,Meta 按 anchor),存储后端抽象为
 * `CacheStore`(v1 提供 `LocalStorageCacheStore`)。
 * 验证脚本:`test/test_nfs_server/verify_browser_client.ts`(需独立启动 nfs_server)。
 *
 * ## 读路径(resolve / stat / list / getMeta)
 *
 * 默认 stale-while-revalidate:
 * - 命中且未过期 → 直接返回缓存,不发请求;
 * - 命中但过期 → 立即返回缓存,后台 revalidate;结果与条目内 `rev` 不同
 *   (rev 为 null 的条目比较序列化数据)则更新缓存并触发 `onInvalidate(ref)`;
 * - 未命中 → 透传 NfspClient 并写入缓存。
 * 可选 `{ cache }`:`'no-cache'` 强制回源并刷新缓存;`'only-if-cached'`
 * 只查缓存,未命中返回 null。带 cursor 的 list 一律透传不缓存(D10)。
 *
 * TTL:条目年龄 > hello 的 `limits.attr_ttl_ms`(默认 5000ms)视为过期;
 * 携带 revision 的条目(容器)在 watch 健康时放宽为 10×(watch 会推失效);
 * 无 revision 的条目(文件属性)watch 无法定位,始终用基础 TTL。
 *
 * ## 写路径(mkdir / move / delete / bindRef / unlink / commitFile /
 * uploadFile / setMeta / collectionPatch)
 *
 * 全部透传 NfspClient(exactly-once 语义不动),成功后立即失效对应容器的
 * 缓存条目,不等 watch 回环。move 失效 from/to 两个容器;delete 还失效目标
 * 子树的 stat/meta;unlink 无法从 entry_ref 定位父容器,保守失效全部
 * list/stat;setMeta 失效对应 meta。其余方法(probe/search/openWrite/
 * grant/revoke/readUrl/readFile/...)由调用方直接走 `this.raw`。
 *
 * ## 失效通道(优先级从高到低)
 *
 * 1. watch 推送:`container_changed` 按事件 ref 定位容器,revision 与条目内
 *    rev 不同则删除该容器全部条目并触发 `onInvalidate(ref)`;`meta_changed`
 *    按 anchor 失效 meta 条目;`resync`(任何 reason,含连接首事件)清空本
 *    server 全部缓存并触发 `onInvalidate(null)`(watch 有损契约)。
 * 2. 直写失效(见上)。
 * 3. attr_ttl 兜底(见 TTL)。
 *
 * ## watch 生命周期
 *
 * `connectWatch()` / `disconnectWatch()`;`autoWatch`(默认 true)在 hello
 * 成功后自动连接。v1 用无 tokens 过滤的全量 watch。断线指数退避重连
 * (1s→2s→…→30s 封顶);重连首事件恒为 resync,自然触发全量失效。
 * 会话过期(调用抛 PERMISSION_DENIED 且 message 含 "session")时自动重新
 * hello 并重放该次调用一次,watch 连接随之换到新 session。
 *
 * ## 缓存键(§协议三档缓存的第 2/3 档)
 *
 * 键空间 `nfsp:v1:<srvHash>:`,值为 `{ rev, at, data }` JSON。
 * - list:  `...:list:<canon>:<want>:<argsHash>`(只缓存首页)
 * - stat:  `...:stat:<canon>:<want>`(resolve 与无 name 的 stat 共享)
 * - meta:  `...:meta:<canon>:<nsKey>`
 * - 倒排索引:`...:ix:<scope>` → 该 scope 下全部数据键(JSON 数组),
 *   scope 有 `c:<canon>`(定位符)、`r:<refId>`(响应里的容器 ref)、
 *   `ma:<anchor>` / `ma:*`(meta 锚)三类,失效以 scope 为单位。
 * 单条目超过 256KB 不缓存;写入失败(配额)按 `at` LRU 逐出最旧 25% 后
 * 重试一次;所有存储失败都被吞掉,绝不影响正常返回。
 *
 * 非目标:数据面内容缓存(浏览器 HTTP 缓存 + ETag 负责)、离线写队列、
 * Frozen 子树缓存(等服务端阶段二)、跨标签页广播。
 */

import {
  NfspClient,
  NfspError,
  toLocator,
  type CollectionPatchOp,
  type CommitResult,
  type HelloResult,
  type Listing,
  type ListOptions,
  type LocatorLike,
  type MetaRecord,
  type NodeInfo,
  type WantGroup,
  type WatchEvent,
  type WireRef,
} from './nfsp_client.ts'

/** 缓存读取模式(语义对齐 fetch 的 RequestCache 子集)。 */
export type CacheMode = 'default' | 'no-cache' | 'only-if-cached'

export interface CachedReadOptions {
  cache?: CacheMode
}

/**
 * 存储后端抽象。异步签名为未来 IndexedDB/OPFS 实现预留;
 * LocalStorageCacheStore 实际是同步实现。`get`/`delete`/`deletePrefix`/
 * `keys` 内部吞掉存储异常(失败视为未命中/未写入);`set` 允许抛出
 * (如 QuotaExceededError),由 NfsBrowserClient 捕获并走 LRU 逐出路径。
 */
export interface CacheStore {
  get(key: string): Promise<string | null>
  set(key: string, value: string): Promise<void>
  delete(key: string): Promise<void>
  /** 删除所有以 prefix 开头的键。 */
  deletePrefix(prefix: string): Promise<void>
  /** 列出所有以 prefix 开头的键(逐出与倒排索引维护用)。 */
  keys(prefix: string): Promise<string[]>
}

export interface NfsBrowserClientOptions {
  baseUrl: string
  /** 默认 LocalStorageCacheStore;测试注入内存实现。 */
  store?: CacheStore
  /** hello 成功后自动连接 watch,默认 true。 */
  autoWatch?: boolean
  /** 覆盖底层 NfspClient(测试注入计数 fetch 时用)。 */
  client?: NfspClient
}

export interface CacheStats {
  hits: number
  misses: number
  revalidations: number
  evictions: number
}

export type InvalidateListener = (ref: WireRef | null) => void

// ---------------------------------------------------------------------------
// 键与规范化
// ---------------------------------------------------------------------------

const KEY_SCHEMA = 'nfsp:v1:'
const MAX_ENTRY_CHARS = 256 * 1024
const DEFAULT_ATTR_TTL_MS = 5000
/** watch 健康时,携带 revision 的条目的 TTL 放宽倍数。 */
const WATCHED_TTL_FACTOR = 10

const fnv1a = (s: string): string => {
  let h = 0x811c9dc5
  for (let i = 0; i < s.length; i++) {
    h ^= s.charCodeAt(i)
    h = Math.imul(h, 0x01000193)
  }
  return (h >>> 0).toString(36)
}

const normPath = (p: string): string => {
  let s = p.replace(/\/+/g, '/')
  if (!s.startsWith('/')) s = '/' + s
  if (s.length > 1 && s.endsWith('/')) s = s.slice(0, -1)
  return s
}

/** WireRef 的等值身份(live 忽略 gen:同一节点不同 gen 是同一容器)。 */
const refId = (ref: WireRef): string =>
  ref.type === 'live'
    ? ref.node_id
    : ref.inner_path
      ? `${ref.obj_id}:${ref.inner_path}`
      : ref.obj_id

/** locator 规范化:live ref 用 node_id;路径用 realm+归一化路径;uri 原样。 */
const canonLocator = (at: LocatorLike, name?: string): string => {
  const loc = toLocator(at)
  if (loc.ref) return `r_${refId(loc.ref)}` + (name !== undefined ? `#${name}` : '')
  if (loc.uri !== undefined) return `u_${loc.uri}` + (name !== undefined ? `#${name}` : '')
  const p = normPath(loc.path ?? '/')
  return `p_${loc.realm ?? 'dfs'}:${name !== undefined ? normPath(`${p}/${name}`) : p}`
}

const wantKey = (want?: WantGroup[]): string =>
  want && want.length > 0 ? [...want].sort().join('+') : '-'

/** 键序稳定的 JSON(list 参数进缓存键用)。 */
const stableStringify = (v: unknown): string => {
  if (v === null || typeof v !== 'object') return JSON.stringify(v) ?? 'undefined'
  if (Array.isArray(v)) return `[${v.map(stableStringify).join(',')}]`
  const obj = v as Record<string, unknown>
  const parts = Object.keys(obj)
    .sort()
    .filter((k) => obj[k] !== undefined)
    .map((k) => `${JSON.stringify(k)}:${stableStringify(obj[k])}`)
  return `{${parts.join(',')}}`
}

const sleep = (ms: number): Promise<void> => new Promise((res) => setTimeout(res, ms))

interface CacheEntry {
  rev: string | null
  at: number
  data: unknown
}

// ---------------------------------------------------------------------------
// LocalStorageCacheStore
// ---------------------------------------------------------------------------

/**
 * localStorage 后端。隐私模式/禁用存储下访问 localStorage 本身会抛,
 * 读类方法全部吞掉异常;`set` 让配额类错误穿透给上层做 LRU 逐出。
 */
export class LocalStorageCacheStore implements CacheStore {
  private storage(): Storage | null {
    try {
      return globalThis.localStorage ?? null
    } catch {
      return null
    }
  }

  async get(key: string): Promise<string | null> {
    try {
      return this.storage()?.getItem(key) ?? null
    } catch {
      return null
    }
  }

  async set(key: string, value: string): Promise<void> {
    // 不 catch:QuotaExceededError 等由调用方(逐出路径)处理。
    this.storage()?.setItem(key, value)
  }

  async delete(key: string): Promise<void> {
    try {
      this.storage()?.removeItem(key)
    } catch {
      // ignore
    }
  }

  async deletePrefix(prefix: string): Promise<void> {
    for (const k of await this.keys(prefix)) await this.delete(k)
  }

  async keys(prefix: string): Promise<string[]> {
    try {
      const ls = this.storage()
      if (!ls) return []
      const out: string[] = []
      for (let i = 0; i < ls.length; i++) {
        const k = ls.key(i)
        if (k !== null && k.startsWith(prefix)) out.push(k)
      }
      return out
    } catch {
      return []
    }
  }
}

// ---------------------------------------------------------------------------
// NfsBrowserClient
// ---------------------------------------------------------------------------

export class NfsBrowserClient {
  /** 底层无状态协议客户端;不缓存的方法(probe/search/grant/...)直接用它。 */
  readonly raw: NfspClient

  private readonly store: CacheStore
  private readonly autoWatch: boolean
  /** 本 server 的键空间前缀 `nfsp:v1:<srvHash>:`。 */
  private readonly sp: string

  private helloed = false
  private readonly listeners = new Set<InvalidateListener>()
  private readonly stats: CacheStats = { hits: 0, misses: 0, revalidations: 0, evictions: 0 }
  /** 进行中的后台 revalidate(按缓存键去重)。 */
  private readonly inflight = new Map<string, Promise<void>>()

  private watchWanted = false
  private watchRunning = false
  private watchHealthy = false
  private watchConn: { close: () => void } | null = null

  constructor(opts: NfsBrowserClientOptions) {
    this.raw = opts.client ?? new NfspClient({ baseUrl: opts.baseUrl })
    this.store = opts.store ?? new LocalStorageCacheStore()
    this.autoWatch = opts.autoWatch ?? true
    this.sp = `${KEY_SCHEMA}${fnv1a(opts.baseUrl.replace(/\/+$/, ''))}:`
  }

  // ---------- 会话与 watch 生命周期 ----------

  async hello(): Promise<HelloResult> {
    const r = await this.raw.hello()
    this.helloed = true
    if (this.autoWatch) this.connectWatch()
    // 旧 session 上的 watch 流已失效,踢掉让重连循环用新 session 重建。
    if (this.watchRunning) this.watchConn?.close()
    return r
  }

  async bye(): Promise<void> {
    this.disconnectWatch()
    this.helloed = false
    await this.raw.bye()
  }

  connectWatch(): void {
    this.watchWanted = true
    if (!this.watchRunning) void this.watchLoop()
  }

  disconnectWatch(): void {
    this.watchWanted = false
    this.watchConn?.close()
  }

  /** watch 连接当前是否健康(影响容器条目的 TTL 放宽)。 */
  get watchConnected(): boolean {
    return this.watchHealthy
  }

  private async watchLoop(): Promise<void> {
    this.watchRunning = true
    let backoff = 1000
    try {
      while (this.watchWanted) {
        try {
          const stream = this.raw.watch()
          this.watchConn = stream
          for await (const ev of stream) {
            backoff = 1000
            this.watchHealthy = true
            await this.handleWatchEvent(ev)
          }
        } catch {
          // 连接失败或流中断:退避后重试。
        }
        this.watchHealthy = false
        this.watchConn = null
        if (!this.watchWanted) break
        await sleep(backoff)
        backoff = Math.min(backoff * 2, 30000)
      }
    } finally {
      this.watchRunning = false
      this.watchHealthy = false
      this.watchConn = null
    }
  }

  private async handleWatchEvent(ev: WatchEvent): Promise<void> {
    if (ev.event === 'resync') {
      // watch 有损契约:resync 即缓存全体存疑,整个 server 前缀清空。
      await this.safe(() => this.store.deletePrefix(this.sp))
      this.fire(null)
      return
    }
    if (ev.event === 'container_changed') {
      const ref = ev.data.ref as WireRef | undefined
      if (!ref || typeof ref !== 'object') return
      const revision = typeof ev.data.revision === 'string' ? ev.data.revision : undefined
      const removed = await this.invalidateScope(`r:${refId(ref)}`, revision)
      if (removed > 0) this.fire(ref)
      return
    }
    if (ev.event === 'meta_changed') {
      const anchor = ev.data.anchor
      if (typeof anchor === 'string') await this.invalidateScope(`ma:${anchor}`)
      await this.invalidateScope('ma:*')
    }
  }

  // ---------- 带缓存读路径 ----------

  async resolve(at: LocatorLike, want?: WantGroup[], opts?: CachedReadOptions): Promise<NodeInfo | null> {
    return this.cachedRead(
      `${this.sp}stat:${canonLocator(at)}:${wantKey(want)}`,
      opts?.cache,
      () => this.raw.resolve(at, want),
      (info) => this.nodeScopes(at, undefined, info),
      (info) => info.revision ?? null,
      (info) => info.ref,
    )
  }

  async stat(
    at: LocatorLike,
    opts?: { name?: string; want?: WantGroup[] } & CachedReadOptions,
  ): Promise<NodeInfo | null> {
    return this.cachedRead(
      `${this.sp}stat:${canonLocator(at, opts?.name)}:${wantKey(opts?.want)}`,
      opts?.cache,
      () => this.raw.stat(at, { name: opts?.name, want: opts?.want }),
      (info) => this.nodeScopes(at, opts?.name, info),
      (info) => info.revision ?? null,
      (info) => info.ref,
    )
  }

  async list(
    at: LocatorLike,
    listOpts?: ListOptions,
    want?: WantGroup[],
    opts?: CachedReadOptions,
  ): Promise<Listing | null> {
    // 只缓存首页:游标语义依赖服务端实时状态(D10),分页请求一律透传。
    if (listOpts?.cursor !== undefined) {
      if (opts?.cache === 'only-if-cached') return null
      return this.withSession(() => this.raw.list(at, listOpts, want))
    }
    const canon = canonLocator(at)
    return this.cachedRead(
      `${this.sp}list:${canon}:${wantKey(want)}:${fnv1a(stableStringify(listOpts ?? {}))}`,
      opts?.cache,
      () => this.raw.list(at, listOpts, want),
      (listing) => [`c:${canon}`, `r:${refId(listing.container.ref)}`],
      (listing) => listing.container.revision ?? null,
      (listing) => listing.container.ref,
    )
  }

  async getMeta(
    target: LocatorLike,
    ns?: string[],
    opts?: CachedReadOptions,
  ): Promise<{ records: MetaRecord[] } | null> {
    const loc = toLocator(target)
    const canon = canonLocator(target)
    const nsKey = ns && ns.length > 0 ? fnv1a([...ns].sort().join('+')) : '*'
    return this.cachedRead(
      `${this.sp}meta:${canon}:${nsKey}`,
      opts?.cache,
      () => this.raw.getMeta(target, ns),
      (r) => {
        // meta 条目按 anchor 参与 watch 失效;推不出 anchor 的(如路径定位的
        // 空结果)挂到 `ma:*`,任何 meta_changed 都会将其失效(保守正确)。
        const scopes = new Set<string>()
        for (const rec of r.records) if (rec.anchor) scopes.add(`ma:${rec.anchor}`)
        if (loc.ref?.type === 'live') scopes.add(`ma:live:${loc.ref.node_id}`)
        if (scopes.size === 0) scopes.add('ma:*')
        return [...scopes]
      },
      () => null,
      () => loc.ref ?? null,
    )
  }

  // ---------- 写路径:透传 + 直写失效 ----------

  async mkdir(
    parent: LocatorLike,
    name?: string,
    opts?: { expectedRevision?: string },
  ): Promise<{ ref: WireRef; existed: boolean; revision?: string }> {
    const r = await this.withSession(() => this.raw.mkdir(parent, name, opts))
    await this.invalidateContainer(parent)
    const loc = toLocator(parent)
    if (name === undefined && loc.path !== undefined) {
      // 路径形式是 mkdir -p:每一级祖先目录都可能新增了子项。
      let p = normPath(loc.path)
      while (p !== '/') {
        p = p.slice(0, p.lastIndexOf('/')) || '/'
        await this.invalidateScope(`c:p_${loc.realm ?? 'dfs'}:${p}`)
      }
    }
    return r
  }

  async move(
    from: { parentRef: WireRef; name: string },
    to: { parentRef: WireRef; name: string },
    opts?: { expectedFromRevision?: string; expectedToRevision?: string },
  ): Promise<{ from_revision: string; to_revision: string }> {
    const r = await this.withSession(() => this.raw.move(from, to, opts))
    await this.invalidateContainer(from.parentRef)
    await this.invalidateContainer(to.parentRef)
    await this.invalidateSubtree(from.parentRef, from.name)
    return r
  }

  async delete(
    parent: LocatorLike,
    name: string,
    opts?: { recursive?: boolean; expectedRevision?: string },
  ): Promise<{ revision: string }> {
    const r = await this.withSession(() => this.raw.delete(parent, name, opts))
    await this.invalidateContainer(parent)
    await this.invalidateSubtree(parent, name)
    return r
  }

  async bindRef(
    parentRef: WireRef,
    name: string,
    targetRef: WireRef,
    opts?: { expectedRevision?: string },
  ): Promise<{ entry_ref: string; revision: string }> {
    const r = await this.withSession(() => this.raw.bindRef(parentRef, name, targetRef, opts))
    await this.invalidateContainer(parentRef)
    return r
  }

  async unlink(entryRef: string, opts?: { expectedRevision?: string }): Promise<{ revision: string }> {
    const r = await this.withSession(() => this.raw.unlink(entryRef, opts))
    // entry_ref 无法反查父容器:保守失效全部 list/stat(unlink 低频,可接受)。
    await this.safe(() => this.store.deletePrefix(`${this.sp}list:`))
    await this.safe(() => this.store.deletePrefix(`${this.sp}stat:`))
    return r
  }

  async commitFile(
    parent: LocatorLike,
    name: string,
    source: { fbHandle: string; leaseId?: string } | { hash: string },
    opts?: { overwrite?: boolean; expectedRevision?: string },
  ): Promise<CommitResult> {
    const r = await this.withSession(() => this.raw.commitFile(parent, name, source, opts))
    await this.invalidateContainer(parent)
    await this.invalidateSubtree(parent, name)
    return r
  }

  async uploadFile(
    parentRef: WireRef,
    name: string,
    content: Uint8Array,
    opts?: { overwrite?: boolean; onProgress?: (sent: number, total: number) => void },
  ): Promise<CommitResult> {
    const r = await this.withSession(() => this.raw.uploadFile(parentRef, name, content, opts))
    await this.invalidateContainer(parentRef)
    await this.invalidateSubtree(parentRef, name)
    return r
  }

  async setMeta(
    target: LocatorLike,
    records: { ns: string; key: string; value: unknown; visibility?: string }[],
  ): Promise<{ updated: number }> {
    const r = await this.withSession(() => this.raw.setMeta(target, records))
    const loc = toLocator(target)
    await this.safe(() => this.store.deletePrefix(`${this.sp}meta:${canonLocator(target)}:`))
    if (loc.ref?.type === 'live') await this.invalidateScope(`ma:live:${loc.ref.node_id}`)
    return r
  }

  async collectionPatch(
    ref: WireRef,
    ops: CollectionPatchOp[],
    opts?: { expectedRevision?: string },
  ): Promise<{ revision: string }> {
    const r = await this.withSession(() => this.raw.collectionPatch(ref, ops, opts))
    await this.invalidateContainer(ref)
    return r
  }

  // ---------- 事件与统计 ----------

  onInvalidate(listener: InvalidateListener): () => void {
    this.listeners.add(listener)
    return () => this.listeners.delete(listener)
  }

  cacheStats(): CacheStats {
    return { ...this.stats }
  }

  // ---------- 核心缓存逻辑 ----------

  /**
   * stale-while-revalidate 读:泛型骨架,四个读方法共用。
   * `scopesOf`/`revOf`/`refOf` 从响应提取索引 scope、revision 与回调 ref。
   */
  private async cachedRead<T>(
    key: string,
    mode: CacheMode | undefined,
    fetcher: () => Promise<T>,
    scopesOf: (data: T) => string[],
    revOf: (data: T) => string | null,
    refOf: (data: T) => WireRef | null,
  ): Promise<T | null> {
    if (mode === 'no-cache') {
      const data = await this.withSession(fetcher)
      await this.putEntry(key, revOf(data), data, scopesOf(data))
      return data
    }
    const entry = await this.readEntry(key)
    if (mode === 'only-if-cached') {
      if (entry) this.stats.hits++
      else this.stats.misses++
      return entry ? (entry.data as T) : null
    }
    if (entry) {
      this.stats.hits++
      if (Date.now() - entry.at > this.ttlFor(entry)) {
        this.stats.revalidations++
        this.revalidate(key, fetcher, scopesOf, revOf, refOf)
      }
      return entry.data as T
    }
    this.stats.misses++
    const data = await this.withSession(fetcher)
    await this.putEntry(key, revOf(data), data, scopesOf(data))
    return data
  }

  private ttlFor(entry: CacheEntry): number {
    const base = this.raw.limits?.attr_ttl_ms ?? DEFAULT_ATTR_TTL_MS
    // 只有携带 revision 的条目(容器)能靠 watch 推失效,才允许放宽。
    return entry.rev !== null && this.watchHealthy ? base * WATCHED_TTL_FACTOR : base
  }

  /** 后台重验证(按键去重);revision 变化时更新缓存并通知 UI。 */
  private revalidate<T>(
    key: string,
    fetcher: () => Promise<T>,
    scopesOf: (data: T) => string[],
    revOf: (data: T) => string | null,
    refOf: (data: T) => WireRef | null,
  ): void {
    if (this.inflight.has(key)) return
    const job = (async () => {
      try {
        const data = await this.withSession(fetcher)
        // 多标签页共享存储:比较用条目内的 rev(现读),不用内存副本。
        const prev = await this.readEntry(key)
        const newRev = revOf(data)
        const changed =
          prev === null ||
          (newRev === null && prev.rev === null
            ? JSON.stringify(data) !== JSON.stringify(prev.data)
            : newRev !== prev.rev)
        await this.putEntry(key, newRev, data, scopesOf(data))
        if (changed) this.fire(refOf(data))
      } catch (e) {
        if (
          e instanceof NfspError &&
          (e.code === 'NOT_FOUND' || e.code === 'STALE' || e.code === 'NOT_A_CONTAINER')
        ) {
          // 目标已消失:删掉陈旧条目并通知(下一次读会把错误交给调用方)。
          await this.safe(() => this.store.delete(key))
          this.fire(null)
        }
        // 其他错误(网络等):保留陈旧条目,下次读触发重试。
      } finally {
        this.inflight.delete(key)
      }
    })()
    this.inflight.set(key, job)
  }

  /** stat/resolve 条目的索引 scope:定位符 canon + 响应节点 ref。 */
  private nodeScopes(at: LocatorLike, name: string | undefined, info: NodeInfo): string[] {
    return [`c:${canonLocator(at, name)}`, `r:${refId(info.ref)}`]
  }

  /** 直写失效:按容器定位符(canon + ref 两种 scope)删除全部条目。 */
  private async invalidateContainer(at: LocatorLike): Promise<void> {
    const loc = toLocator(at)
    await this.invalidateScope(`c:${canonLocator(at)}`)
    if (loc.ref) await this.invalidateScope(`r:${refId(loc.ref)}`)
  }

  /** delete/move/commit 后:失效 (parent, name) 子树的 stat/meta/list 条目。 */
  private async invalidateSubtree(parent: LocatorLike, name: string): Promise<void> {
    const child = canonLocator(parent, name)
    for (const nsPrefix of ['stat:', 'list:', 'meta:']) {
      // `:` 终结精确命中,`/` 前缀命中路径形式的后代。
      await this.safe(() => this.store.deletePrefix(`${this.sp}${nsPrefix}${child}:`))
      await this.safe(() => this.store.deletePrefix(`${this.sp}${nsPrefix}${child}/`))
    }
  }

  /**
   * 删除一个 scope 下的全部条目。给了 `keepRevision` 时保留 rev 恰好相等的
   * 条目(watch 事件里的 revision 与缓存一致说明缓存已是最新)。
   * 返回实际删除的条目数。
   */
  private async invalidateScope(scope: string, keepRevision?: string): Promise<number> {
    const idxKey = `${this.sp}ix:${scope}`
    const keys = await this.readIndex(idxKey)
    let removed = 0
    const kept: string[] = []
    for (const k of keys) {
      if (keepRevision !== undefined) {
        const e = await this.readEntry(k)
        if (e === null) continue // 已不存在,顺带从索引清掉
        if (e.rev === keepRevision) {
          kept.push(k)
          continue
        }
      }
      await this.safe(() => this.store.delete(k))
      removed++
    }
    if (kept.length > 0) await this.safe(() => this.store.set(idxKey, JSON.stringify(kept)))
    else await this.safe(() => this.store.delete(idxKey))
    return removed
  }

  // ---------- 存储层(全部失败安全) ----------

  private async readEntry(key: string): Promise<CacheEntry | null> {
    const raw = await this.safe(() => this.store.get(key), null)
    if (raw === null) return null
    try {
      const p = JSON.parse(raw) as CacheEntry
      if (p !== null && typeof p === 'object' && typeof p.at === 'number' && 'data' in p) {
        return { rev: typeof p.rev === 'string' ? p.rev : null, at: p.at, data: p.data }
      }
    } catch {
      // 脏数据容忍:视为未命中并删除。
    }
    await this.safe(() => this.store.delete(key))
    return null
  }

  private async putEntry(key: string, rev: string | null, data: unknown, scopes: string[]): Promise<void> {
    try {
      const value = JSON.stringify({ rev, at: Date.now(), data })
      if (value.length > MAX_ENTRY_CHARS) return // 超大条目不缓存,直接透传语义
      if (!(await this.trySet(key, value))) return
      for (const s of scopes) await this.indexAdd(s, key)
    } catch {
      // 缓存失败绝不影响正常返回。
    }
  }

  /** 写入;配额失败时按 LRU 逐出最旧 25% 后重试一次,再失败放弃。 */
  private async trySet(key: string, value: string): Promise<boolean> {
    try {
      await this.store.set(key, value)
      return true
    } catch {
      await this.evictOldest()
    }
    try {
      await this.store.set(key, value)
      return true
    } catch {
      return false
    }
  }

  private async evictOldest(fraction = 0.25): Promise<void> {
    try {
      const aged: { key: string; at: number }[] = []
      for (const nsPrefix of ['list:', 'stat:', 'meta:']) {
        for (const k of await this.safe(() => this.store.keys(this.sp + nsPrefix), [] as string[])) {
          let at = 0
          const raw = await this.safe(() => this.store.get(k), null)
          if (raw !== null) {
            try {
              const p = JSON.parse(raw) as { at?: unknown }
              if (typeof p?.at === 'number') at = p.at
            } catch {
              // 脏条目 at=0,最先逐出
            }
          }
          aged.push({ key: k, at })
        }
      }
      if (aged.length === 0) return
      aged.sort((a, b) => a.at - b.at)
      const n = Math.max(1, Math.ceil(aged.length * fraction))
      for (const { key } of aged.slice(0, n)) {
        await this.safe(() => this.store.delete(key))
        this.stats.evictions++
      }
    } catch {
      // ignore
    }
  }

  private async readIndex(idxKey: string): Promise<string[]> {
    const raw = await this.safe(() => this.store.get(idxKey), null)
    if (raw === null) return []
    try {
      const p = JSON.parse(raw) as unknown
      if (Array.isArray(p)) return p.filter((k): k is string => typeof k === 'string')
    } catch {
      // fall through
    }
    await this.safe(() => this.store.delete(idxKey))
    return []
  }

  private async indexAdd(scope: string, key: string): Promise<void> {
    const idxKey = `${this.sp}ix:${scope}`
    const keys = await this.readIndex(idxKey)
    if (keys.includes(key)) return
    keys.push(key)
    await this.safe(() => this.store.set(idxKey, JSON.stringify(keys)))
  }

  private async safe<T>(fn: () => Promise<T>, fallback?: T): Promise<T> {
    try {
      return await fn()
    } catch {
      return fallback as T
    }
  }

  // ---------- 会话过期自愈 ----------

  /**
   * 会话过期(PERMISSION_DENIED 且 message 提到 session)时自动重新 hello
   * 并重放该次调用一次。业务性的权限拒绝(如受限 meta ns)原样抛出。
   */
  private async withSession<T>(fn: () => Promise<T>): Promise<T> {
    try {
      return await fn()
    } catch (e) {
      if (
        this.helloed &&
        e instanceof NfspError &&
        e.code === 'PERMISSION_DENIED' &&
        e.message.toLowerCase().includes('session')
      ) {
        await this.hello()
        return await fn()
      }
      throw e
    }
  }

  private fire(ref: WireRef | null): void {
    for (const l of this.listeners) {
      try {
        l(ref)
      } catch {
        // listener 异常不影响其他 listener 与主流程
      }
    }
  }
}
