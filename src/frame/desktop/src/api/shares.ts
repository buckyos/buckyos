/**
 * Shares — the data seam for the system share dialog (`sysdlg/share`), My
 * Content and the app targets (MessageHub, HomeStation) that report back.
 *
 * The real implementation talks to nfs-server's `share.*` NFSP methods over
 * the File Browser's session (one NFSP session per Desktop); the mock keeps
 * an in-memory set so the UI runs without a backend. Sources are DFS paths
 * (`/home/alice/report.pdf`), as the File Browser shows them.
 */

import type {
  PublicListing,
  ShareCapabilities,
  ShareDeliveryState,
  ShareDetail,
  ShareInfo,
  ShareListFilter,
  ShareOperation,
  SharePlan,
  SharePolicyInput,
  ShareStatsResult,
  ShareStatus,
  ShareTargetApp,
} from './nfsp_client'
import { NfspError } from './nfsp_client'
import { isMockRuntime } from '../runtime'

export type {
  PublicEntry,
  PublicListing,
  ShareAccess,
  ShareCapabilities,
  ShareContentKind,
  ShareDeliveryState,
  ShareDetail,
  ShareHandoff,
  ShareInfo,
  ShareKind,
  ShareListFilter,
  ShareOperation,
  SharePlan,
  SharePolicyInput,
  ShareStatsPoint,
  ShareStatsResult,
  ShareStatus,
  ShareTargetApp,
} from './nfsp_client'

export interface ShareChanges {
  title?: string
  expires_at?: number | null
  password?: string | null
  allow_download?: boolean
}

export interface ShareService {
  capabilities(): Promise<ShareCapabilities>
  plan(source: string, opts?: { intent?: 'create_link' | 'app_handoff'; target?: ShareTargetApp }): Promise<SharePlan>
  create(source: string, policy: SharePolicyInput, opts?: { title?: string; idempotencyKey?: string; planFingerprint?: string }): Promise<ShareOperation>
  operation(operationId: string, opts?: { cancel?: boolean }): Promise<ShareOperation>
  get(shareId: string): Promise<ShareDetail>
  list(filter?: ShareListFilter): Promise<{ items: ShareInfo[]; next_cursor?: string | null }>
  update(shareId: string, revision: number, changes: ShareChanges): Promise<{ revision: number; share: ShareInfo }>
  stop(shareId: string, revision: number, reason?: string): Promise<{ status: ShareStatus; revision: number }>
  hide(shareId: string, hidden: boolean): Promise<{ hidden: boolean; revision: number }>
  handoff(source: string, target: ShareTargetApp, opts?: { title?: string; idempotencyKey?: string }): Promise<ShareOperation>
  report(
    shareId: string,
    state: Exclude<ShareDeliveryState, 'handed_off'>,
    opts?: { businessRef?: Record<string, unknown>; error?: string; observedAt?: number },
  ): Promise<{ accepted: boolean; delivery_state: string }>
  publicList(opts?: { refresh?: boolean }): Promise<PublicListing>
  stats(opts?: { shareId?: string; from?: number; to?: number; granularity?: 'hour' | 'day' }): Promise<ShareStatsResult>
}

/** Error with the server's stable business reason (`source_not_readable`, `revision_conflict`, …). */
export class ShareError extends Error {
  readonly code: string
  readonly reason?: string
  readonly details: Record<string, unknown>

  constructor(message: string, code: string, reason?: string, details: Record<string, unknown> = {}) {
    super(message)
    this.name = 'ShareError'
    this.code = code
    this.reason = reason
    this.details = details
  }
}

export function toShareError(err: unknown): ShareError {
  if (err instanceof ShareError) return err
  if (err instanceof NfspError) {
    const reason = typeof err.details.reason === 'string' ? err.details.reason : undefined
    return new ShareError(err.message, err.code, reason, err.details)
  }
  return new ShareError(err instanceof Error ? err.message : String(err), 'INTERNAL')
}

