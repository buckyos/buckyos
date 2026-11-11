import { useState } from 'react'
import clsx from 'clsx'
import { ChevronRight, Plus, Server } from 'lucide-react'
import { ago, t, type TextKey } from '../i18n'
import { sessionHref, useQuery } from '../lib'
import {
  dataModel,
  type ComputerAccess,
  type ExtCheck,
  type KnownRuntime,
  type RuntimeImpact,
  type RuntimeSettings,
} from '../model'
import { isExpired, isReadOnly, mayHoldWorkspaces, outerTarget, runtimeAvailability, runtimeRole, settingIdOf, type RuntimeRole } from '../runtimes'
import { Dialog, HomeShell } from '../shell'
import { ErrorBox } from '../ui'

const connectorLabel: Record<KnownRuntime['connector']['type'], TextKey> = {
  native: 'runtimeConnectorNative',
  node_executor: 'runtimeConnectorNode',
  ssh: 'runtimeConnectorSsh',
}

const computerLabel: Record<ComputerAccess, TextKey> = {
  none: 'computerNone',
  observe: 'computerObserve',
  control: 'computerControl',
}

const roleLabel: Record<RuntimeRole, TextKey> = {
  provider: 'runtimeRoleProvider',
  owner_default: 'runtimeRoleOwnerDefault',
  assigned: 'runtimeRoleAssigned',
}

function extToolsLine(ext: ExtCheck): string {
  switch (ext.status) {
    case 'ok': return t('runtimeExtOk')
    case 'partial': return t('runtimeExtPartial', { tools: ext.missing.join(', ') })
    case 'none': return t('runtimeExtNone')
    case 'system': return t('runtimeExtSystem', { tools: ext.system.join(', ') })
  }
}

function ImpactSessions({ impact }: { impact: RuntimeImpact }) {
  return (
    <div className="mb-3 text-sm" data-testid="runtime-impact-sessions">
      <div className="text-xs text-mute">{t('runtimeImpactSessions', { count: impact.running_sessions.length })}</div>
      {impact.active_sessions.filter((session) => impact.running_sessions.includes(session.session_id)).map((session) => (
        <a key={session.session_id} href={sessionHref(session.session_id)} className="block break-words text-accent">
          {session.title || session.session_id} · {session.run_state}
        </a>
      ))}
    </div>
  )
}

/** Switching a runtime on or off: one with sessions in progress is not switched off, the sessions are shown. */
function useRuntimeToggle(runtime: KnownRuntime, enabled: boolean, refresh: () => Promise<unknown>) {
  const [busy, setBusy] = useState(false)
  const [error, setError] = useState<unknown>()
  const [blocked, setBlocked] = useState<RuntimeImpact | null>(null)
  const run = async (action: () => Promise<unknown>) => {
    setBusy(true)
    setError(undefined)
    try {
      await action()
      await refresh()
    } catch (cause) {
      setError(cause)
      await refresh()
    } finally {
      setBusy(false)
    }
  }
  const toggle = () => run(async () => {
    if (!enabled) return dataModel.setRuntimeSetting({ runtime_id: runtime.runtime_id, enabled: true })
    const found = await dataModel.runtimeImpact(runtime.runtime_id)
    if (found.blocking) setBlocked(found)
    else await dataModel.setRuntimeSetting({ runtime_id: runtime.runtime_id, enabled: false })
  })
  const dialog = blocked && (
    <Dialog title={t('runtimeInUseTitle', { name: runtime.name })} busy={busy} testId="runtime-impact" onClose={() => setBlocked(null)}>
      <p className="my-3 text-sm text-mute">{t('runtimeInUseHint')}</p>
      <ImpactSessions impact={blocked} />
    </Dialog>
  )
  return { busy, error: blocked ? undefined : error, run, toggle, dialog }
}

function Switch({ on, busy, onClick, label }: { on: boolean; busy: boolean; onClick: () => void; label: string }) {
  return (
    <button role="switch" aria-checked={on} aria-label={label} disabled={busy} onClick={onClick} data-testid="runtime-toggle"
      className="flex min-h-11 w-14 shrink-0 items-center justify-end disabled:opacity-50">
      <span className={clsx('flex h-6 w-11 items-center rounded-full px-0.5 transition-colors', on ? 'justify-end bg-accent' : 'justify-start bg-line')}>
        <span className="h-5 w-5 rounded-full bg-panel" />
      </span>
    </button>
  )
}

