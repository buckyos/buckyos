import { Button, Dialog, DialogActions, DialogContent, DialogTitle, IconButton, Tooltip } from '@mui/material'
import { RefreshCw } from 'lucide-react'
import { useCallback, useEffect, useState } from 'react'
import { useI18n } from '../../../i18n/provider'
import {
  downloadExtToolsOnline,
  fetchExtToolsPlan,
  fetchExtToolsStatus,
  setExtToolsDesired,
  type ExtToolsNodeStatus,
  type ExtToolsOnlineStatus,
  type ExtToolsPackage,
  type ExtToolsPlan,
  type ExtToolsStage,
  type ExtToolsStatusResponse,
} from '../../../api/settings'
import { ListCard, Section, SectionNote, StatusBadge } from '../components/shared/Section'
import { formatBytes } from '../components/shared/format'
import {
  ONLINE_POLL_MS,
  onlineBadge,
  onlineBusy,
  onlineErrorText,
  onlineItemName,
  onlineItemsOf,
  onlineRequest,
  onlineStateLabel,
  requestPending,
  waitingOf,
  type OnlineRequest,
} from './extToolsOnline'
import { OnlineProgress } from './OnlineProgress'

type Translate = ReturnType<typeof useI18n>['t']

const POLL_MS = 5000

/** One online item of one package on one node. */
interface OnlineTarget {
  pkg: string
  item: string
  node: string
}

const requestKey = ({ pkg, item, node }: OnlineTarget) => `${pkg}/${item}/${node}`

interface OnlineActions {
  requests: Record<string, OnlineRequest>
  busy: boolean
  /** `again`: the item is ready there and is downloaded anew, which asks first. */
  onDownload: (target: OnlineTarget, before: ExtToolsOnlineStatus, again: boolean) => void
}

function stageBadge(stage: ExtToolsStage): 'pass' | 'warn' | 'fail' | 'neutral' {
  if (stage === 'active') return 'pass'
  if (stage === 'failed') return 'fail'
  if (stage === 'unsupported' || stage === 'removed') return 'neutral'
  return 'warn'
}

function stageLabel(stage: ExtToolsStage, t: Translate): string {
  const labels: Record<ExtToolsStage, string> = {
    queued: t('settings.extTools.stage.queued', 'Queued'),
    downloading: t('settings.extTools.stage.downloading', 'Downloading'),
    verifying: t('settings.extTools.stage.verifying', 'Verifying'),
    staging: t('settings.extTools.stage.staging', 'Unpacking'),
    checking: t('settings.extTools.stage.checking', 'Checking'),
    active: t('settings.extTools.stage.active', 'Active'),
    removing: t('settings.extTools.stage.removing', 'Removing'),
    removed: t('settings.extTools.stage.removed', 'Removed'),
    failed: t('settings.extTools.stage.failed', 'Failed'),
    unsupported: t('settings.extTools.stage.unsupported', 'No payload for this platform'),
  }
  return labels[stage] ?? stage
}

interface PendingChange {
  id: string
  enabled: boolean
  version?: string
  plan: ExtToolsPlan
}

function isEnabled(pkg: ExtToolsPackage): boolean {
  if (pkg.desired) return pkg.desired.enabled
  return Object.keys(pkg.nodes ?? {}).length > 0
}

function NodeRow({ node, status, t }: { node: string; status: ExtToolsNodeStatus; t: Translate }) {
  return (
    <div className="flex flex-wrap items-center gap-2 text-sm" data-testid={`ext-tools-node-${node}`}>
      <span className="text-[color:var(--cp-muted)]">{node}</span>
      <StatusBadge status={stageBadge(status.stage)} label={stageLabel(status.stage, t)} />
      {status.version ? <span>{status.version}</span> : null}
      {status.error ? <span className="break-all text-[color:var(--cp-danger)]">{status.error}</span> : null}
    </div>
  )
}

