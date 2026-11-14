/* One entry, added or changed (§8.3, §8.4): subject × resource × allowed × refused, written with `acl.set` — online,
 * never queued, never undoable. Refusals are shown and kept; clearing both sets removes the entry. Whatever touches
 * `manage` is the owner's (R17): a manager sees it read-only with the reason. */

import { useState } from 'react'
import { describeError } from '../../api/session'
import type { AclEntry, Capability } from '../../api/types'
import { useStore } from '../../state/hooks'
import { useAiwsText } from '../../useAiwsText'
import { entityLabel } from '../sources/dataOps'
import { useAcl, useWorkspaceCaps } from './aclData'
import { CapTags, CapabilityEditor, Sheet } from './CapabilityEditor'
import { TypeIcon } from './icons'
import { useLabels } from './labels'
import { EMPTY_CAPS, OWNER, forbiddenAllow, isEmptyEntry, sameCaps, type EntryCaps } from './model'
import { checkResourcePath, subjectProblem } from './logic'
import { ResourcePathInput } from './ResourcePathInput'
import { SubjectPicker } from './SubjectPicker'

export interface RuleTarget {
  subject?: string
  /** The resource as a path (typed) or, when fixed, an entity id (`/` the Workspace). */
  resource?: string
  initial?: EntryCaps
}

