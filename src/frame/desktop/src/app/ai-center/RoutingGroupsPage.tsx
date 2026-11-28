import { useMemo, useState } from 'react'
import { ArrowLeft, Layers3, Link2, Pencil, Plus, Trash2, X } from 'lucide-react'
import { useI18n } from '../../i18n/provider'
import { useViewerIsAdmin } from '../settings/hooks/use-viewer-is-admin'
import { isRevisionConflict } from '../../api/aicc_routing'
import { useRoutingApi, useRoutingApps, useRoutingGroups } from './hooks/use-routing-api'
import type { RoutingApp, RoutingBinding, RoutingGroup } from './datamodel/usecases'
import { UsecaseTableView } from './components/usecases/UsecaseTableView'
import { Modal } from './components/policy/PolicyWidgets'
import { buttonStyle, errorText, fieldStyle, panelStyle } from './components/policy/policy-labels'
import { ConfirmDialog } from './components/shared/ConfirmDialog'
import { EmptyState } from './components/shared/EmptyState'

const muted = { color: 'var(--cp-muted)' }

function useConflictText() {
  const { t } = useI18n()
  return (reason: unknown) => isRevisionConflict(reason)
    ? t('aiCenter.groups.conflict', 'The group was changed elsewhere and has been reloaded. Try again.')
    : errorText(reason)
}

function NameDialog({ title, initial, onSubmit, onClose }: { title: string; initial: string; onSubmit: (name: string) => Promise<void>; onClose: () => void }) {
  const { t } = useI18n()
  const [name, setName] = useState(initial)
  const [busy, setBusy] = useState(false)
  const [error, setError] = useState<string | null>(null)
  const conflictText = useConflictText()
  const valid = name.trim().length > 0 && [...name.trim()].length <= 64
  const submit = async () => {
    setBusy(true)
    setError(null)
    try {
      await onSubmit(name.trim())
    } catch (reason) {
      setError(conflictText(reason))
      setBusy(false)
    }
  }
  return (
    <Modal title={title} onClose={onClose} busy={busy} testId="routing-group-name-dialog">
      <form className="flex flex-col gap-3" onSubmit={(event) => { event.preventDefault(); if (valid && !busy) void submit() }}>
        <label className="flex flex-col gap-1 text-sm">
          <span style={{ color: 'var(--cp-text)' }}>{t('aiCenter.groups.name', 'Name')}</span>
          <input autoFocus data-testid="routing-group-name" value={name} maxLength={64} onChange={(event) => setName(event.target.value)} placeholder={t('aiCenter.groups.namePlaceholder', 'For example: Writing')} className="min-h-11 rounded-lg px-3" style={fieldStyle} />
        </label>
        {error && <p role="alert" className="break-words text-xs" style={{ color: 'var(--cp-danger)' }}>{error}</p>}
        <div className="flex flex-col-reverse gap-2 sm:flex-row sm:justify-end">
          <button type="button" onClick={onClose} disabled={busy} className="min-h-11 rounded-lg px-4 text-sm" style={buttonStyle.secondary}>{t('common.cancel', 'Cancel')}</button>
          <button type="submit" data-testid="routing-group-name-save" disabled={!valid || busy} className="min-h-11 rounded-lg px-4 text-sm font-medium disabled:opacity-50" style={buttonStyle.primary}>{t('common.save', 'Save')}</button>
        </div>
      </form>
    </Modal>
  )
}