function OnlineNodeRow({ target, status, actions, t }: { target: OnlineTarget; status: ExtToolsOnlineStatus; actions: OnlineActions; t: Translate }) {
  const pending = requestPending(actions.requests[requestKey(target)], status)
  const action = pending || status.state === 'downloading' ? null : status.state === 'ready' ? 'again' : status.state === 'failed' ? 'retry' : 'download'
  const problem = onlineErrorText(status, t)
  const labels = {
    download: t('settings.extTools.online.download', 'Download'),
    retry: t('settings.extTools.online.retry', 'Retry'),
    again: t('settings.extTools.online.again', 'Download again'),
  }
  return (
    <div className="flex flex-col gap-1.5 text-sm" data-testid={`ext-tools-online-${target.item}-${target.node}`} data-state={pending ? 'requested' : status.state}>
      <div className="flex flex-wrap items-center gap-2">
        <span className="text-[color:var(--cp-muted)]">{target.node}</span>
        <StatusBadge status={onlineBadge(status, pending)} label={onlineStateLabel(status, pending, t)} />
        {action ? (
          <Button size="small" variant={action === 'again' ? 'text' : 'outlined'} disabled={actions.busy} onClick={() => actions.onDownload(target, status, action === 'again')} data-testid="ext-tools-online-download">
            {labels[action]}
          </Button>
        ) : null}
      </div>
      {status.state === 'downloading' ? <OnlineProgress status={status} /> : null}
      {problem ? <span className="break-words text-[color:var(--cp-danger)]" data-testid="ext-tools-online-error">{problem}</span> : null}
      {action === 'again' ? <span className="text-xs text-[color:var(--cp-muted)]">{t('settings.extTools.online.againHint', 'Downloading again fetches it anew and replaces the copy as a whole.')}</span> : null}
    </div>
  )
}

/** What the package only declares and each node downloads: where every node got to, and a download for the administrator. */
function OnlineItems({ pkg, actions, t }: { pkg: ExtToolsPackage; actions: OnlineActions; t: Translate }) {
  const items = onlineItemsOf(pkg)
  if (items.length === 0) return null
  const declared = new Map((pkg.latest?.manifest.online ?? []).map((item) => [item.id, item]))
  return (
    <div className="mt-1 flex flex-col gap-3 border-t border-[color:var(--cp-border)] pt-3" data-testid={`ext-tools-online-${pkg.id}`}>
      <div>
        <p className="text-sm font-medium">{t('settings.extTools.online.title', 'Online items')}</p>
        <p className="text-xs text-[color:var(--cp-muted)]">{t('settings.extTools.online.hint', 'Not in the package: once it is installed, each node downloads these from their official source.')}</p>
      </div>
      {items.map((item) => {
        const manifest = declared.get(item)
        const nodes = Object.entries(pkg.nodes ?? {}).flatMap(([node, status]) => (status.online?.[item] ? [{ node, status: status.online[item] }] : []))
        const version = manifest?.version ?? nodes[0]?.status.version
        const size = nodes.find((entry) => entry.status.size_bytes > 0)?.status.size_bytes ?? Math.max(0, ...Object.values(manifest?.platforms ?? {}).map((platform) => platform.size_bytes))
        return (
          <div key={item} className="flex flex-col gap-2" data-testid={`ext-tools-online-item-${item}`}>
            <div className="flex flex-wrap items-baseline gap-x-2 text-sm">
              <span className="font-medium">{item}</span>
              {version ? <span className="text-[color:var(--cp-muted)]">{version}</span> : null}
              {size ? <span className="text-[color:var(--cp-muted)]">{formatBytes(size)}</span> : null}
            </div>
            {manifest?.desc ? <p className="text-xs text-[color:var(--cp-muted)]">{manifest.desc}</p> : null}
            {manifest?.license ? (
              <details className="text-xs" data-testid="ext-tools-online-license">
                <summary className="cursor-pointer text-[color:var(--cp-muted)]">{t('settings.extTools.online.license', 'Source and terms')}</summary>
                <p className="mt-1 break-words">{manifest.license}</p>
              </details>
            ) : null}
            {nodes.map(({ node, status }) => <OnlineNodeRow key={node} target={{ pkg: pkg.id, item, node }} status={status} actions={actions} t={t} />)}
          </div>
        )
      })}
    </div>
  )
}

