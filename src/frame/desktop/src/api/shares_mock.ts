/**
 * In-memory share service for the mock runtime (`VITE_CP_USE_MOCK`,
 * `VITE_MY_CONTENT_USE_MOCK`, `?shareData=mock`). It follows nfs-server's
 * `share.*` contract closely enough for the dialog, My Content and e2e tests:
 * sources containing "big" run as background operations, PDFs and folders go
 * to HomeStation as links, and seeded shares carry 30 days of statistics.
 */

import type {
  PublicListing,
  ShareCapabilities,
  ShareDetail,
  ShareHandoff,
  ShareInfo,
  ShareOperation,
  SharePlan,
  SharePolicyInput,
  ShareStatsPoint,
  ShareTargetApp,
  ShareTargetFit,
} from './nfsp_client'
import { ShareError, type ShareChanges, type ShareService } from './shares'

const SHARE_BASE = 'https://share.example.zone'
const PUBLIC_BASE = 'https://public.example.zone'
const DAY = 86400
const MIB = 1024 * 1024
const HOMESTATION_MAX = 32 * MIB
const MESSAGEHUB_MAX = 32 * MIB

const now = () => Math.floor(Date.now() / 1000)
const wait = (ms = 120) => new Promise((resolve) => setTimeout(resolve, ms))
const randomId = (prefix: string) => `${prefix}_${Math.random().toString(16).slice(2, 10)}${Math.random().toString(16).slice(2, 10)}`

function hash(text: string): number {
  let h = 2166136261
  for (let i = 0; i < text.length; i++) h = Math.imul(h ^ text.charCodeAt(i), 16777619)
  return h >>> 0
}

function baseName(path: string) {
  return path.replace(/\/+$/, '').split('/').pop() || path
}

function mimeOf(name: string): string | null {
  const ext = name.includes('.') ? name.split('.').pop()!.toLowerCase() : ''
  const table: Record<string, string> = {
    jpg: 'image/jpeg', jpeg: 'image/jpeg', png: 'image/png', gif: 'image/gif', webp: 'image/webp', heic: 'image/heic',
    mp4: 'video/mp4', mov: 'video/quicktime', webm: 'video/webm', mp3: 'audio/mpeg', wav: 'audio/wav', flac: 'audio/flac',
    pdf: 'application/pdf', txt: 'text/plain', md: 'text/plain', zip: 'application/zip', html: 'text/html',
  }
  return table[ext] ?? (ext ? 'application/octet-stream' : null)
}

function kindOf(mime: string | null, dir: boolean): ShareInfo['content_kind'] {
  if (dir) return 'folder'
  const major = mime?.split('/')[0]
  return major === 'image' || major === 'video' || major === 'audio' ? major : 'file'
}

interface MockSource {
  path: string
  name: string
  dir: boolean
  mime: string | null
  size: number
  items: number
}

function describe(source: string): MockSource {
  const path = source.startsWith('/') ? source : `/${source}`
  const name = baseName(path)
  const dir = !name.includes('.')
  const size = name.includes('big') ? 300 * MIB : dir ? 4 * MIB + (hash(path) % (20 * MIB)) : 2048 + (hash(path) % (6 * MIB))
  return { path, name, dir, mime: dir ? null : mimeOf(name), size, items: dir ? 3 + (hash(path) % 40) : 1 }
}

function fit(target: ShareTargetApp, src: MockSource): ShareTargetFit {
  const kind = kindOf(src.mime, src.dir) ?? 'file'
  if (src.dir) return { app: target, mode: 'link', reason: 'folder', kind }
  if (target === 'homestation') {
    if (!['image', 'video', 'audio'].includes(kind)) return { app: target, mode: 'link', reason: 'not_media', kind }
    if (src.size > HOMESTATION_MAX) return { app: target, mode: 'link', reason: 'too_large', kind }
  } else if (src.size > MESSAGEHUB_MAX) {
    return { app: target, mode: 'link', reason: 'too_large', kind }
  }
  return { app: target, mode: 'attachment', reason: null, kind }
}

interface Record_ {
  info: ShareInfo
  password?: string
  audit: ShareDetail['audit']
  series: ShareStatsPoint[]
}

interface MockOp {
  op: ShareOperation
  startedAt: number
  finish: () => void
}

