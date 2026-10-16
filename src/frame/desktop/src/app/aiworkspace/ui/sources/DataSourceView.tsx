/* The data-source top-level mode (phase two §6): left the data tree and permissions, middle the
 * selected data's content or relations, right its properties. Panel widths and the selection
 * live in the user work state. */

import { useEffect } from 'react'
import { useOutlineVersion, useStore, useUserState } from '../../state/hooks'
import { useAiwsText } from '../../useAiwsText'
import { DataDetail } from './DataDetail'
import { DataTree, DataTreeActions } from './DataTree'
import { PermissionsPanel } from './PermissionsPanel'
import { PropertiesPanel } from './PropertiesPanel'

export function DataSourceView({ selected, onSelect }: { selected: string | null; onSelect: (id: string | null) => void }) {
  const { t } = useAiwsText()
  const store = useStore()
  useOutlineVersion()
  const entity = selected ? store.outline.get(selected) : undefined
  const panel = useUserState<'tree' | 'permissions'>('sources:left') ?? 'tree'
  const leftWidth = useUserState<number>('sources:left-width') ?? 280
  useEffect(() => { if (selected && !entity) onSelect(null) }, [selected, entity, onSelect])
  return (
    <div className="aiws-sources" data-testid="aiws-sources">
      <aside className="aiws-sources-left" style={{ width: leftWidth }}>
        <div className="aiws-inline-form">
          <button type="button" aria-pressed={panel === 'tree'} data-testid="aiws-left-tree" onClick={() => store.userState.set('sources:left', 'tree')}>{t('aiworkspace.data.tree.title')}</button>
          <button type="button" aria-pressed={panel === 'permissions'} data-testid="aiws-left-permissions" onClick={() => store.userState.set('sources:left', 'permissions')}>{t('aiworkspace.data.perm.title')}</button>
          <span className="aiws-grow" />
          <button type="button" className="aiws-icon" title={t('aiworkspace.data.sources.resize')} onClick={() => store.userState.set('sources:left-width', leftWidth >= 360 ? 240 : leftWidth + 60)}>⇔</button>
        </div>
        {panel === 'tree' ? (
          <>
            <DataTree selected={selected} onSelect={onSelect} />
            <DataTreeActions selected={selected} onSelect={onSelect} />
          </>
        ) : <PermissionsPanel />}
      </aside>
      <main className="aiws-sources-main">
        {entity ? <DataDetail key={entity.entity_id} entity={entity} /> : (
          <div className="aiws-empty" data-testid="aiws-detail-empty">
            <p>{t('aiworkspace.data.sources.empty')}</p>
            <p className="aiws-muted">{t('aiworkspace.data.sources.emptyHint')}</p>
          </div>
        )}
      </main>
      <aside className="aiws-sources-right">
        {entity ? <PropertiesPanel key={entity.entity_id} entity={entity} /> : <div className="aiws-muted">{t('aiworkspace.data.props.title')}</div>}
      </aside>
    </div>
  )
}