function BindDialog({ group, apps, bindings, groupNames, onBind, onClose }: {
  group: RoutingGroup
  apps: RoutingApp[]
  bindings: RoutingBinding[]
  groupNames: Record<string, string>
  onBind: (appId: string) => Promise<void>
  onClose: () => void
}) {
  const { t } = useI18n()
  const choices = apps.filter((app) => !group.app_ids?.includes(app.app_id))
  const [appId, setAppId] = useState(choices[0]?.app_id ?? '')
  const [busy, setBusy] = useState(false)
  const [error, setError] = useState<string | null>(null)
  const current = bindings.find((binding) => binding.app_id === appId && binding.group_id !== group.group_id)
  const bind = async () => {
    setBusy(true)
    setError(null)
    try {
      await onBind(appId)
    } catch (reason) {
      setError(errorText(reason))
      setBusy(false)
    }
  }
  const options = (agent: boolean) => choices.filter((app) => app.agent === agent).map((app) => <option key={app.app_id} value={app.app_id}>{app.label === app.app_id ? app.label : `${app.label} (${app.app_id})`}</option>)
  return (
    <Modal title={t('aiCenter.groups.bindTitle', 'Use {{name}} for an App', { name: group.name })} onClose={onClose} busy={busy} testId="routing-group-bind-dialog">
      <div className="flex flex-col gap-3">
        {choices.length === 0 ? (
          <p className="text-sm" style={muted}>{t('aiCenter.groups.noApps', 'No other App or Agent to choose.')}</p>
        ) : (
          <label className="flex flex-col gap-1 text-sm">
            <span style={{ color: 'var(--cp-text)' }}>{t('aiCenter.groups.app', 'App or Agent')}</span>
            <select data-testid="routing-group-bind-app" value={appId} onChange={(event) => setAppId(event.target.value)} className="min-h-11 rounded-lg px-3" style={fieldStyle}>
              {choices.some((app) => !app.agent) && <optgroup label={t('aiCenter.groups.apps', 'Apps')}>{options(false)}</optgroup>}
              {choices.some((app) => app.agent) && <optgroup label={t('aiCenter.groups.agents', 'Agents')}>{options(true)}</optgroup>}
            </select>
          </label>
        )}
        <p className="text-xs leading-5" style={muted}>
          {current
            ? t('aiCenter.groups.bindMoves', 'It uses {{group}} now; it will use this group instead.', { group: groupNames[current.group_id] ?? current.group_id })
            : t('aiCenter.groups.bindHint', 'Calls you make through this App use this group on top of your own table. An App uses one group at a time.')}
        </p>
        {error && <p role="alert" className="break-words text-xs" style={{ color: 'var(--cp-danger)' }}>{error}</p>}
        <div className="flex flex-col-reverse gap-2 sm:flex-row sm:justify-end">
          <button type="button" onClick={onClose} disabled={busy} className="min-h-11 rounded-lg px-4 text-sm" style={buttonStyle.secondary}>{t('common.cancel', 'Cancel')}</button>
          <button type="button" data-testid="routing-group-bind-confirm" disabled={!appId || busy} onClick={() => void bind()} className="min-h-11 rounded-lg px-4 text-sm font-medium disabled:opacity-50" style={buttonStyle.primary}>{t('aiCenter.groups.bind', 'Use for this App')}</button>
        </div>
      </div>
    </Modal>
  )
}

