/* The data-source top-level mode (phase two §6): left the data tree and permissions, middle the
 * selected data's content or relations, right its properties. Panel widths and the selection
 * live in the user work state. */

import { useEffect } from 'react'
import { useOutlineVersion, useStore, useUserState } from '../../state/hooks'
import { DataDetail } from './DataDetail'
import { DataTree, DataTreeActions } from './DataTree'
import { PermissionsPanel } from './PermissionsPanel'
import { PropertiesPanel } from './PropertiesPanel'

export function DataSourceView({ selected, onSelect }: { selected: string | null; onSelect: (id: string | null) => void }) {
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
          <button type="button" aria-pressed={panel === 'tree'} data-testid="aiws-left-tree" onClick={() => store.userState.set('sources:left', 'tree')}>数据树</button>
          <button type="button" aria-pressed={panel === 'permissions'} data-testid="aiws-left-permissions" onClick={() => store.userState.set('sources:left', 'permissions')}>权限管理</button>
          <span className="aiws-grow" />
          <button type="button" className="aiws-icon" title="调整宽度" onClick={() => store.userState.set('sources:left-width', leftWidth >= 360 ? 240 : leftWidth + 60)}>⇔</button>
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
            <p>在左侧数据树中选择一项数据：中间显示它的内容与引用关系，右侧显示属性。</p>
            <p className="aiws-muted">画布上创建的内容默认也是数据，放在“画布内容区”里（默认折叠）。</p>
          </div>
        )}
      </main>
      <aside className="aiws-sources-right">
        {entity ? <PropertiesPanel key={entity.entity_id} entity={entity} /> : <div className="aiws-muted">属性</div>}
      </aside>
    </div>
  )
}
