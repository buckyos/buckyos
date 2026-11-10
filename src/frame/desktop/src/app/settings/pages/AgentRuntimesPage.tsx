import { Alert, Button, IconButton, Tooltip } from '@mui/material'
import { Plus, RefreshCw, X } from 'lucide-react'
import { useCallback, useEffect, useState } from 'react'
import { fetchCurrentAccount, type CurrentAccount } from '../../../api/account'
import { isPrivilegedUserType } from '../../../api/user_type'
import {
  fetchAgentList,
  fetchAgentRuntimeCatalog,
  fetchExecutorDevices,
  issueExecutorDevice,
  type AgentEntry,
  type AgentRuntimeAssignment,
  type AgentRuntimeCatalog,
  type AgentRuntimeCatalogEntry,
  type ExecutorDeviceIssued,
  type ExecutorDeviceRow,
} from '../../../api/user_mgr'
import { errorMessage } from '../../agent-setup/model'
import { useI18n } from '../../../i18n/provider'
import { ListCard, Section, SectionNote, StatusBadge } from '../components/shared/Section'
import {
  AddDeviceDialog,
  AssignRuntimeDialog,
  EditNodeDialog,
  IssuedDeviceDialog,
  RemoveDeviceDialog,
  UnassignRuntimeDialog,
} from './AgentRuntimeDialogs'

type Translate = ReturnType<typeof useI18n>['t']

const POLL_MS = 5000

function runtimeAllows(runtime: Pick<AgentRuntimeCatalogEntry, 'workdir_roots' | 'read_only' | 'computer' | 'baseline'>, t: Translate): string {
  return [
    runtime.workdir_roots?.length ? runtime.workdir_roots.join(', ') : t('settings.runtimes.noDirectories', 'No directories'),
    runtime.read_only ? t('settings.runtimes.readOnly', 'Read only') : t('settings.runtimes.readWrite', 'Read and write'),
    t('settings.runtimes.computerLevel', 'ComputerUse: {{level}}', { level: t(`settings.runtimes.computer.${runtime.computer}`, runtime.computer) }),
    runtime.baseline === 'extended' ? t('settings.runtimes.baseline.extended', 'Extended environment') : t('settings.runtimes.baseline.standard', 'Standard environment'),
  ].join(' · ')
}

function deviceExpired(runtime: AgentRuntimeCatalogEntry) {
  return runtime.expires_at_ms != null && runtime.expires_at_ms <= Date.now()
}

/** A reset URL keeps the device's term; an expired device gets another day. */
function renewedExpiry(runtime: AgentRuntimeCatalogEntry) {
  if (runtime.expires_at_ms == null) return null
  return deviceExpired(runtime) ? Date.now() + 24 * 3600_000 : runtime.expires_at_ms
}