function GroupDetail({ group, apps, bindings, groupNames, isAdmin, onBack, onChanged }: {
  group: RoutingGroup
  apps: RoutingApp[]
  bindings: RoutingBinding[]
  groupNames: Record<string, string>
  isAdmin: boolean
  onBack: () => void
  onChanged: () => Promise<unknown>
}) {
  const { t } = useI18n()
  const api = useRoutingApi()
  const conflictText = useConflictText()
  const [dialog, setDialog] = useState<'rename' | 'bind' | 'delete' | null>(null)
  const [deleting, setDeleting] = useState(false)
  const [error, setError] = useState<string | null>(null)
  const appLabel = (appId: string) => apps.find((app) => app.app_id === appId)?.label ?? appId
  const unbind = async (appId: string) => {
    setError(null)
    try {
      await api.unbind(appId)
    } catch (reason) {
      setError(errorText(reason))
    }
    await onChanged()
  }
  const remove = async () => {
    setDeleting(true)
    setError(null)
    try {
      await api.deleteGroup(group.group_id, group.version)
      await onChanged()
      onBack()
    } catch (reason) {
      setError(conflictText(reason))
      setDeleting(false)
      await onChanged()
    }
  }
  return (
    <div className="flex min-w-0 flex-col gap-4" data-testid="routing-group-detail" data-group={group.group_id}>
      <div className="flex flex-wrap items-center gap-2">
        <button type="button" onClick={onBack} className="inline-flex min-h-10 items-center gap-1 text-sm" style={{ color: 'var(--cp-accent)' }}>
          <ArrowLeft size={16} />{t('aiCenter.groups.all', 'All groups')}
        </button>
      </div>
      <header className="flex flex-wrap items-center justify-between gap-3">
        <h2 className="min-w-0 break-words text-lg font-semibold" style={{ color: 'var(--cp-text)' }}>{group.name}</h2>
        <div className="flex gap-2">
          <button type="button" onClick={() => setDialog('rename')} className="inline-flex min-h-10 items-center gap-1.5 rounded-lg px-3 text-sm" style={buttonStyle.secondary}><Pencil size={14} />{t('aiCenter.groups.rename', 'Rename')}</button>
          <button type="button" data-testid="routing-group-delete" onClick={() => setDialog('delete')} className="inline-flex min-h-10 items-center gap-1.5 rounded-lg px-3 text-sm" style={buttonStyle.danger}><Trash2 size={14} />{t('aiCenter.groups.delete', 'Delete')}</button>
        </div>
      </header>
      {error && <p role="alert" className="break-words text-sm" style={{ color: 'var(--cp-danger)' }}>{error}</p>}
      <section className="flex flex-col gap-2 rounded-xl p-4" style={panelStyle}>
        <div className="flex flex-wrap items-center justify-between gap-2">
          <h3 className="text-sm font-semibold" style={{ color: 'var(--cp-text)' }}>{t('aiCenter.groups.appsTitle', 'Used by')}</h3>
          <button type="button" data-testid="routing-group-bind" onClick={() => setDialog('bind')} className="inline-flex min-h-10 items-center gap-1.5 rounded-lg px-3 text-sm" style={buttonStyle.secondary}><Link2 size={14} />{t('aiCenter.groups.bindOpen', 'Use for an App')}</button>
        </div>
        {(group.app_ids?.length ?? 0) === 0
          ? <p className="text-xs" style={muted}>{t('aiCenter.groups.noBindings', 'No App uses this group yet. Sessions can also use it when they ask for it.')}</p>
          : (
            <div className="flex flex-wrap gap-2">
              {group.app_ids!.map((appId) => (
                <span key={appId} data-testid="routing-group-app" data-app={appId} className="inline-flex min-h-9 items-center gap-1 rounded-lg pl-3 pr-1 text-sm" style={{ background: 'var(--cp-surface-2)', color: 'var(--cp-text)' }}>
                  {appLabel(appId)}
                  <button type="button" onClick={() => void unbind(appId)} aria-label={t('aiCenter.groups.unbind', 'Stop using it for {{app}}', { app: appLabel(appId) })} className="flex h-8 w-8 items-center justify-center rounded-md" style={muted}><X size={14} /></button>
                </span>
              ))}
            </div>
          )}
      </section>
      <section className="flex flex-col gap-2">
        <h3 className="text-sm font-semibold" style={{ color: 'var(--cp-text)' }}>{t('aiCenter.groups.usecasesTitle', 'Usecases in this group')}</h3>
        <p className="text-xs leading-5" style={muted}>{t('aiCenter.groups.usecasesHint', 'The table as it is while the group is in effect. Rows set by the group show it as their source; the others follow your own table.')}</p>
        <UsecaseTableView view={`group:${group.group_id}`} groupNames={groupNames} isAdmin={isAdmin} onChanged={() => void onChanged()} />
      </section>
      {dialog === 'rename' && (
        <NameDialog
          title={t('aiCenter.groups.renameTitle', 'Rename the group')}
          initial={group.name}
          onClose={() => setDialog(null)}
          onSubmit={async (name) => {
            try {
              await api.updateGroup({ group_id: group.group_id, version: group.version, name })
            } finally {
              await onChanged()
            }
            setDialog(null)
          }}
        />
      )}
      {dialog === 'bind' && (
        <BindDialog
          group={group}
          apps={apps}
          bindings={bindings}
          groupNames={groupNames}
          onClose={() => setDialog(null)}
          onBind={async (appId) => {
            await api.bind(appId, group.group_id)
            await onChanged()
            setDialog(null)
          }}
        />
      )}
      <ConfirmDialog
        open={dialog === 'delete'}
        title={t('aiCenter.groups.deleteTitle', 'Delete {{name}}?', { name: group.name })}
        message={t('aiCenter.groups.deleteMessage', 'The Apps that use it go back to your own usecase table.')}
        confirmLabel={t('aiCenter.groups.delete', 'Delete')}
        confirming={deleting}
        onConfirm={() => void remove()}
        onCancel={() => { if (!deleting) setDialog(null) }}
      />
    </div>
  )
}