let active: ShareService | null = null
let overrideMode: 'mock' | 'nfsp' | null | undefined

function modeOverride(): 'mock' | 'nfsp' | null {
  if (overrideMode !== undefined) return overrideMode
  try {
    const params = new URLSearchParams(window.location.search)
    const raw = params.get('shareData') ?? params.get('fbData')
    overrideMode = raw === 'mock' || raw === 'nfsp' ? raw : null
  } catch {
    overrideMode = null
  }
  return overrideMode
}

function mockSelected(): boolean {
  const flag = String(import.meta.env?.VITE_MY_CONTENT_USE_MOCK ?? '').trim().toLowerCase()
  if (['1', 'true', 'yes', 'mock'].includes(flag)) return true
  return (modeOverride() ?? (isMockRuntime() ? 'mock' : 'nfsp')) === 'mock'
}

export function registerShareService(service: ShareService): () => void {
  active = service
  return () => {
    if (active === service) active = null
  }
}

/** The active share service (mock in the mock runtime, NFSP otherwise). */
export async function shareService(): Promise<ShareService> {
  if (active) return active
  const impl = mockSelected() ? (await import('./shares_mock')).createMockShareService() : await createNfspShareService()
  active ??= impl
  return active
}

async function createNfspShareService(): Promise<ShareService> {
  const { ensureSession, nfspClient } = await import('../app/filebrowser/data/nfsp/client')
  const run = async <T>(op: (client: ReturnType<typeof nfspClient>['raw']) => Promise<T>): Promise<T> => {
    try {
      await ensureSession()
      return await op(nfspClient().raw)
    } catch (err) {
      throw toShareError(err)
    }
  }
  return {
    capabilities: () => run((c) => c.shareCapabilities()),
    plan: (source, opts) => run((c) => c.sharePlan(source, opts)),
    create: (source, policy, opts) => run((c) => c.shareCreate(source, policy, opts)),
    operation: (id, opts) => run((c) => c.shareOperation(id, opts)),
    get: (id) => run((c) => c.shareGet(id)),
    list: (filter) => run((c) => c.shareList(filter)),
    update: (id, revision, changes) => run((c) => c.shareUpdate(id, revision, changes)),
    stop: (id, revision, reason) => run((c) => c.shareStop(id, revision, reason)),
    hide: (id, hidden) => run((c) => c.shareHide(id, hidden)),
    handoff: (source, target, opts) => run((c) => c.shareAppHandoff(source, target, opts)),
    report: (id, state, opts) => run((c) => c.shareAppReport(id, state, opts)),
    publicList: (opts) => run((c) => c.sharePublicList(opts)),
    stats: (opts) => run((c) => c.shareStats(opts)),
  }
}

/** Polls a background operation until it settles (or `signal` aborts). */
export async function waitShareOperation(
  service: ShareService,
  op: ShareOperation,
  opts?: { intervalMs?: number; signal?: AbortSignal; onProgress?: (op: ShareOperation) => void },
): Promise<ShareOperation> {
  let cur = op
  while (cur.state === 'running' || cur.state === 'preparing') {
    if (opts?.signal?.aborted) return cur
    await new Promise((resolve) => setTimeout(resolve, opts?.intervalMs ?? 1000))
    cur = await service.operation(cur.operation_id)
    opts?.onProgress?.(cur)
  }
  return cur
}

/** Fire-and-forget report from an app target; failures only lose the "confirmed" mark. */
export function reportShareDelivery(
  shareId: string,
  state: Exclude<ShareDeliveryState, 'handed_off'>,
  opts?: { businessRef?: Record<string, unknown>; error?: string },
) {
  const observedAt = Date.now()
  void shareService()
    .then((service) => service.report(shareId, state, { ...opts, observedAt }))
    .catch((err) => console.warn('share delivery report failed', err))
}