function PackageCard({ pkg, onChange, busy, online, t }: { pkg: ExtToolsPackage; onChange: (id: string, enabled: boolean, version?: string) => void; busy: boolean; online: OnlineActions; t: Translate }) {
  const manifest = pkg.latest?.manifest
  const enabled = isEnabled(pkg)
  const current = pkg.desired?.enabled ? pkg.desired.version : Object.values(pkg.nodes ?? {})[0]?.version
  const upgrade = enabled && manifest && current && current !== manifest.version ? manifest.version : null
  const seeded = !pkg.desired && Object.keys(pkg.nodes ?? {}).length > 0
  const skills = [...new Set(Object.values(pkg.skills ?? {}).flat())].sort()
  const sizes = Object.values(manifest?.size_bytes ?? {})
  const waiting = waitingOf(pkg)
  const waitingFor = (items: string[]) => t('settings.extTools.waiting', 'waiting for {{item}} to download', { item: items.map((item) => onlineItemName(item, t)).join(', ') })
  const waitingByItem = new Map<string, string[]>()
  for (const [command, items] of waiting) for (const item of items) waitingByItem.set(item, [...(waitingByItem.get(item) ?? []), command])
  return (
    <div className="flex flex-col gap-2 p-4" data-testid={`ext-tools-package-${pkg.id}`}>
      <div className="flex flex-wrap items-center justify-between gap-2">
        <div className="min-w-0">
          <span className="font-medium">{pkg.id}</span>
          {current ?? manifest?.version ? <span className="ml-2 text-sm text-[color:var(--cp-muted)]">{current ?? manifest?.version}</span> : null}
          {seeded ? <span className="ml-2 text-sm text-[color:var(--cp-muted)]">{t('settings.extTools.seeded', 'Came with BuckyOS')}</span> : null}
        </div>
        <div className="flex gap-2">
          {upgrade ? (
            <Button size="small" variant="outlined" disabled={busy} onClick={() => onChange(pkg.id, true, upgrade)}>
              {t('settings.extTools.upgrade', 'Upgrade to {{version}}', { version: upgrade })}
            </Button>
          ) : null}
          <Button size="small" variant={enabled ? 'text' : 'contained'} disabled={busy} onClick={() => onChange(pkg.id, !enabled)}>
            {enabled ? t('settings.extTools.disable', 'Disable') : t('settings.extTools.enable', 'Enable')}
          </Button>
        </div>
      </div>
      {manifest?.desc ? <p className="text-sm text-[color:var(--cp-muted)]">{manifest.desc}</p> : null}
      {manifest ? (
        manifest.provides.length === 0 ? null : <p className="break-words text-sm">
          <span className="text-[color:var(--cp-muted)]">{t('settings.extTools.provides', 'Commands')}: </span>
          {manifest.provides.map((p, index) => (
            <span key={p.command}>
              {index ? ', ' : ''}
              {p.command}
              {waiting.has(p.command) ? <span className="text-[color:var(--cp-warning)]" data-testid={`ext-tools-waiting-${p.command}`}> ({waitingFor(waiting.get(p.command) ?? [])})</span> : null}
            </span>
          ))}
        </p>
      ) : (
        [...waitingByItem].map(([item, commands]) => (
          <p key={item} className="break-words text-sm text-[color:var(--cp-warning)]" data-testid={`ext-tools-waiting-for-${item}`}>
            {commands.join(', ')} ({waitingFor([item])})
          </p>
        ))
      )}
      {manifest?.requires?.length ? (
        <p className="text-sm">
          <span className="text-[color:var(--cp-muted)]">{t('settings.extTools.requires', 'Requires')}: </span>
          {manifest.requires.map((r) => r.id).join(', ')}
        </p>
      ) : null}
      {sizes.length ? (
        <p className="text-sm">
          <span className="text-[color:var(--cp-muted)]">{t('settings.extTools.size', 'Size')}: </span>
          {formatBytes(Math.max(...sizes))}
        </p>
      ) : null}
      {skills.length ? (
        <p className="break-words text-sm">
          <span className="text-[color:var(--cp-muted)]">{t('settings.extTools.skills', 'Skills that use it')}: </span>
          {skills.join(', ')}
        </p>
      ) : null}
      {Object.entries(pkg.nodes ?? {}).map(([node, status]) => <NodeRow key={node} node={node} status={status} t={t} />)}
      <OnlineItems pkg={pkg} actions={online} t={t} />
    </div>
  )
}