function RuntimeRow({ runtime, settings, refresh }: { runtime: KnownRuntime; settings?: RuntimeSettings; refresh: () => Promise<unknown> }) {
  const chosen = settings?.runtimes[settingIdOf(runtime)] ?? null
  const role = runtimeRole(runtime, settings)
  const { busy, error, toggle, dialog } = useRuntimeToggle(runtime, Boolean(chosen?.enabled), refresh)
  const shared = role === 'provider' && Boolean(runtime.connector.type === 'native' && runtime.connector.aliases?.length)
  const note = isExpired(runtime) ? t('runtimeExpired') : role === 'provider' ? t('runtimeAlwaysOn') : null
  return (
    <li className="min-w-0 py-1" data-testid={`runtime-${runtime.runtime_id}`}>
      <div className="flex items-center gap-2">
        <a href={`#/runtime/${encodeURIComponent(runtime.runtime_id)}`} className="flex min-h-14 min-w-0 flex-1 items-center gap-3 rounded-xl py-1 active:bg-soft">
          <Server size={18} className="shrink-0 text-accent" />
          <span className="min-w-0 flex-1">
            <span className="block truncate text-sm font-medium">{runtime.name}</span>
            <span className="block truncate text-xs text-mute" data-testid="runtime-line">
              <span className="text-accent">{t(roleLabel[role])}</span>
              {shared && ` · ${t('runtimeSharedWithOwner')}`}
              {note && <span className={clsx(isExpired(runtime) && 'text-warn')}> · {note}</span>}
              {runtime.desc && ` · ${runtime.desc}`}
            </span>
          </span>
          <ChevronRight size={18} className="shrink-0 text-mute" />
        </a>
        {role !== 'provider' && chosen && <Switch on={chosen.enabled} busy={busy} onClick={() => void toggle()} label={t(chosen.enabled ? 'runtimeDisable' : 'runtimeEnable')} />}
      </div>
      <ErrorBox error={error} />
      {dialog}
    </li>
  )
}

function AddRuntime() {
  const [open, setOpen] = useState(false)
  const profile = useQuery(['agent.profile'], () => dataModel.profile())
  const desktop = profile.data?.desktop_url
  return (
    <>
      <button className="tap border border-line text-xs" onClick={() => setOpen(true)} data-testid="runtime-add">
        <Plus size={14} />
        {t('runtimeAdd')}
      </button>
      {open && (
        <Dialog title={t('runtimeAdd')} busy={false} onClose={() => setOpen(false)} testId="runtime-add-dialog"
          footer={<>
            <button type="button" className="tap flex-1 border border-line" onClick={() => setOpen(false)}>{t('close')}</button>
            {desktop && <a className="tap flex-1 bg-accent text-panel" href={`${desktop}/?settings=agent-runtimes`} target={outerTarget} data-testid="runtime-add-open">{t('runtimeAddOpen')}</a>}
          </>}>
          <p className="my-3 text-sm text-mute">{t('runtimeAddHint')}</p>
          {!desktop && <p className="mb-3 text-sm text-mute">{t('runtimeAddNoDesktop')}</p>}
        </Dialog>
      )}
    </>
  )
}

const ROLE_ORDER: Record<RuntimeRole, number> = { provider: 0, owner_default: 1, assigned: 2 }