function seriesFor(seed: string, days: number, scale: number): ShareStatsPoint[] {
  const out: ShareStatsPoint[] = []
  const today = now() - (now() % DAY)
  for (let d = days - 1; d >= 0; d--) {
    const h = hash(`${seed}:${d}`)
    const views = Math.round(((h % 17) + (d < 5 ? 6 : 1)) * scale)
    if (!views) continue
    const downloads = Math.round(views * ((h >>> 5) % 5) / 10)
    out.push({ t: today - d * DAY, views, downloads, bytes: downloads * (2 + (h % 30)) * MIB, visitors: Math.max(1, Math.round(views * 0.6)) })
  }
  return out
}

export function createMockShareService(): ShareService {
  const shares = new Map<string, Record_>()
  const ops = new Map<string, MockOp>()
  const keys = new Map<string, string>()

  const audit = (action: string, revision: number, reason?: string): ShareDetail['audit'][number] => ({
    action, revision_before: action === 'create' ? null : revision - 1, revision_after: revision,
    actor_user_id: 'devtest', actor_app_id: null, source: 'gui', reason: reason ?? null, outcome: 'ok', at: now(),
  })

  const statsOf = (rec: Record_) => {
    const total = rec.series.reduce((a, p) => ({ views: a.views + p.views, downloads: a.downloads + p.downloads, bytes: a.bytes + p.bytes, visitors: a.visitors + p.visitors }), { views: 0, downloads: 0, bytes: 0, visitors: 0 })
    return { ...total, last_access_at: rec.series.length ? rec.series[rec.series.length - 1].t + 3600 : null }
  }

  const view = (rec: Record_): ShareInfo => {
    const info = { ...rec.info, stats: statsOf(rec) }
    if (info.status === 'active' && info.expires_at && info.expires_at <= now()) info.status = 'expired'
    return info
  }

  const insert = (src: MockSource, kind: ShareInfo['kind'], extra: Partial<ShareInfo>, series: ShareStatsPoint[] = []): Record_ => {
    const id = randomId('sh')
    const t = extra.created_at ?? now()
    const info: ShareInfo = {
      share_id: id,
      kind,
      title: src.name,
      status: 'active',
      hidden: false,
      revision: 1,
      created_at: t,
      updated_at: t,
      owner_user_id: 'devtest',
      obj_id: `${src.dir ? 'cydir' : 'cyfile'}:${hash(src.path + t).toString(16).padStart(8, '0')}${'0'.repeat(56)}`,
      obj_type: src.dir ? 'cydir' : 'cyfile',
      name: src.name,
      mime: src.mime,
      total_size: src.size,
      item_count: src.items,
      content_kind: kindOf(src.mime, src.dir),
      excluded: src.dir ? [{ path: '.env', reason: 'secret' }] : [],
      prepared_at: t,
      source_path: `cyfs://${src.path}`,
      retained: true,
      stats: { views: 0, downloads: 0, bytes: 0, visitors: 0 },
      ...extra,
    }
    if (kind === 'link') {
      info.access ??= 'holder'
      info.has_password ??= false
      info.allow_download ??= true
      info.url = `${SHARE_BASE}/s/${randomId('t').slice(2)}${randomId('t').slice(2)}`
    }
    const rec: Record_ = { info, audit: [audit('create', 1)], series }
    shares.set(id, rec)
    return rec
  }

  // Seed: a few links and app deliveries with history.
  const t0 = now()
  insert(describe('/home/devtest/Documents/Q3-report.pdf'), 'link', { created_at: t0 - 12 * DAY, expires_at: t0 + 2 * DAY }, seriesFor('q3', 12, 1.4))
  insert(describe('/home/devtest/Pictures/Trip'), 'link', { created_at: t0 - 25 * DAY, expires_at: null, access: 'password', has_password: true }, seriesFor('trip', 25, 0.8))
  insert(describe('/home/devtest/Videos/demo.mp4'), 'link', { created_at: t0 - 40 * DAY, expires_at: t0 - 10 * DAY, status: 'expired', retained: false, stopped_at: t0 - 10 * DAY }, seriesFor('demo', 30, 0.5))
  insert(describe('/home/devtest/Documents/notes.txt'), 'link', { created_at: t0 - 8 * DAY, expires_at: t0 + 20 * DAY, status: 'stopped', stopped_at: t0 - 3 * DAY, retained: false, revision: 2 }, seriesFor('notes', 5, 0.3))
  insert(describe('/home/devtest/Pictures/sunset.jpg'), 'app_delivery', { created_at: t0 - 2 * DAY, target_app: 'homestation', delivery_state: 'confirmed', handoff_at: t0 - 2 * DAY, reported_at: (t0 - 2 * DAY + 40) * 1000, business_ref: { entry: 'cyfs://example.zone/home/devtest/posts/@/p1' } })
  insert(describe('/home/devtest/Documents/contract.pdf'), 'app_delivery', { created_at: t0 - 1 * DAY, target_app: 'messagehub', delivery_state: 'handed_off', handoff_at: t0 - DAY })

  const find = (id: string) => {
    const rec = shares.get(id)
    if (!rec) throw new ShareError(`share '${id}' not found`, 'NOT_FOUND')
    return rec
  }

  const opView = (id: string): ShareOperation => {
    const m = ops.get(id)
    if (!m) throw new ShareError(`operation '${id}' not found`, 'NOT_FOUND')
    if (m.op.state === 'running') {
      const elapsed = Date.now() - m.startedAt
      const total = m.op.progress.total_bytes ?? 0
      m.op.progress = { total_bytes: total, done_bytes: Math.min(total, Math.round((total * elapsed) / 2500)) }
      if (elapsed >= 2500) m.finish()
    }
    return structuredClone(m.op)
  }

  const handoffView = (delivery: Record_, link?: Record_): ShareHandoff => {
    const d = delivery.info
    const mode = link ? 'link' : 'attachment'
    const target = d.target_app as ShareTargetApp
    const payload = target === 'messagehub'
      ? { share: link ? { shareId: d.share_id, name: d.name, size: d.total_size, mime: d.mime, url: link.info.url, title: d.title } : { shareId: d.share_id, objId: d.obj_id, name: d.name, size: d.total_size, mime: d.mime } }
      : { compose: link ? { shareId: d.share_id, link: { url: link.info.url, title: d.title.slice(0, 120) } } : { shareId: d.share_id, attachments: [{ kind: d.content_kind, name: d.name, object: d.obj_id, status: 'uploaded' }] } }
    return {
      share_id: d.share_id, obj_id: d.obj_id, name: d.name ?? d.title, size: d.total_size ?? 0, mime: d.mime, kind: d.content_kind ?? 'file',
      mode, link_share_id: link?.info.share_id ?? null, url: link?.info.url ?? null, launch: { appId: target, payload },
    }
  }

  const startOp = (intent: ShareOperation['intent'], key: string | undefined, src: MockSource, make: () => { share: Record_; handoff?: ShareHandoff }): ShareOperation => {
    if (key && keys.has(key)) return opView(keys.get(key)!)
    const id = randomId('op')
    const op: ShareOperation = { operation_id: id, intent, state: 'running', progress: { done_bytes: 0, total_bytes: src.size }, created_at: now(), updated_at: now() }
    const m: MockOp = {
      op,
      startedAt: Date.now(),
      finish: () => {
        const made = make()
        m.op = { ...m.op, state: 'completed', share_id: made.share.info.share_id, updated_at: now(), progress: { done_bytes: src.size, total_bytes: src.size }, share: view(made.share), url: made.share.info.url ?? null, handoff: made.handoff ?? null, obj_id: made.share.info.obj_id, source_changed: false }
      },
    }
    ops.set(id, m)
    if (key) keys.set(key, id)
    if (src.size <= 64 * MIB) m.finish()
    return opView(id)
  }

  const service: ShareService = {
    async capabilities(): Promise<ShareCapabilities> {
      await wait(60)
      return {
        actions: ['create_link', 'app_handoff'],
        access: ['holder', 'password'],
        expiry_presets: [DAY, 7 * DAY, 30 * DAY, null],
        default_expiry: 7 * DAY,
        limits: { sync_threshold: 64 * MIB, max_total_size: 64 * 1024 * MIB, max_items: 20000 },
        targets: [
          { app: 'messagehub', accepts: 'any', max_item_size: MESSAGEHUB_MAX, fallback: 'link' },
          { app: 'homestation', accepts: ['image', 'video', 'audio'], max_item_size: HOMESTATION_MAX, fallback: 'link' },
        ],
        blocked: [],
        share_base_url: SHARE_BASE,
        public: { dir: 'cyfs:///srv/publish', base_url: PUBLIC_BASE },
      }
    },
    async plan(source, opts): Promise<SharePlan> {
      await wait()
      const src = describe(source)
      const existing = [...shares.values()].filter((r) => r.info.kind === 'link' && r.info.source_path === `cyfs://${src.path}` && view(r).status === 'active').map(view)
      const inPublish = src.path.startsWith('/srv/publish/')
      return {
        plan_fingerprint: hash(src.path).toString(16),
        source: { path: `cyfs://${src.path}`, name: src.name, kind: src.dir ? 'dir' : 'file', mime: src.mime },
        estimate: { total_size: src.size, item_count: src.items, excluded: src.dir ? [{ path: '.env', reason: 'secret' }] : [] },
        warnings: [...(src.dir ? ['excluded_items'] : []), ...(src.size > 64 * MIB ? ['runs_in_background'] : []), ...(inPublish ? ['already_public'] : [])],
        requires_task: src.size > 64 * MIB,
        existing,
        source_public: { in_publish_dir: inPublish, public_acl: inPublish, public_url: inPublish ? `${PUBLIC_BASE}${src.path.slice('/srv/publish'.length)}` : null },
        site: { has_index_html: src.dir && /site|web|www/i.test(src.name) },
        target_fit: opts?.target ? fit(opts.target, src) : undefined,
      }
    },
    async create(source, policy: SharePolicyInput, opts) {
      await wait()
      if (policy.access === 'password' && !policy.password) throw new ShareError('password required', 'INVALID_ARGUMENT')
      const src = describe(source)
      return startOp('create_link', opts?.idempotencyKey, src, () => ({
        share: insert(src, 'link', {
          title: opts?.title || src.name,
          access: policy.password ? 'password' : 'holder',
          has_password: !!policy.password,
          allow_download: policy.allow_download ?? true,
          expires_at: policy.expires_at === undefined ? now() + 7 * DAY : policy.expires_at,
        }),
      }))
    },
    async operation(id, opts) {
      await wait(40)
      if (opts?.cancel) {
        const m = ops.get(id)
        if (m && m.op.state === 'running') m.op = { ...m.op, state: 'canceled', error: { code: 'INTERNAL', message: 'snapshot canceled', reason: 'canceled' } }
      }
      return opView(id)
    },
    async get(id) {
      await wait(60)
      const rec = find(id)
      const related = rec.info.related_share_id ? shares.get(rec.info.related_share_id) : undefined
      return {
        ...view(rec),
        retention: [{ obj_id: rec.info.obj_id, pin_owner: `share:${id}`, pinned_at: rec.info.created_at, released_at: rec.info.retained ? null : rec.info.stopped_at ?? now() }],
        audit: [...rec.audit].reverse(),
        related: related ? view(related) : undefined,
      }
    },
    async list(filter) {
      await wait(80)
      let items = [...shares.values()].map(view).sort((a, b) => b.created_at - a.created_at)
      if (filter?.kind) items = items.filter((s) => s.kind === filter.kind)
      if (filter?.status) items = items.filter((s) => s.status === filter.status)
      if (!filter?.includeHidden) items = items.filter((s) => !s.hidden)
      if (filter?.q) {
        const q = filter.q.toLowerCase()
        items = items.filter((s) => s.title.toLowerCase().includes(q) || s.source_path.toLowerCase().includes(q))
      }
      const offset = Number(filter?.cursor ?? 0)
      const limit = filter?.limit ?? 50
      return { items: items.slice(offset, offset + limit), next_cursor: offset + limit < items.length ? String(offset + limit) : null }
    },
    async update(id, revision, changes: ShareChanges) {
      await wait()
      const rec = find(id)
      if (rec.info.revision !== revision) throw new ShareError('revision conflict', 'REV_MISMATCH', 'revision_conflict', { current: view(rec) })
      if (view(rec).status !== 'active') throw new ShareError('share is not active', 'STALE', 'share_not_active')
      if (changes.title) rec.info.title = changes.title
      if (changes.expires_at !== undefined) rec.info.expires_at = changes.expires_at
      if (changes.password !== undefined) {
        rec.password = changes.password ?? undefined
        rec.info.has_password = !!changes.password
        rec.info.access = changes.password ? 'password' : 'holder'
      }
      if (changes.allow_download !== undefined) rec.info.allow_download = changes.allow_download
      rec.info.revision += 1
      rec.info.updated_at = now()
      rec.audit.push(audit('update', rec.info.revision))
      return { revision: rec.info.revision, share: view(rec) }
    },
    async stop(id, revision, reason) {
      await wait()
      const rec = find(id)
      if (['stopped', 'expired'].includes(view(rec).status)) return { status: view(rec).status, revision: rec.info.revision }
      if (rec.info.revision !== revision) throw new ShareError('revision conflict', 'REV_MISMATCH', 'revision_conflict', { current: view(rec) })
      rec.info.revision += 1
      rec.info.status = 'stopped'
      rec.info.stopped_at = now()
      rec.info.retained = false
      rec.info.status_reason = reason ?? null
      rec.audit.push(audit('stop', rec.info.revision, reason))
      return { status: 'stopped', revision: rec.info.revision }
    },
    async hide(id, hidden) {
      await wait(60)
      const rec = find(id)
      rec.info.hidden = hidden
      rec.info.revision += 1
      rec.audit.push(audit('hide', rec.info.revision, hidden ? 'hide' : 'show'))
      return { hidden, revision: rec.info.revision }
    },
    async handoff(source, target, opts) {
      await wait()
      const src = describe(source)
      const f = fit(target, src)
      return startOp('app_handoff', opts?.idempotencyKey, src, () => {
        const delivery = insert(src, 'app_delivery', { title: opts?.title || src.name, target_app: target, delivery_state: 'handed_off', handoff_at: now() })
        let link: Record_ | undefined
        if (f.mode === 'link') {
          link = insert(src, 'link', { title: delivery.info.title, expires_at: null, related_share_id: delivery.info.share_id })
          delivery.info.related_share_id = link.info.share_id
        }
        return { share: delivery, handoff: handoffView(delivery, link) }
      })
    },
    async report(id, state, opts) {
      await wait(40)
      const rec = find(id)
      const at = opts?.observedAt ?? Date.now()
      if (rec.info.reported_at && rec.info.reported_at > at) throw new ShareError('stale report', 'STALE', 'stale_report')
      rec.info.delivery_state = state
      rec.info.reported_at = at
      if (opts?.businessRef) rec.info.business_ref = opts.businessRef
      if (state === 'canceled' || state === 'failed') {
        for (const r of [rec, rec.info.related_share_id ? shares.get(rec.info.related_share_id) : undefined]) {
          if (r && r.info.status === 'active') Object.assign(r.info, { status: 'stopped', stopped_at: now(), retained: false, status_reason: state })
        }
      }
      return { accepted: true, delivery_state: state }
    },
    async publicList(): Promise<PublicListing> {
      await wait()
      const t = now()
      const entry = (rel_path: string, entry_kind: 'file' | 'dir' | 'symlink', size: number | null, extra = {}) => ({
        rel_path, entry_kind, size, mtime: t - 3 * DAY, first_seen_at: t - 20 * DAY, last_seen_at: t, missing_since: null, link_target: null, link_ok: null,
        url: entry_kind === 'dir' ? null : `${PUBLIC_BASE}/${rel_path}`, ...extra,
      })
      return {
        entries: [
          entry('index.html', 'file', 2048),
          entry('resume.pdf', 'file', 182_000),
          entry('talks', 'dir', null),
          entry('talks/personal-server-talk.pdf', 'file', 4_200_000),
          entry('photos', 'symlink', null, { link_target: '/etc/passwd', link_ok: false }),
          entry('old-banner.png', 'file', 90_000, { missing_since: t - DAY, url: null }),
        ],
        last_scan_at: t - 60,
        scan_ok: true,
        dir: 'cyfs:///srv/publish',
        base_url: PUBLIC_BASE,
      }
    },
    async stats(opts) {
      await wait(80)
      const to = opts?.to ?? now()
      const from = opts?.from ?? to - 30 * DAY
      const recs = opts?.shareId ? [find(opts.shareId)] : [...shares.values()]
      const byT = new Map<number, ShareStatsPoint>()
      for (const r of recs) {
        for (const p of r.series) {
          if (p.t < from || p.t >= to) continue
          const cur = byT.get(p.t) ?? { t: p.t, views: 0, downloads: 0, bytes: 0, visitors: 0 }
          byT.set(p.t, { t: p.t, views: cur.views + p.views, downloads: cur.downloads + p.downloads, bytes: cur.bytes + p.bytes, visitors: cur.visitors + p.visitors })
        }
      }
      const series = [...byT.values()].sort((a, b) => a.t - b.t)
      const totals = series.reduce((a, p) => ({ views: a.views + p.views, downloads: a.downloads + p.downloads, bytes: a.bytes + p.bytes, visitors: a.visitors + p.visitors }), { views: 0, downloads: 0, bytes: 0, visitors: 0 })
      const active = [...shares.values()].map(view).filter((s) => s.status === 'active')
      return {
        granularity: opts?.granularity === 'hour' ? 'hour' : 'day',
        from,
        to,
        series,
        totals,
        summary: { active: active.length, expiring_soon: active.filter((s) => s.expires_at && s.expires_at - now() <= 3 * DAY).length, expiring_within: 3 * DAY },
        unavailable: ['public', 'messagehub', 'homestation'],
      }
    },
  }
  return service
}