export function RuleEditor({ target, fixedSubject = false, fixedResource = false, entries, entryTargets, onClose, onSaved }: {
  target: RuleTarget
  fixedSubject?: boolean
  fixedResource?: boolean
  /** The entries known (to say when the chosen subject and resource already have one). */
  entries?: readonly AclEntry[]
  /** Groups some share entry leads into (never `manage` or `run`, R18). */
  entryTargets?: ReadonlySet<string>
  onClose: () => void
  onSaved?: () => void
}) {
  const { t } = useAiwsText()
  const store = useStore()
  const labels = useLabels()
  const { api, online } = useAcl()
  const { owner } = useWorkspaceCaps()
  const [subject, setSubject] = useState(target.subject ?? '')
  const [path, setPath] = useState(target.resource ?? '')
  const [caps, setCaps] = useState<EntryCaps>(target.initial ?? EMPTY_CAPS)
  const [busy, setBusy] = useState(false)
  const [failure, setFailure] = useState<string | null>(null)
  const fixedEntity = fixedResource && target.resource && target.resource !== '/' ? store.outline.get(target.resource) : undefined
  const check = fixedResource ? null : checkResourcePath(store.outline, path)
  const resourceId = fixedResource ? (target.resource ?? '/') : check?.ok ? (check.entity?.entity_id ?? '/') : null
  const existing = entries?.find((e) => e.subject === subject && (e.resource.entity_id ?? '/') === resourceId)
  const initial = target.initial ?? EMPTY_CAPS
  const isNew = isEmptyEntry(initial)
  // a canvas's own data lives in its content folder, outside the canvas's subtree: the same rule there by default
  const resourceEntity = resourceId && resourceId !== '/' ? store.outline.get(resourceId) : undefined
  const contentId = resourceEntity?.kind === 'surface' ? resourceEntity.content_folder_id ?? null : null
  const contentEntry = contentId ? entries?.find((e) => e.subject === subject && e.resource.entity_id === contentId) : undefined
  const [pair, setPair] = useState<boolean | null>(null)
  const paired = contentId !== null && (pair ?? (isNew || Boolean(contentEntry && sameCaps(contentEntry, initial))))
  const subjectError = fixedSubject ? null : subjectProblem(subject, labels.groups)
  const touches = (e: EntryCaps | undefined) => Boolean(e && (e.allow.includes('manage') || e.deny.includes('manage')))
  // touching `manage` (now or before) is the owner's (R17)
  const managed = !owner && (touches(initial) || touches(existing) || (paired && touches(contentEntry)))
  const forbidden: Capability[] = forbiddenAllow(subject, { entryTarget: entryTargets?.has(subject) })
  const removing = !isNew && isEmptyEntry(caps)
  const changed = !sameCaps(caps, initial) || pair !== null
  const canSave = online && !busy && !managed && !subjectError && resourceId !== null && subject !== OWNER && (removing || !isEmptyEntry(caps)) && changed
  const save = async (next: EntryCaps) => {
    if (resourceId === null) return
    setBusy(true)
    setFailure(null)
    try {
      const alsoContent = paired && contentId && (contentEntry || !isEmptyEntry(next))
      await api.set([{ subject, resource: resourceId, allow: next.allow, deny: next.deny }, ...(alsoContent ? [{ subject, resource: contentId, allow: next.allow, deny: next.deny }] : [])])
      onSaved?.()
      onClose()
    } catch (error) {
      setFailure(t('aiworkspace.perm.rule.saveFailed', { error: describeError(error) }))
    } finally { setBusy(false) }
  }
  const title = isNew ? t('aiworkspace.perm.rule.add') : t('aiworkspace.perm.rule.edit')
  return (
    <Sheet label={title} testId="aiws-perm-rule-editor" onClose={onClose}>
      <div className="aiws-perm-form">
        <label className="aiws-perm-field"><span>{t('aiworkspace.perm.subject')}</span>
          {fixedSubject ? <b data-testid="aiws-perm-rule-subject">{labels.subject(subject)}</b> : <SubjectPicker value={subject} onChange={setSubject} labels={labels} />}
        </label>
        <div className="aiws-perm-field"><span>{t('aiworkspace.perm.resource')}</span>
          {fixedResource
            ? <b className="aiws-perm-resource" data-testid="aiws-perm-rule-resource"><TypeIcon entity={fixedEntity} />{fixedEntity ? entityLabel(fixedEntity) : t('aiworkspace.perm.wholeWorkspace')}</b>
            : <ResourcePathInput value={path} onChange={setPath} />}
        </div>
        {existing && isNew && (
          <div className="aiws-warning" data-testid="aiws-perm-rule-exists">
            {t('aiworkspace.perm.rule.exists')} <CapTags allow={existing.allow} deny={existing.deny} />
            <button type="button" className="aiws-link" data-testid="aiws-perm-rule-load-existing" onClick={() => setCaps({ allow: existing.allow, deny: existing.deny })}>{t('aiworkspace.perm.rule.loadExisting')}</button>
          </div>
        )}
        {managed && <div className="aiws-warning" data-testid="aiws-perm-rule-owner-only">{t('aiworkspace.perm.ownerOnlyManage')}</div>}
        <CapabilityEditor value={caps} onChange={setCaps} forbidden={forbidden} locked={owner ? [] : ['manage']} disabled={managed || busy} />
        {contentId && (
          <label className="aiws-perm-check" data-testid="aiws-perm-rule-pair-field">
            <input type="checkbox" data-testid="aiws-perm-rule-pair" checked={paired} disabled={busy} onChange={(event) => setPair(event.target.checked)} />
            {t('aiworkspace.perm.rule.pairContent', { path: labels.resource({ entity_id: contentId, path: contentId }) })}
          </label>
        )}
        {removing && <div className="aiws-muted" data-testid="aiws-perm-rule-removing">{t('aiworkspace.perm.rule.clearRemoves')}</div>}
        {!online && <div className="aiws-warning">{t('aiworkspace.perm.offline')}</div>}
        {failure && <div className="aiws-error" role="alert" data-testid="aiws-perm-rule-error">{failure}</div>}
        <div className="aiws-dialog-actions">
          {!isNew && !managed && <button type="button" data-testid="aiws-perm-rule-delete" disabled={!online || busy} onClick={() => { void save(EMPTY_CAPS) }}>{t('aiworkspace.perm.rule.delete')}</button>}
          <span className="aiws-grow" />
          <button type="button" onClick={onClose}>{t('aiworkspace.common.cancel')}</button>
          <button type="button" className="is-primary" data-testid="aiws-perm-rule-save" disabled={!canSave} onClick={() => { void save(caps) }}>{removing ? t('aiworkspace.perm.rule.delete') : t('aiworkspace.common.save')}</button>
        </div>
      </div>
    </Sheet>
  )
}
