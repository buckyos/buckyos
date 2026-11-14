/* The resource of a new entry, typed as a path (§8.3) — with help: completion one level at a time, the object picker
 * (the first choice on a phone), examples always shown, a check while typing (the object's icon, name and type, or why
 * the path names nothing that can be a target) and what an entry on a path means. Paths resolve over the outline this
 * window holds, which is what the caller can read: exactly what the service resolves for it (api/paths.ts). */

import { useMemo, useState, type ReactNode } from 'react'
import { ChevronDown, ChevronRight, FolderTree } from 'lucide-react'
import { pathOf, resolvePath } from '../../api/paths'
import type { EntityEnvelope } from '../../api/types'
import { useOutlineVersion, useStore } from '../../state/hooks'
import type { OutlineModel } from '../../state/outline'
import { useAiwsText } from '../../useAiwsText'
import { entityLabel } from '../sources/dataOps'
import { usePhone } from '../shell/shellContext'
import { Sheet } from './CapabilityEditor'
import { TypeIcon } from './icons'
import { useTypeName } from './labels'
import { checkResourcePath } from './logic'

function named(outline: OutlineModel, parentId: string): EntityEnvelope[] {
  return outline.childrenOf(parentId).filter((c) => !c.deleted && c.name && (c.scope ?? 'shared') === 'shared')
}

/** The next step's candidates for what is typed: the children of the typed parent whose name starts with the last step. */
function suggestions(outline: OutlineModel, value: string): { path: string; entity: EntityEnvelope }[] {
  if (value.startsWith('~')) return []
  const v = value.startsWith('/') ? value : `/${value}`
  const cut = v.lastIndexOf('/')
  const parentPath = v.slice(0, cut) || '/'
  const prefix = v.slice(cut + 1).toLowerCase()
  const parent = parentPath === '/' ? outline.get('root') : resolvePath(outline, parentPath)
  if (!parent) return []
  return named(outline, parent.entity_id)
    .filter((c) => (c.name ?? '').toLowerCase().startsWith(prefix) && `${parentPath === '/' ? '' : parentPath}/${c.name}` !== v)
    .slice(0, 8)
    .map((c) => ({ path: pathOf(outline, c.entity_id), entity: c }))
}

export function ResourcePathInput({ value, onChange, testId = 'aiws-perm-path' }: { value: string; onChange: (path: string) => void; testId?: string }) {
  const { t, tNode } = useAiwsText()
  const store = useStore()
  const version = useOutlineVersion()
  const phone = usePhone()
  const typeName = useTypeName()
  const [focused, setFocused] = useState(false)
  const [picking, setPicking] = useState(false)
  const check = useMemo(() => checkResourcePath(store.outline, value), [store.outline, value, version]) // eslint-disable-line react-hooks/exhaustive-deps
  const next = useMemo(() => (focused ? suggestions(store.outline, value) : []), [store.outline, value, focused, version]) // eslint-disable-line react-hooks/exhaustive-deps
  const pick = (
    <button type="button" className={phone ? 'is-primary' : undefined} data-testid={`${testId}-pick`} onClick={() => setPicking(true)}>
      <FolderTree size={16} aria-hidden="true" />{t('aiworkspace.perm.path.pick')}
    </button>
  )
  return (
    <div className="aiws-perm-path" data-testid={`${testId}-field`}>
      {phone && pick}
      <div className="aiws-perm-path-row">
        <input aria-label={t('aiworkspace.perm.path.label')} data-testid={testId} value={value} placeholder="/data/…" spellCheck={false} autoCapitalize="off" autoCorrect="off"
          aria-invalid={!check.ok && check.reason !== 'empty' ? true : undefined}
          onChange={(event) => onChange(event.target.value)} onFocus={() => setFocused(true)} onBlur={() => window.setTimeout(() => setFocused(false), 150)} />
        {!phone && pick}
      </div>
      {next.length > 0 && (
        <div className="aiws-perm-path-suggest" role="listbox" aria-label={t('aiworkspace.perm.path.suggest')} data-testid={`${testId}-suggest`}>
          {next.map((s) => (
            <button key={s.entity.entity_id} type="button" role="option" aria-selected={false} data-testid={`${testId}-suggestion`} data-path={s.path}
              onMouseDown={(event) => event.preventDefault()} onClick={() => onChange(s.path)}>
              <TypeIcon entity={s.entity} /><span>{s.entity.name}</span><span className="aiws-muted">{typeName(s.entity)}</span>
            </button>
          ))}
        </div>
      )}
      <div className="aiws-perm-path-check" data-testid={`${testId}-check`} data-state={check.ok ? 'ok' : check.reason === 'empty' ? 'empty' : 'error'} role="status">
        {check.ok
          ? <><TypeIcon entity={check.entity} /><b>{check.entity ? entityLabel(check.entity) : t('aiworkspace.perm.wholeWorkspace')}</b><span className="aiws-muted">{typeName(check.entity)}</span></>
          : check.reason !== 'empty' && <span className="aiws-perm-invalid">{t(`aiworkspace.perm.path.${check.reason}`)}</span>}
      </div>
      <div className="aiws-muted aiws-perm-path-examples">{tNode('aiworkspace.perm.path.examples', {
        root: <code>/</code>, data: <code>/data/{t('aiworkspace.perm.path.exampleData')}</code>, surface: <code>/surfaces/{t('aiworkspace.perm.path.exampleSurface')}</code>,
        block: <code>/surfaces/{t('aiworkspace.perm.path.exampleSurface')}/text-3</code>,
      })}</div>
      <div className="aiws-muted">{t('aiworkspace.perm.path.binds')}</div>
      {picking && <ObjectPicker onClose={() => setPicking(false)} onPick={(path) => { onChange(path); setPicking(false) }} />}
    </div>
  )
}

