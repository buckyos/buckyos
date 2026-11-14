/* The permissions of one object (§8.4): every subject with entries on it or above it — its effective capabilities, where
 * each entry sits (this object, inherited from a parent path, the Workspace), which refusal overrides which allowance —
 * and the macros bound to a group that reach it ("through macro X", with who may change X). A subject's entry on this
 * object is changed in place, or a subject is added with the resource already given. Without `manage` on the object
 * it is "my permissions". Used by the data mode's right sidebar ("权限") and the canvas side drawer. */

import { useState } from 'react'
import { Pencil, Plus } from 'lucide-react'
import type { AclEntitySubject, AclEntityView } from '../../api/types'
import { useEntity } from '../../state/hooks'
import { useAiwsText } from '../../useAiwsText'
import { entityLabel } from '../sources/dataOps'
import { useAcl, useAclRead, useWorkspaceCaps } from './aclData'
import { CapTags } from './CapabilityEditor'
import { SubjectIcon, TypeIcon } from './icons'
import { useLabels, type Labels } from './labels'
import { GUEST, OWNER, subjectKind } from './model'
import { MyPermissions } from './MyPermissions'
import { RuleEditor, type RuleTarget } from './RuleEditor'

export interface EntityPermissionsProps {
  entityId: string
}

export function EntityPermissions({ entityId }: EntityPermissionsProps) {
  const entity = useEntity(entityId)
  const { capabilities } = useWorkspaceCaps()
  const manages = entity ? entity.capabilities.includes('manage') : capabilities.includes('manage')
  if (!manages) return <MyPermissions entityId={entityId} />
  return <EntityView entityId={entityId} />
}

function sourceText(t: (key: string, vars?: Record<string, string | number>) => string, source: 'direct' | 'inherited' | 'workspace', path: string) {
  return source === 'direct' ? t('aiworkspace.perm.source.direct') : source === 'workspace' ? t('aiworkspace.perm.source.workspace') : t('aiworkspace.perm.source.inherited', { path })
}

function SubjectCard({ item, view, labels, onEdit }: { item: AclEntitySubject; view: AclEntityView; labels: Labels; onEdit: (() => void) | null }) {
  const { t } = useAiwsText()
  // the refusals of others overriding this subject's allowances (§8.4 "被哪条拒绝压过")
  const overriddenBy = view.subjects.flatMap((other) => other.entries.flatMap((e) => (e.overrides ?? []).filter((o) => o.subject === item.subject).map((o) => ({ by: other.subject, at: e.resource.path, over: o }))))
  return (
    <div className="aiws-perm-entity-subject" data-testid="aiws-perm-entity-subject" data-subject={item.subject}>
      <div className="aiws-perm-entity-subject-head">
        <SubjectIcon subject={item.subject} />
        <b className="aiws-perm-name">{labels.subject(item.subject)}</b>
        {item.subject === OWNER && item.principal && <span className="aiws-muted">{labels.member(item.principal)}</span>}
        {subjectKind(item.subject) === 'publication' && <span className="aiws-chip">{t('aiworkspace.perm.publicationChip')}</span>}
        {item.subject === GUEST && !view.anonymous && <span className="aiws-chip aiws-chip-warn">{t('aiworkspace.perm.guestInactive')}</span>}
        <span className="aiws-grow" />
        {onEdit && <button type="button" className="aiws-tool" aria-label={t('aiworkspace.perm.entity.editHere')} title={t('aiworkspace.perm.entity.editHere')} data-testid="aiws-perm-entity-edit" onClick={onEdit}><Pencil size={16} /></button>}
      </div>
      <div className="aiws-perm-entity-effective" data-testid="aiws-perm-entity-effective"><span className="aiws-muted">{t('aiworkspace.perm.effective')}</span> <CapTags allow={item.effective} /></div>
      {item.subject === OWNER && <div className="aiws-muted">{t('aiworkspace.perm.fixedDesc.owner')}</div>}
      {item.entries.map((e) => (
        <div key={e.resource.entity_id ?? '/'} className="aiws-perm-entity-entry" data-testid="aiws-perm-entity-entry" data-source={e.source} data-resource={e.resource.path}>
          <span className="aiws-muted">{sourceText(t, e.source, e.resource.path)}</span>
          <CapTags allow={e.allow} deny={e.deny} />
          {(e.overrides ?? []).map((o) => (
            <div key={`${o.subject} ${o.resource.path}`} className="aiws-perm-override" data-testid="aiws-perm-override" data-subject={o.subject}>
              {t('aiworkspace.perm.entity.overrides', { subject: labels.subject(o.subject), where: sourceText(t, o.resource.path === view.resource.path ? 'direct' : o.resource.path === '/' ? 'workspace' : 'inherited', o.resource.path) })} <CapTags allow={o.capabilities} />
            </div>
          ))}
        </div>
      ))}
      {overriddenBy.map((o) => (
        <div key={`${o.by} ${o.at}`} className="aiws-perm-overridden" data-testid="aiws-perm-overridden-by" data-subject={o.by}>
          {t('aiworkspace.perm.entity.overriddenBy', { subject: labels.subject(o.by), where: sourceText(t, o.at === view.resource.path ? 'direct' : o.at === '/' ? 'workspace' : 'inherited', o.at) })} <CapTags allow={[]} deny={o.over.capabilities} />
        </div>
      ))}
    </div>
  )
}