function RuntimeRow({ runtime, device, agentName, canManage, busy, onAssign, onUnassign, onEdit, onReset, onRemove }: {
  runtime: AgentRuntimeCatalogEntry
  device?: ExecutorDeviceRow
  agentName: (agentId: string) => string
  canManage: boolean
  busy: boolean
  onAssign: () => void
  onUnassign: (assignment: AgentRuntimeAssignment) => void
  onEdit: () => void
  onReset: () => void
  onRemove: (action: 'revoke' | 'delete') => void
}) {
  const { t, locale } = useI18n()
  const isDevice = runtime.kind === 'executor'
  const expired = isDevice && deviceExpired(runtime)
  const capabilities = device?.online?.capabilities
  const facts = [
    runtime.runtime_id,
    isDevice
      ? capabilities?.platform ?? t('settings.runtimes.platformUnknown', 'Platform unavailable')
      : [runtime.os, runtime.device_type].filter(Boolean).join(' / '),
    isDevice && capabilities?.ui?.supported
      ? capabilities.ui.ready ? t('settings.runtimes.desktopReady', 'Desktop ready') : t('settings.runtimes.desktopNotReady', 'Desktop not ready — run doctor on the device')
      : '',
    isDevice && !expired && !runtime.closed
      ? runtime.expires_at_ms != null
        ? t('settings.runtimes.expiresAt', 'Expires {{date}}', { date: new Date(runtime.expires_at_ms).toLocaleString(locale) })
        : t('settings.runtimes.noExpiry', 'No expiry')
      : '',
  ].filter(Boolean)
  return (
    <ListCard testId={`settings-runtime-${runtime.runtime_id}`}>
      <div className="flex flex-wrap items-start justify-between gap-2">
        <div className="min-w-0 space-y-1">
          <div className="flex flex-wrap items-center gap-2">
            <span className="text-sm font-medium" style={{ color: 'var(--cp-text)' }}>{runtime.name || runtime.node_id || runtime.runtime_id}</span>
            {isDevice ? (
              <StatusBadge status={runtime.online ? 'pass' : 'neutral'} label={runtime.online ? t('settings.cluster.online', 'Online') : t('settings.cluster.offline', 'Offline')} />
            ) : null}
            {expired ? (
              <StatusBadge status="warn" label={t('settings.runtimes.expired', 'Expired')} />
            ) : runtime.closed ? (
              <StatusBadge status="warn" label={isDevice ? t('settings.runtimes.revoked', 'Access revoked') : t('settings.runtimes.closed', 'Closed')} />
            ) : !isDevice ? (
              <StatusBadge status="pass" label={t('settings.runtimes.open', 'Open')} />
            ) : null}
          </div>
          <p className="break-all text-xs" style={{ color: 'var(--cp-muted)' }}>{facts.join(' · ')}</p>
          <p className="break-all text-xs" style={{ color: 'var(--cp-muted)' }}>
            {t('settings.runtimes.allows', 'Allows: {{allows}}', { allows: runtimeAllows(runtime, t) })}
            {runtime.run_as ? ` · ${t('settings.runtimes.runAs', 'Runs as the OS account chosen when the node was activated')}` : ''}
          </p>
        </div>
        <div className="flex flex-wrap gap-1">
          {!runtime.closed ? <Button size="small" disabled={busy} onClick={onAssign}>{t('settings.runtimes.assign', 'Assign to Agent')}</Button> : null}
          {!isDevice && canManage ? <Button size="small" disabled={busy} onClick={onEdit}>{t('settings.runtimes.edit', 'Edit')}</Button> : null}
          {isDevice && canManage ? (
            <>
              <Button size="small" disabled={busy} onClick={onReset}>{t('settings.runtimes.resetUrl', 'Reset URL')}</Button>
              <Button size="small" disabled={busy || (runtime.closed && !expired)} onClick={() => onRemove('revoke')}>{t('settings.runtimes.revoke', 'Revoke')}</Button>
              <Button size="small" color="error" disabled={busy} onClick={() => onRemove('delete')}>{t('settings.runtimes.delete', 'Delete')}</Button>
            </>
          ) : null}
        </div>
      </div>
      <div className="mt-2 flex flex-wrap items-center gap-2 text-xs" style={{ color: 'var(--cp-muted)' }}>
        <span>{t('settings.runtimes.assignedTo', 'Assigned to')}</span>
        {runtime.assigned_to.length === 0 ? <span>{t('settings.runtimes.unassigned', 'no Agent')}</span> : null}
        {runtime.assigned_to.map((assignment) => (
          <span
            key={`${assignment.agent_id}:${assignment.runtime_id}`}
            className="inline-flex items-center gap-1.5 rounded-full border px-2 py-0.5"
            style={{ borderColor: 'var(--cp-border)', color: 'var(--cp-text)' }}
            data-testid={`settings-runtime-assignment-${assignment.agent_id}`}
          >
            {agentName(assignment.agent_id)}
            <StatusBadge status={assignment.enabled ? 'pass' : 'neutral'} label={assignment.enabled ? t('settings.runtimes.on', 'On') : t('settings.runtimes.off', 'Off')} />
            {assignment.default ? <StatusBadge status="info" label={t('settings.runtimes.default', 'Default')} /> : null}
            <Tooltip title={t('settings.runtimes.unassign', 'Unassign')}>
              <IconButton size="small" disabled={busy} aria-label={t('settings.runtimes.unassignFrom', 'Unassign from {{agent}}', { agent: agentName(assignment.agent_id) })} onClick={() => onUnassign(assignment)}>
                <X size={12} />
              </IconButton>
            </Tooltip>
          </span>
        ))}
      </div>
    </ListCard>
  )
}