export function RuntimeList() {
  const runtimes = useQuery(['runtimes.list'], () => dataModel.runtimes())
  const settings = useQuery(['runtimes.settings'], () => dataModel.runtimeSettings())
  const records = runtimes.data ?? []
  const current = records.filter((r) => r.lifecycle !== 'retired')
    .sort((a, b) => ROLE_ORDER[runtimeRole(a, settings.data)] - ROLE_ORDER[runtimeRole(b, settings.data)])
  const retired = records.filter((r) => r.lifecycle === 'retired')
  const refresh = () => Promise.all([runtimes.mutate(), settings.mutate()])
  return (
    <section className="card min-w-0 p-4" data-testid="known-runtimes">
      <div className="mb-1 flex flex-wrap items-center justify-between gap-2">
        <h2 className="text-base font-semibold">{t('runtimes')}
          {runtimes.data && <span className="ml-1.5 text-sm font-normal text-mute">{current.length}</span>}
        </h2>
        <AddRuntime />
      </div>
      <p className="mb-2 text-xs text-mute">{t('runtimeManagedHint')}</p>
      <ErrorBox error={runtimes.error ?? settings.error} />
      {current.length > 0 ? <ul className="divide-y divide-line" data-testid="runtime-current">
        {current.map((runtime) => <RuntimeRow key={runtime.runtime_id} runtime={runtime} settings={settings.data} refresh={refresh} />)}
      </ul> : !runtimes.error && <p className="py-2 text-sm text-mute">{t(runtimes.isLoading ? 'loading' : 'noRuntimes')}</p>}
      {retired.length > 0 && (
        <details className="mt-2 border-t border-line pt-1" data-testid="runtime-history">
          <summary className="flex min-h-11 cursor-pointer items-center text-sm text-mute">{t('runtimeHistory')} · {retired.length}</summary>
          <ul className="divide-y divide-line">
            {retired.map((runtime) => (
              <li key={runtime.runtime_id} data-testid={`runtime-${runtime.runtime_id}`}>
                <a href={`#/runtime/${encodeURIComponent(runtime.runtime_id)}`} className="flex min-h-14 min-w-0 items-center gap-3 py-1 opacity-70">
                  <span className="min-w-0 flex-1">
                    <span className="block truncate text-sm font-medium">{runtime.name}</span>
                    <span className="block truncate text-xs text-mute">
                      {runtime.usage.last_used_at_ms ? t('runtimeLastUsed', { time: ago(runtime.usage.last_used_at_ms) }) : t('runtimeNeverUsed')}
                      {' · '}{t('runtimeSessionsUsed', { sessions: runtime.usage.sessions_total })}
                    </span>
                  </span>
                  <ChevronRight size={18} className="shrink-0 text-mute" />
                </a>
              </li>
            ))}
          </ul>
        </details>
      )}
    </section>
  )
}

/** What the node installed for this runtime. Read-only: packages are chosen for the whole Zone, not per Agent. */
function RuntimeExtPackages({ runtimeId }: { runtimeId: string }) {
  const report = useQuery(['runtimes.ext_tools', runtimeId], () => dataModel.runtimeExtTools(runtimeId), false)
  const packages = Object.entries(report.data?.packages ?? {})
  if (packages.length === 0) return null
  const names = packages.map(([id, install]) => `${id} ${install.version}`).join(', ')
  return (
    <p className="mt-1 break-words text-xs text-mute" data-testid="runtime-ext-packages">
      {t('runtimeExtPackages', { packages: names })} · {t('runtimeExtManagedByNode')}
    </p>
  )
}