/** The Zone's extension packages. What is ticked here is what every node installs for all its Agents, on its own time. */
export function ExtToolsPage() {
  const { t } = useI18n()
  const [status, setStatus] = useState<ExtToolsStatusResponse | null>(null)
  const [error, setError] = useState<string | null>(null)
  const [pending, setPending] = useState<PendingChange | null>(null)
  const [busy, setBusy] = useState(false)
  const [requests, setRequests] = useState<Record<string, OnlineRequest>>({})
  const [again, setAgain] = useState<{ target: OnlineTarget; before: ExtToolsOnlineStatus } | null>(null)

  const load = useCallback(async () => {
    const { data, error: failed } = await fetchExtToolsStatus()
    if (data) setStatus(data)
    setError(failed ? String(failed) : null)
  }, [])

  // A download under way, or asked for, is followed closely; the rest of the page at the usual pace.
  const following = (status?.packages ?? []).some((pkg) =>
    Object.entries(pkg.nodes ?? {}).some(([node, report]) =>
      Object.entries(report.online ?? {}).some(([item, online]) => onlineBusy(online, requests[requestKey({ pkg: pkg.id, item, node })])),
    ),
  )

  useEffect(() => {
    const first = window.setTimeout(() => void load(), 0)
    return () => window.clearTimeout(first)
  }, [load])

  useEffect(() => {
    const timer = window.setInterval(() => void load(), following ? ONLINE_POLL_MS : POLL_MS)
    return () => window.clearInterval(timer)
  }, [load, following])

  const download = async (target: OnlineTarget, before: ExtToolsOnlineStatus) => {
    setBusy(true)
    const { error: failed } = await downloadExtToolsOnline(target.pkg, target.item, target.node)
    setBusy(false)
    if (failed) {
      setError(String(failed))
      return
    }
    setRequests((current) => ({ ...current, [requestKey(target)]: onlineRequest(before) }))
    void load()
  }

  const online: OnlineActions = {
    requests,
    busy,
    onDownload: (target, before, isAgain) => {
      if (isAgain) setAgain({ target, before })
      else void download(target, before)
    },
  }

  const ask = async (id: string, enabled: boolean, version?: string) => {
    setBusy(true)
    const { data, error: failed } = await fetchExtToolsPlan(id, enabled, version)
    setBusy(false)
    if (data) setPending({ id, enabled, version, plan: data })
    else setError(String(failed))
  }

  const confirm = async () => {
    if (!pending) return
    setBusy(true)
    const { error: failed } = await setExtToolsDesired(pending.id, pending.enabled, pending.version)
    setBusy(false)
    setPending(null)
    if (failed) setError(String(failed))
    void load()
  }

  const packages = status?.packages ?? []
  return (
    <Section
      testId="ext-tools-page"
      title={t('settings.extTools.title', 'Extension tools')}
      description={t('settings.extTools.description', 'Packages of local tools every Agent in the Zone shares. Each node installs what is enabled here on its own and reports back below.')}
      actions={
        <Tooltip title={t('settings.common.refresh', 'Refresh')}>
          <IconButton size="small" onClick={() => void load()}>
            <RefreshCw size={16} />
          </IconButton>
        </Tooltip>
      }
    >
      {error ? <SectionNote tone="danger">{error}</SectionNote> : null}
      {status && packages.length === 0 ? (
        <SectionNote>{t('settings.extTools.empty', 'No package is published in this Zone yet. Publish one with `buckyos app publish`, or import a release with `buckyos ext-tools import`.')}</SectionNote>
      ) : null}
      {packages.map((pkg) => (
        <ListCard key={pkg.id}>
          <PackageCard pkg={pkg} onChange={(id, enabled, version) => void ask(id, enabled, version)} busy={busy} online={online} t={t} />
        </ListCard>
      ))}
      <Dialog open={pending != null} onClose={() => setPending(null)} data-testid="ext-tools-confirm">
        <DialogTitle>{pending?.enabled ? t('settings.extTools.confirmEnable', 'Enable {{id}}?', { id: pending.id }) : t('settings.extTools.confirmDisable', 'Disable {{id}}?', { id: pending?.id ?? '' })}</DialogTitle>
        <DialogContent>
          {pending?.plan.errors.length ? (
            <SectionNote tone="danger">{pending.plan.errors.join('; ')}</SectionNote>
          ) : pending?.enabled ? (
            <>
              <p className="mb-2 text-sm">{t('settings.extTools.confirmNodes', 'Every node of the Zone will install these for all its Agents:')}</p>
              <ul className="list-disc pl-5 text-sm">
                {pending.plan.enable.map((entry) => <li key={entry.manifest.id}>{entry.manifest.id} {entry.manifest.version}</li>)}
              </ul>
            </>
          ) : (
            <p className="text-sm">{t('settings.extTools.confirmRemove', 'Every node will remove it. Skills that rely on its commands stay installed and report what is missing.')}</p>
          )}
        </DialogContent>
        <DialogActions>
          <Button onClick={() => setPending(null)}>{t('settings.common.cancel', 'Cancel')}</Button>
          <Button variant="contained" disabled={busy || Boolean(pending?.plan.errors.length)} onClick={() => void confirm()} data-testid="ext-tools-confirm-ok">
            {t('settings.common.confirm', 'Confirm')}
          </Button>
        </DialogActions>
      </Dialog>
      <Dialog open={again != null} onClose={() => setAgain(null)} data-testid="ext-tools-online-again">
        <DialogTitle>{t('settings.extTools.online.againTitle', 'Download {{item}} again on {{node}}?', { item: again?.target.item ?? '', node: again?.target.node ?? '' })}</DialogTitle>
        <DialogContent>
          <p className="text-sm">{t('settings.extTools.online.againBody', 'The node downloads it anew from its official source and then replaces the copy it has as a whole. The copy in use keeps working until then; if the download fails, it is kept.')}</p>
        </DialogContent>
        <DialogActions>
          <Button onClick={() => setAgain(null)}>{t('settings.common.cancel', 'Cancel')}</Button>
          <Button
            variant="contained"
            disabled={busy}
            onClick={() => {
              if (again) void download(again.target, again.before)
              setAgain(null)
            }}
            data-testid="ext-tools-online-again-ok"
          >
            {t('settings.extTools.online.again', 'Download again')}
          </Button>
        </DialogActions>
      </Dialog>
    </Section>
  )
}