/** Routing groups: sets of usecase choices the viewer applies to Apps and sessions; they only affect their owner. */
export function RoutingGroupsPage() {
  const { t } = useI18n()
  const api = useRoutingApi()
  const isAdmin = useViewerIsAdmin() === true
  const { data, error, isLoading, mutate } = useRoutingGroups()
  const { data: apps } = useRoutingApps()
  const [selected, setSelected] = useState<string | null>(null)
  const [creating, setCreating] = useState(false)
  const groups = useMemo(() => data?.groups ?? [], [data])
  const groupNames = useMemo(() => Object.fromEntries(groups.map((group) => [group.group_id, group.name])), [groups])
  const appList = useMemo(() => {
    const list = [...(apps ?? [])]
    for (const binding of data?.bindings ?? []) if (!list.some((app) => app.app_id === binding.app_id)) list.push({ app_id: binding.app_id, label: binding.app_id, agent: false })
    return list
  }, [apps, data])
  const group = groups.find((entry) => entry.group_id === selected)
  const appLabel = (appId: string) => appList.find((app) => app.app_id === appId)?.label ?? appId

  if (group) {
    return <GroupDetail group={group} apps={appList} bindings={data?.bindings ?? []} groupNames={groupNames} isAdmin={isAdmin} onBack={() => setSelected(null)} onChanged={() => mutate()} />
  }

  return (
    <div className="flex min-w-0 flex-col gap-4" style={{ color: 'var(--cp-text)' }}>
      <header className="flex flex-wrap items-start justify-between gap-3">
        <div className="min-w-0">
          <h2 className="text-lg font-semibold">{t('aiCenter.groups.title', 'Routing groups')}</h2>
          <p className="mt-1 max-w-3xl text-sm leading-6" style={muted}>
            {t('aiCenter.groups.subtitle', 'A routing group is a set of usecase choices that applies only where you use it: in an App or Agent, or in a session that asks for it. Groups only affect your own calls.')}
          </p>
        </div>
        {groups.length > 0 && (
          <button type="button" data-testid="routing-group-new" onClick={() => setCreating(true)} className="inline-flex min-h-11 items-center gap-1.5 rounded-lg px-4 text-sm font-medium" style={buttonStyle.primary}>
            <Plus size={16} />{t('aiCenter.groups.new', 'New group')}
          </button>
        )}
      </header>
      {error && !data && (
        <div role="alert" className="flex flex-wrap items-center justify-between gap-2 rounded-xl p-3 text-sm" style={{ ...panelStyle, color: 'var(--cp-danger)' }}>
          <span className="min-w-0 break-words">{errorText(error)}</span>
          <button type="button" className="min-h-10 px-3 underline" onClick={() => void mutate()}>{t('common.retry', 'Retry')}</button>
        </div>
      )}
      {isLoading && !data && <p role="status" className="py-8 text-center text-sm" style={muted}>{t('common.loading', 'Loading...')}</p>}
      {data && groups.length === 0 && (
        <EmptyState
          icon={<Layers3 size={40} />}
          title={t('aiCenter.groups.emptyTitle', 'No routing groups yet')}
          description={t('aiCenter.groups.emptyHint', 'Create one to give an App or Agent its own models, for example a stronger model for a coding Agent.')}
          action={{ label: t('aiCenter.groups.new', 'New group'), onClick: () => setCreating(true) }}
        />
      )}
      {groups.length > 0 && (
        <div className="grid grid-cols-1 gap-2 md:grid-cols-2">
          {groups.map((entry) => {
            const count = Object.keys(entry.usecases ?? {}).length
            return (
              <button
                key={entry.group_id}
                type="button"
                data-testid="routing-group"
                data-group={entry.group_id}
                onClick={() => setSelected(entry.group_id)}
                className="flex min-w-0 flex-col gap-1.5 rounded-xl p-4 text-left"
                style={panelStyle}
              >
                <span className="break-words text-sm font-medium" style={{ color: 'var(--cp-text)' }}>{entry.name}</span>
                <span className="text-xs" style={muted}>
                  {t('aiCenter.groups.summary', 'Usecases set: {{count}} · Apps: {{apps}}', { count, apps: entry.app_ids?.length ?? 0 })}
                </span>
                {(entry.app_ids?.length ?? 0) > 0 && <span className="truncate text-xs" style={muted}>{entry.app_ids!.map(appLabel).join(', ')}</span>}
              </button>
            )
          })}
        </div>
      )}
      {creating && (
        <NameDialog
          title={t('aiCenter.groups.newTitle', 'New routing group')}
          initial=""
          onClose={() => setCreating(false)}
          onSubmit={async (name) => {
            const created = await api.createGroup(name)
            await mutate()
            setCreating(false)
            setSelected(created.group_id)
          }}
        />
      )}
    </div>
  )
}