type DialogState =
  | { kind: 'add' }
  | { kind: 'edit'; runtime: AgentRuntimeCatalogEntry }
  | { kind: 'assign'; runtime: AgentRuntimeCatalogEntry }
  | { kind: 'unassign'; runtime: AgentRuntimeCatalogEntry; assignment: AgentRuntimeAssignment }
  | { kind: 'remove'; runtime: AgentRuntimeCatalogEntry; action: 'revoke' | 'delete' }

/** The owner's runtimes: what each allows is its own configuration; assigning one only says which Agent uses it. */
export function AgentRuntimesPage() {
  const { t } = useI18n()
  const [catalog, setCatalog] = useState<AgentRuntimeCatalog | null>(null)
  const [devices, setDevices] = useState<ExecutorDeviceRow[]>([])
  const [agents, setAgents] = useState<AgentEntry[]>([])
  const [account, setAccount] = useState<CurrentAccount | null>(null)
  const [loadError, setLoadError] = useState<string | null>(null)
  const [error, setError] = useState<string | null>(null)
  const [busy, setBusy] = useState(false)
  const [dialog, setDialog] = useState<DialogState | null>(null)
  const [issued, setIssued] = useState<ExecutorDeviceIssued | null>(null)
  const canManage = isPrivilegedUserType(account?.user_type)

  const load = useCallback(async () => {
    const [catalogResult, agentsResult] = await Promise.all([fetchAgentRuntimeCatalog(), fetchAgentList()])
    if (catalogResult.data) setCatalog(catalogResult.data)
    setLoadError(catalogResult.error ? errorMessage(catalogResult.error) : null)
    if (agentsResult.data) setAgents(agentsResult.data.agents)
    return catalogResult.data
  }, [])

  useEffect(() => {
    void fetchCurrentAccount().catch(() => null).then(setAccount)
    const first = window.setTimeout(() => void load(), 0)
    const timer = window.setInterval(() => void load(), POLL_MS)
    return () => {
      window.clearTimeout(first)
      window.clearInterval(timer)
    }
  }, [load])

  useEffect(() => {
    if (!canManage || !catalog?.runtimes.some((runtime) => runtime.kind === 'executor')) return
    let live = true
    void fetchExecutorDevices().then(({ data }) => { if (live && data) setDevices(data.devices) })
    return () => { live = false }
  }, [canManage, catalog])

  const agentName = (agentId: string) => {
    const agent = agents.find((entry) => entry.agent_id === agentId)
    return agent ? agent.display_name || agent.name : agentId
  }
  const ownAgents = agents.filter((entry) => entry.owner_user_id === account?.user_id && entry.install.state === 'ready')

  const resetUrl = async (runtime: AgentRuntimeCatalogEntry) => {
    if (!runtime.executor_id) return
    setBusy(true)
    setError(null)
    const result = await issueExecutorDevice(runtime.executor_id, renewedExpiry(runtime))
    setBusy(false)
    if (result.data) setIssued(result.data)
    else setError(errorMessage(result.error))
    void load()
  }

  const close = (changed?: boolean) => {
    setDialog(null)
    if (changed) void load()
  }

  const nodes = catalog?.runtimes.filter((runtime) => runtime.kind === 'node') ?? []
  const deviceRuntimes = catalog?.runtimes.filter((runtime) => runtime.kind === 'executor') ?? []
  const row = (runtime: AgentRuntimeCatalogEntry) => (
    <RuntimeRow
      key={runtime.runtime_id}
      runtime={runtime}
      device={devices.find((item) => item.device.executor_id === runtime.executor_id)}
      agentName={agentName}
      canManage={canManage}
      busy={busy}
      onAssign={() => setDialog({ kind: 'assign', runtime })}
      onUnassign={(assignment) => setDialog({ kind: 'unassign', runtime, assignment })}
      onEdit={() => setDialog({ kind: 'edit', runtime })}
      onReset={() => void resetUrl(runtime)}
      onRemove={(action) => setDialog({ kind: 'remove', runtime, action })}
    />
  )

  return (
    <div className="space-y-4" data-testid="settings-agent-runtimes">
      <Section
        title={t('settings.runtimes.title', 'Agent Runtimes')}
        description={t('settings.runtimes.description', 'A runtime is a sandbox an Agent works in. What it allows — directories, read or write, ComputerUse, environment, term — is the runtime’s own configuration; assigning it only says which Agent uses it.')}
        actions={
          <Tooltip title={t('settings.common.refresh', 'Refresh')}>
            <IconButton size="small" aria-label={t('settings.common.refresh', 'Refresh')} onClick={() => void load()}>
              <RefreshCw size={16} />
            </IconButton>
          </Tooltip>
        }
      >
        {loadError ? <SectionNote tone="danger">{loadError}</SectionNote> : null}
        {error ? <Alert severity="error" onClose={() => setError(null)}>{error}</Alert> : null}
        {!catalog && !loadError ? <SectionNote>{t('settings.common.loading', 'Loading…')}</SectionNote> : null}
      </Section>

      <Section
        title={t('settings.runtimes.nodes', 'Nodes')}
        description={t('settings.runtimes.nodesHint', 'Nodes of this Zone that are open to the Zone owner’s Agents. Each Agent has its own profile on a node, so several Agents can use one at the same time.')}
        testId="settings-runtime-nodes"
      >
        {catalog && nodes.length === 0 ? (
          <SectionNote>{t('settings.runtimes.noNodes', 'No node is available. A node is open to Agents only when the Zone owner chose an OS account for them when it was activated.')}</SectionNote>
        ) : null}
        <div className="space-y-2">{nodes.map(row)}</div>
      </Section>

      <Section
        title={t('settings.runtimes.devices', 'Devices')}
        description={t('settings.runtimes.devicesHint', 'Devices joined with a daemon URL. A device is used by one Agent at a time; its configuration is fixed when it is issued — to change it, issue another device.')}
        testId="settings-runtime-devices"
        actions={canManage ? (
          <Tooltip title={t('settings.cluster.addDevice', 'Add device')}>
            <IconButton size="small" aria-label={t('settings.cluster.addDevice', 'Add device')} onClick={() => setDialog({ kind: 'add' })} data-testid="settings-add-device">
              <Plus size={16} />
            </IconButton>
          </Tooltip>
        ) : undefined}
      >
        {account && !canManage ? <SectionNote>{t('settings.runtimes.adminOnly', 'Only Zone administrators can add or revoke devices.')}</SectionNote> : null}
        {catalog && deviceRuntimes.length === 0 ? <SectionNote>{t('settings.runtimes.noDevices', 'No device has been added yet.')}</SectionNote> : null}
        <div className="space-y-2">{deviceRuntimes.map(row)}</div>
      </Section>

      <AddDeviceDialog
        open={dialog?.kind === 'add'}
        agents={ownAgents}
        onClose={() => close()}
        onIssued={(result, assignError) => {
          setIssued(result)
          setError(assignError)
          close(true)
        }}
      />
      {dialog?.kind === 'edit' ? <EditNodeDialog runtime={dialog.runtime} onClose={close} /> : null}
      {dialog?.kind === 'assign' ? <AssignRuntimeDialog runtime={dialog.runtime} agents={ownAgents} agentName={agentName} reload={load} onClose={close} /> : null}
      {dialog?.kind === 'unassign' ? <UnassignRuntimeDialog runtime={dialog.runtime} assignment={dialog.assignment} agents={agents} agentName={agentName} onClose={close} /> : null}
      {dialog?.kind === 'remove' ? <RemoveDeviceDialog runtime={dialog.runtime} action={dialog.action} agents={agents} agentName={agentName} onClose={close} /> : null}
      <IssuedDeviceDialog issued={issued} onClose={() => setIssued(null)} />
    </div>
  )
}
