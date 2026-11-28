import { useMemo, useState } from 'react'
import { useI18n } from '../../i18n/provider'
import { useViewerIsAdmin } from '../settings/hooks/use-viewer-is-admin'
import { useRoutingApps, useRoutingGroups } from './hooks/use-routing-api'
import { UsecaseTableView } from './components/usecases/UsecaseTableView'
import { fieldStyle } from './components/policy/policy-labels'
import type { AICenterPage } from './components/layout/nav'

const muted = { color: 'var(--cp-muted)' }

/** Which model each usecase uses, where that choice comes from and what happens when its Provider is unavailable. */
export function UsecasesPage({ navigate }: { navigate: (page: AICenterPage) => void }) {
  const { t } = useI18n()
  const isAdmin = useViewerIsAdmin() === true
  const { data: routing } = useRoutingGroups()
  const { data: apps } = useRoutingApps()
  const [view, setView] = useState('self')
  const groups = useMemo(() => routing?.groups ?? [], [routing])
  const groupNames = useMemo(() => Object.fromEntries(groups.map((group) => [group.group_id, group.name])), [groups])
  const appOptions = useMemo(() => {
    const known = new Map((apps ?? []).map((app) => [app.app_id, app.label]))
    for (const binding of routing?.bindings ?? []) if (!known.has(binding.app_id)) known.set(binding.app_id, binding.app_id)
    return [...known.entries()].map(([id, label]) => ({ id, label }))
  }, [apps, routing])
  const boundGroup = view.startsWith('app:') ? routing?.bindings.find((binding) => binding.app_id === view.slice('app:'.length))?.group_id : undefined

  const viewHint = view === 'zone'
    ? t('aiCenter.usecases.viewHint.zone', 'The Zone default for every user. Locked cells cannot be changed by users or their groups.')
    : view === 'self'
      ? t('aiCenter.usecases.viewHint.self', 'What applies to your own calls: the Zone default, then your own changes.')
      : view.startsWith('group:')
        ? t('aiCenter.usecases.viewHint.group', 'What applies while this group is in effect (for its Apps and sessions). Changes are saved in the group.')
        : boundGroup
          ? t('aiCenter.usecases.viewHint.appBound', 'What calls from this App see: your table with the group {{group}} on top.', { group: groupNames[boundGroup] ?? boundGroup })
          : t('aiCenter.usecases.viewHint.appUnbound', 'What calls from this App see. No routing group is bound to it, so your own table applies.')

  return (
    <div className="flex min-w-0 flex-col gap-4" style={{ color: 'var(--cp-text)' }}>
      <header className="flex flex-col gap-1">
        <h2 className="text-lg font-semibold">{t('aiCenter.usecases.title', 'Usecases')}</h2>
        <p className="max-w-3xl text-sm leading-6" style={muted}>
          {t('aiCenter.usecases.subtitle', 'Each usecase uses one model. See where that choice comes from and what happens when its Provider is unavailable; tap a row to change it.')}
        </p>
      </header>
      <section className="flex flex-col gap-2 sm:flex-row sm:items-center">
        <label className="flex min-w-0 flex-col gap-1 text-xs sm:w-80" style={muted}>
          {t('aiCenter.usecases.viewLabel', 'View')}
          <select
            data-testid="usecase-view-select"
            value={view}
            onChange={(event) => setView(event.target.value)}
            className="min-h-11 rounded-lg px-3 text-sm"
            style={fieldStyle}
          >
            <option value="self">{t('aiCenter.usecases.view.self', 'Me')}</option>
            {isAdmin && <option value="zone">{t('aiCenter.usecases.view.zone', 'Zone default')}</option>}
            {groups.length > 0 && (
              <optgroup label={t('aiCenter.usecases.view.groups', 'Routing groups')}>
                {groups.map((group) => <option key={group.group_id} value={`group:${group.group_id}`}>{group.name}</option>)}
              </optgroup>
            )}
            {appOptions.length > 0 && (
              <optgroup label={t('aiCenter.usecases.view.apps', 'Apps and Agents')}>
                {appOptions.map((app) => <option key={app.id} value={`app:${app.id}`}>{app.label}</option>)}
              </optgroup>
            )}
          </select>
        </label>
        <p className="min-w-0 flex-1 text-xs leading-5 sm:pt-5" data-testid="usecase-view-hint" style={muted}>
          {viewHint}
          {view.startsWith('app:') && (
            <button type="button" onClick={() => navigate('routing-groups')} className="ml-1 underline-offset-2 hover:underline" style={{ color: 'var(--cp-accent)' }}>
              {t('aiCenter.usecases.openGroups', 'Routing groups')}
            </button>
          )}
        </p>
      </section>
      <UsecaseTableView view={view} groupNames={groupNames} isAdmin={isAdmin} />
    </div>
  )
}
