/* Mounted once at the workspace shell: answers `openPermissions` (requests.ts) and a Block's "权限…" (blockui/bus.ts) —
 * the Workspace's editor as a tab of the data mode (ui/datamode, a phone's too), an object's
 * permissions in the side drawer without leaving the canvas — and keeps the entity capabilities of the whole window
 * current when the permissions move (权限需求 R4: the outline is read again, `info()` was refreshed by the session). */

import { useEffect, useRef, useState, useSyncExternalStore } from 'react'
import { X } from 'lucide-react'
import { useEntity, useStore } from '../../state/hooks'
import { useAiwsText } from '../../useAiwsText'
import { onOpenPermissions } from '../blockui/bus'
import { openDataEditor, PERMISSIONS_TAB } from '../datamode/tabs'
import { ENTITY_PERMISSIONS_EVENT } from '../surfaces/tree'
import { entityLabel } from '../sources/dataOps'
import { useOverlayMounted } from '../shell/popover'
import { useShell } from '../shell/shellContext'
import { useAclRev, useWorkspaceCaps } from './aclData'
import { AclAuditList } from './AclAuditList'
import { EntityPermissions } from './EntityPermissions'
import { MacroIdentity } from './MacroIdentity'
import { permissionRequests } from './requests'

/** The side drawer of one object's permissions (§3.2 "侧边抽屉", §8.1): its by-entity view, a macro's identity, the
 * records naming it. */
export function EntityPermissionsDrawer({ entityId: first, alsoId = null, onClose }: { entityId: string; alsoId?: string | null; onClose: () => void }) {
  const { t } = useAiwsText()
  const shell = useShell()
  const [entityId, setEntityId] = useState(first)
  const entity = useEntity(entityId)
  const firstEntity = useEntity(first)
  const also = useEntity(alsoId ?? '')
  const { capabilities } = useWorkspaceCaps()
  useOverlayMounted(shell.phone)
  useEffect(() => {
    const onKey = (event: KeyboardEvent) => { if (event.key === 'Escape') onClose() }
    document.addEventListener('keydown', onKey)
    return () => document.removeEventListener('keydown', onKey)
  }, [onClose])
  return (
    <aside className={`aiws-perm-drawer${shell.phone ? ' is-phone' : ''}`} role="dialog" aria-label={t('aiworkspace.perm.drawer', { name: entity ? entityLabel(entity) : entityId })} data-testid="aiws-perm-drawer" data-entity-id={entityId}>
      <div className="aiws-perm-drawer-head">
        <b>{t('aiworkspace.perm.drawerTitle')}</b>
        <span className="aiws-grow" />
        <button type="button" className="aiws-tool" aria-label={t('aiworkspace.common.close')} data-testid="aiws-perm-drawer-close" onClick={onClose}><X size={18} /></button>
      </div>
      {alsoId && also && firstEntity && (
        <div className="aiws-perm-tabs" role="tablist" aria-label={t('aiworkspace.perm.drawerTitle')}>
          {[firstEntity, also].map((e) => <button key={e.entity_id} type="button" role="tab" aria-selected={entityId === e.entity_id} data-testid="aiws-perm-drawer-target" data-entity-id={e.entity_id} onClick={() => setEntityId(e.entity_id)}>{entityLabel(e)}</button>)}
        </div>
      )}
      <div className="aiws-perm-drawer-body">
        {entity ? <EntityPermissions entityId={entityId} /> : <div className="aiws-muted" data-testid="aiws-perm-drawer-gone">{t('aiworkspace.perm.entityGone')}</div>}
        {entity?.type_id === 'buckyos.wish' && <MacroIdentity wishId={entityId} />}
        {entity && capabilities.includes('manage') && <details className="aiws-perm-drawer-audit"><summary>{t('aiworkspace.perm.audit.title')}</summary><AclAuditList entityId={entityId} /></details>}
      </div>
    </aside>
  )
}

export function PermissionsHost() {
  const store = useStore()
  const request = useSyncExternalStore(permissionRequests.subscribe, permissionRequests.snapshot)
  const rev = useAclRev()
  const seen = useRef(rev)
  // a Block's "权限…": the data it shows first, the Block itself beside it
  useEffect(() => onOpenPermissions((r) => permissionRequests.open({ kind: 'entity', entityId: r.dataId ?? r.entityId, alsoId: r.dataId ? r.entityId : null })), [])
  // a canvas's or a canvas folder's "权限…" in the canvas picker (§8.1): the same drawer, without leaving the canvas
  useEffect(() => {
    const onEntity = (event: Event) => {
      const entityId = (event as CustomEvent<{ entityId?: string }>).detail?.entityId
      if (!entityId) return
      event.preventDefault()
      permissionRequests.open({ kind: 'entity', entityId })
    }
    window.addEventListener(ENTITY_PERMISSIONS_EVENT, onEntity)
    return () => window.removeEventListener(ENTITY_PERMISSIONS_EVENT, onEntity)
  }, [])
  useEffect(() => {
    if (rev <= seen.current) return
    seen.current = rev
    // the capabilities of every entity may have moved: read the outline again (the envelopes carry them)
    void store.outline.reload().then(() => store.versions.bump(['outline', 'any'])).catch(() => undefined)
  }, [rev, store])
  // the Workspace's editor is a tab of the data mode, a phone's included (UI 框架 PRD §5.6, §8.1)
  useEffect(() => {
    if (request?.kind !== 'workspace') return
    permissionRequests.open(null)
    openDataEditor(PERMISSIONS_TAB)
  }, [request])
  if (!request || request.kind === 'workspace') return null
  return <EntityPermissionsDrawer key={`${request.entityId} ${request.alsoId ?? ''}`} entityId={request.entityId} alsoId={request.alsoId} onClose={() => permissionRequests.open(null)} />
}