function RuntimeDetail({ runtime, settings, refresh }: { runtime: KnownRuntime; settings?: RuntimeSettings; refresh: () => Promise<unknown> }) {
  const chosen = settings?.runtimes[settingIdOf(runtime)] ?? null
  const role = runtimeRole(runtime, settings)
  const { busy, error, run, toggle, dialog } = useRuntimeToggle(runtime, Boolean(chosen?.enabled), refresh)
  const [removal, setRemoval] = useState<RuntimeImpact | null>(null)
  const [removeError, setRemoveError] = useState<unknown>()
  const [removing, setRemoving] = useState(false)
  const retired = runtime.lifecycle === 'retired'
  const { caps, grant, usage } = runtime
  const check = caps.baseline_check
  const devHelpers = Object.entries(check?.resolved ?? {}).filter(([, command]) => command.provided_by === 'dev').map(([name]) => name)
  const extMissing = check?.ext?.missing ?? []
  const standardMissing = check?.missing.filter((tool) => !extMissing.includes(tool)) ?? []
  const toolProblem = check?.error
    ? t('runtimeToolCheckFailed', { error: check.error })
    : [
        standardMissing.length > 0 && t('runtimeToolsMissing', { tools: standardMissing.join(', ') }),
        check?.unmanaged?.length && t('runtimeToolsUnmanaged', { tools: check.unmanaged.join(', ') }),
      ].filter(Boolean).join(' · ')
  const askRemove = () => run(async () => {
    setRemoveError(undefined)
    setRemoval(await dataModel.runtimeImpact(runtime.runtime_id))
  })
  const remove = async () => {
    setRemoving(true)
    setRemoveError(undefined)
    try {
      await dataModel.removeRuntime(runtime.runtime_id)
      await refresh()
      window.location.hash = '#/'
    } catch (cause) {
      setRemoveError(cause)
      setRemoving(false)
    }
  }
  return (
    <>
      <section className="card min-w-0 p-4" data-testid={`runtime-${runtime.runtime_id}`}>
        <div className="flex items-start gap-2">
          <Server size={18} className="mt-0.5 shrink-0 text-accent" />
          <div className="min-w-0 flex-1">
            <h2 className="break-words text-sm font-semibold">{runtime.name}
              <span className="ml-1.5 font-mono text-xs font-normal text-mute">{runtime.runtime_id}</span>
            </h2>
            {runtime.desc && <p className="mt-1 break-words text-sm text-mute">{runtime.desc}</p>}
          </div>
        </div>
        <div className="my-2 flex flex-wrap gap-x-3 gap-y-1 text-xs">
          {role !== 'owner_default' && <span className="text-accent" data-testid="runtime-role">{t(roleLabel[role])}</span>}
          <span>{t(connectorLabel[runtime.connector.type])}</span>
          <span className={clsx(runtime.availability === 'available' ? 'text-ok' : 'text-warn')}>
            {t(retired ? 'runtimeRetired' : runtimeAvailability[runtime.availability])}
          </span>
          {(runtime.default || role === 'owner_default') && <span className="text-accent" data-testid="runtime-default">{t(runtime.default ? 'runtimeDefault' : 'runtimeDefaultOff')}</span>}
          {runtime.builtin && <span>{t('runtimeBuiltin')}</span>}
          <span data-testid="runtime-computer">{t('runtimeComputer')}: {t(computerLabel[grant.computer ?? 'none'])}</span>
          <span data-testid="runtime-baseline">{t(caps.baseline === 'extended' ? 'runtimeBaselineExtended' : 'runtimeBaselineStandard')}</span>
          {caps.os && <span className="font-mono">{caps.os}{caps.shell ? ` · ${caps.shell}` : ''}{caps.identity_mode ? ` · ${caps.identity_mode}` : ''}</span>}
          {caps.desktop && <span>{t('runtimeDesktop')}</span>}
          {!mayHoldWorkspaces(runtime) && <span>{t('runtimeNoWorkspaces')}</span>}
          {isReadOnly(runtime) && <span>{t('runtimeReadOnly')}</span>}
          {grant.expires_at_ms != null && <span className="text-warn">{isExpired(runtime) ? t('runtimeExpired') : t('runtimeExpires', { time: new Date(grant.expires_at_ms).toLocaleString() })}</span>}
        </div>
        {grant.roots.length > 0 && <dl className="grid min-w-0 grid-cols-[auto_minmax(0,1fr)] gap-x-3 gap-y-1 text-xs">
          <dt className="text-mute">{t('runtimeRoots')}</dt>
          <dd className="break-all font-mono">{grant.roots.join(' · ')}</dd>
        </dl>}
        {(usage.sessions_total > 0 || usage.workspaces > 0) &&
          <p className="mt-1 text-xs text-mute" data-testid="runtime-usage">
            {t('runtimeUsage', { sessions: String(usage.sessions_total), workspaces: String(usage.workspaces) })}
            {usage.last_session_id && <> · <a className="font-mono text-accent" href={sessionHref(usage.last_session_id)}>{usage.last_session_id}</a></>}
          </p>}
        {runtime.last_error && runtime.availability !== 'available' &&
          <p className="mt-2 break-words text-xs text-warn">{runtime.last_error}</p>}
        {caps.desktop_error && <p className="mt-2 break-words text-xs text-warn" data-testid="runtime-desktop-error">{caps.desktop_error}</p>}
        {toolProblem &&
          <p className="mt-2 break-words text-xs text-warn" data-testid="runtime-tool-check">{toolProblem}</p>}
        {check?.ext &&
          <p className={clsx('mt-1 break-words text-xs', check.missing.some((tool) => extMissing.includes(tool)) ? 'text-warn' : 'text-mute')} data-testid="runtime-ext">{extToolsLine(check.ext)}</p>}
        {!retired && runtime.availability === 'available' && <RuntimeExtPackages runtimeId={runtime.runtime_id} />}
        {devHelpers.length > 0 &&
          <p className="mt-1 break-words text-xs text-mute" data-testid="runtime-dev-helpers">{t('runtimeDevHelpers', { tools: devHelpers.join(', ') })}</p>}
        {caps.tools_check && caps.tools_check.present.length > 0 &&
          <p className="mt-1 break-words text-xs text-mute">{t('runtimeCommandsVerified', { tools: caps.tools_check.present.join(', ') })}</p>}
        <p className="mt-2 text-xs text-mute" data-testid="runtime-config-hint">{t('runtimeConfigHint')}</p>
      </section>
      {!retired && (
        <section className="card min-w-0 p-4" data-testid="runtime-actions">
          <ErrorBox error={error} />
          <div className="flex flex-wrap items-center gap-2">
            <button className="tap border border-line text-xs" disabled={busy} onClick={() => void run(() => dataModel.checkRuntime(runtime.runtime_id))}>{t('runtimeCheck')}</button>
            {chosen && role !== 'provider' && <button className="tap border border-line text-xs" disabled={busy} data-testid="runtime-toggle"
              onClick={() => void toggle()}>
              {t(chosen.enabled ? 'runtimeDisable' : 'runtimeEnable')}
            </button>}
            {!runtime.default && (role === 'provider' || chosen?.enabled) && <button className="tap border border-line text-xs" disabled={busy} data-testid="runtime-make-default"
              onClick={() => void run(() => dataModel.setRuntimeSetting({ runtime_id: runtime.runtime_id, default: true }))}>
              {t('runtimeMakeDefault')}
            </button>}
            {chosen && role !== 'provider' && <button className="tap border border-bad text-xs text-bad" disabled={busy} data-testid="runtime-remove"
              onClick={() => void askRemove()}>
              {t('runtimeRemove')}
            </button>}
          </div>
        </section>
      )}
      {dialog}
      {removal && (removal.blocking ? (
        <Dialog title={t('runtimeInUseTitle', { name: runtime.name })} busy={false} testId="runtime-impact" onClose={() => setRemoval(null)}>
          <p className="my-3 text-sm text-mute">{t('runtimeRemoveInUseHint')}</p>
          <ImpactSessions impact={removal} />
        </Dialog>
      ) : (
        <Dialog title={t('runtimeRemoveTitle', { name: runtime.name })} busy={removing} testId="runtime-remove-dialog" danger
          onSubmit={() => void remove()} onClose={() => setRemoval(null)} label={t('runtimeRemove')}>
          <p className="my-3 text-sm text-mute">{t('runtimeRemoveHint')}</p>
          <ul className="mb-3 space-y-1 text-xs" data-testid="runtime-remove-impact">
            {removal.workspaces.length > 0 && <li>{t('runtimeImpactWorkspaces', { count: removal.workspaces.length })}: {removal.workspaces.map((w) => w.name).join(', ')}</li>}
            {removal.pending_todos.length > 0 && <li>{t('runtimeImpactTodos', { count: removal.pending_todos.length })}: {removal.pending_todos.map((todo) => todo.title).join(', ')}</li>}
            <li>
              {t('runtimeSessionsUsed', { sessions: removal.usage.sessions_total })}
              {removal.usage.last_session_id && <> · <a className="font-mono text-accent" href={sessionHref(removal.usage.last_session_id)}>{removal.usage.last_session_id}</a></>}
            </li>
          </ul>
          <ErrorBox error={removeError} />
        </Dialog>
      ))}
    </>
  )
}

export default function RuntimeDetailPage({ runtimeId }: { runtimeId: string }) {
  const runtimes = useQuery(['runtimes.list'], () => dataModel.runtimes())
  const settings = useQuery(['runtimes.settings'], () => dataModel.runtimeSettings())
  const runtime = runtimes.data?.find((r) => r.runtime_id === runtimeId)
  return (
    <HomeShell title={runtime?.name ?? runtimeId} testId="runtime-page">
      <ErrorBox error={runtimes.error ?? settings.error} />
      {runtime
        ? <RuntimeDetail runtime={runtime} settings={settings.data} refresh={() => Promise.all([runtimes.mutate(), settings.mutate()])} />
        : !runtimes.error && <p className="py-2 text-sm text-mute">{t(runtimes.data ? 'runtimeNotFound' : 'loading')}</p>}
    </HomeShell>
  )
}