function EntityView({ entityId }: { entityId: string }) {
  const { t, list } = useAiwsText()
  const labels = useLabels()
  const { api } = useAcl()
  const view = useAclRead(`entity:${entityId}`, () => api.entity(entityId))
  const all = useAclRead('all', api.all)
  const shares = useAclRead('share-entries', api.entries)
  const [editing, setEditing] = useState<{ target: RuleTarget; fixedSubject: boolean } | null>(null)
  const entity = labels.entity(entityId)
  const entryTargets = new Set((shares.data ?? []).filter((e) => e.state === 'active').map((e) => `@${e.group_id}`))
  const data = view.data
  return (
    <section className="aiws-perm-entity" data-testid="aiws-perm-entity" data-entity-id={entityId}>
      <div className="aiws-perm-section-head">
        <TypeIcon entity={entity} />
        <h3>{entity ? entityLabel(entity) : entityId}</h3>
        {data && <code className="aiws-muted">{data.resource.path}</code>}
        <span className="aiws-grow" />
        <button type="button" className="is-primary" data-testid="aiws-perm-entity-add" onClick={() => setEditing({ target: { resource: entityId }, fixedSubject: false })}><Plus size={16} aria-hidden="true" />{t('aiworkspace.perm.entity.add')}</button>
      </div>
      <p className="aiws-muted">{t('aiworkspace.perm.entity.intro')}</p>
      {view.error && <div className="aiws-error" role="alert">{view.error}</div>}
      {view.loading && !data && <div className="aiws-muted">{t('aiworkspace.perm.loading')}</div>}
      {data && data.subjects.map((s) => {
        const direct = s.entries.find((e) => e.source === 'direct')
        return (
          <SubjectCard key={s.subject} item={s} view={data} labels={labels}
            onEdit={s.subject === OWNER ? null : () => setEditing({ target: { subject: s.subject, resource: entityId, initial: direct ? { allow: direct.allow, deny: direct.deny } : undefined }, fixedSubject: true })} />
        )
      })}
      {data && data.macros.length > 0 && (
        <div className="aiws-perm-macros" data-testid="aiws-perm-macros">
          <div className="aiws-perm-subhead">{t('aiworkspace.perm.entity.macros')}</div>
          {data.macros.map((m) => {
            const wish = labels.entity(m.wish.entity_id)
            return (
              <div key={`${m.wish.entity_id} ${m.group_id}`} className="aiws-perm-macro" data-testid="aiws-perm-macro" data-wish-id={m.wish.entity_id ?? ''}>
                <div>
                  <TypeIcon entity={wish} />{' '}
                  {t(m.mode === 'union' ? 'aiworkspace.perm.entity.macroUnion' : 'aiworkspace.perm.entity.macroIntersection', { wish: wish ? entityLabel(wish) : m.wish.path, group: labels.subject(`@${m.group_id}`) })}{' '}
                  <CapTags allow={m.capabilities} />
                </div>
                <div className="aiws-muted" data-testid="aiws-perm-macro-editors">{t('aiworkspace.perm.entity.macroEditors', { editors: list(m.editors.map((e) => labels.subject(e))) })}</div>
              </div>
            )
          })}
        </div>
      )}
      {editing && <RuleEditor target={editing.target} fixedSubject={editing.fixedSubject} fixedResource entries={all.data?.entries} entryTargets={entryTargets} onClose={() => setEditing(null)} />}
    </section>
  )
}
