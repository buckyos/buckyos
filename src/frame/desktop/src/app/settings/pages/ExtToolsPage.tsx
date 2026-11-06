import { Button, Dialog, DialogActions, DialogContent, DialogTitle, IconButton, Tooltip } from '@mui/material'
import { RefreshCw } from 'lucide-react'
import { useCallback, useEffect, useState } from 'react'
import { useI18n } from '../../../i18n/provider'
import {
  fetchExtToolsPlan,
  fetchExtToolsStatus,
  setExtToolsDesired,
  type ExtToolsNodeStatus,
  type ExtToolsPackage,
  type ExtToolsPlan,
  type ExtToolsStage,
  type ExtToolsStatusResponse,
} from '../../../api/settings'
import { ListCard, Section, SectionNote, StatusBadge } from '../components/shared/Section'
import { formatBytes } from '../components/shared/format'

type Translate = ReturnType<typeof useI18n>['t']

const POLL_MS = 5000

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

function PackageCard({ pkg, onChange, busy, t }: { pkg: ExtToolsPackage; onChange: (id: string, enabled: boolean, version?: string) => void; busy: boolean; t: Translate }) {
  const manifest = pkg.latest?.manifest
  const enabled = isEnabled(pkg)
  const current = pkg.desired?.enabled ? pkg.desired.version : Object.values(pkg.nodes ?? {})[0]?.version
  const upgrade = enabled && manifest && current && current !== manifest.version ? manifest.version : null
  const seeded = !pkg.desired && !manifest
  const skills = [...new Set(Object.values(pkg.skills ?? {}).flat())].sort()
  const sizes = Object.values(manifest?.size_bytes ?? {})
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
        <p className="break-words text-sm">
          <span className="text-[color:var(--cp-muted)]">{t('settings.extTools.provides', 'Commands')}: </span>
          {manifest.provides.map((p) => p.command).join(', ')}
        </p>
      ) : null}
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

  const load = useCallback(async () => {
    const { data, error: failed } = await fetchExtToolsStatus()
    if (data) setStatus(data)
    setError(failed ? String(failed) : null)
  }, [])

  useEffect(() => {
    const first = window.setTimeout(() => void load(), 0)
    const timer = window.setInterval(() => void load(), POLL_MS)
    return () => {
      window.clearTimeout(first)
      window.clearInterval(timer)
    }
  }, [load])

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
          <PackageCard pkg={pkg} onChange={(id, enabled, version) => void ask(id, enabled, version)} busy={busy} t={t} />
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
    </Section>
  )
}