/** The object picker (§8.3): the Workspace's tree of what can carry entries, with a search; picking one gives its path. */
export function ObjectPicker({ onPick, onClose }: { onPick: (path: string) => void; onClose: () => void }) {
  const { t } = useAiwsText()
  const store = useStore()
  useOutlineVersion()
  const typeName = useTypeName()
  const [query, setQuery] = useState('')
  const [open, setOpen] = useState<ReadonlySet<string>>(() => new Set(['data', 'surfaces']))
  const q = query.trim().toLowerCase()
  const row = (e: EntityEnvelope, depth: number, showPath = false) => {
    const kids = named(store.outline, e.entity_id).length > 0 || store.outline.childrenOf(e.entity_id).some((c) => !c.deleted && (c.scope ?? 'shared') === 'shared')
    const expanded = open.has(e.entity_id)
    return (
      <div key={e.entity_id} className="aiws-perm-picker-row" style={{ paddingInlineStart: depth * 16 }}>
        {!showPath && (kids
          ? <button type="button" className="aiws-tool" aria-label={expanded ? t('aiworkspace.perm.collapse') : t('aiworkspace.perm.expand')} aria-expanded={expanded} data-testid="aiws-perm-picker-expand" data-entity-id={e.entity_id}
            onClick={() => setOpen((s) => { const n = new Set(s); if (n.has(e.entity_id)) n.delete(e.entity_id); else n.add(e.entity_id); return n })}>{expanded ? <ChevronDown size={16} /> : <ChevronRight size={16} />}</button>
          : <span className="aiws-perm-picker-spacer" />)}
        <button type="button" className="aiws-perm-picker-item" data-testid="aiws-perm-picker-item" data-entity-id={e.entity_id} onClick={() => onPick(pathOf(store.outline, e.entity_id))}>
          <TypeIcon entity={e} /><span>{entityLabel(e)}</span><span className="aiws-muted">{showPath ? pathOf(store.outline, e.entity_id) : typeName(e)}</span>
        </button>
      </div>
    )
  }
  const tree = (parentId: string, depth: number): ReactNode[] => store.outline.childrenOf(parentId)
    .filter((c) => !c.deleted && (c.scope ?? 'shared') === 'shared')
    .flatMap((c) => [row(c, depth), ...(open.has(c.entity_id) ? tree(c.entity_id, depth + 1) : [])])
  const found = q ? store.outline.all().filter((e) => !e.deleted && e.entity_id !== 'root' && (e.scope ?? 'shared') === 'shared' && (entityLabel(e).toLowerCase().includes(q) || (e.name ?? '').toLowerCase().includes(q))).slice(0, 100) : []
  return (
    <Sheet label={t('aiworkspace.perm.path.pickTitle')} testId="aiws-perm-picker" onClose={onClose} wide>
      <input aria-label={t('aiworkspace.perm.path.search')} placeholder={t('aiworkspace.perm.path.search')} data-testid="aiws-perm-picker-search" value={query} onChange={(event) => setQuery(event.target.value)} />
      <div className="aiws-perm-picker-tree">
        {!q && (
          <div className="aiws-perm-picker-row">
            <span className="aiws-perm-picker-spacer" />
            <button type="button" className="aiws-perm-picker-item" data-testid="aiws-perm-picker-item" data-entity-id="root" onClick={() => onPick('/')}>
              <TypeIcon entity={null} /><span>{t('aiworkspace.perm.wholeWorkspace')}</span><span className="aiws-muted">/</span>
            </button>
          </div>
        )}
        {q ? found.map((e) => row(e, 0, true)) : tree('root', 0)}
        {q && found.length === 0 && <div className="aiws-muted">{t('aiworkspace.perm.path.noMatch')}</div>}
      </div>
    </Sheet>
  )
}
