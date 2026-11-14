/* The rule list (§8.3): every entry of the Workspace — subject by name, the resource with its type icon (a click
 * locates it), allowed and refused capabilities in their own colours — filtered by subject, by path prefix, or to the
 * entries with a refusal; each opens the rule editor, and rules are added with the path help. On a narrow panel the
 * rows become cards (container query in permissions.css). */

import { useContext, useState } from 'react'
import { Pencil, Plus } from 'lucide-react'
import type { AclEntry, AclEntryList } from '../../api/types'
import { WorkspaceUiContext } from '../../state/hooks'
import { useAiwsText } from '../../useAiwsText'
import { CapTags } from './CapabilityEditor'
import { SubjectIcon, TypeIcon } from './icons'
import { useLabels, type Labels } from './labels'
import { sortEntries } from './logic'
import { GUEST, groupsHoldingManage, subjectKind } from './model'
import { RuleEditor, type RuleTarget } from './RuleEditor'

/** The rows of some entries; `onEdit` null: read-only. `showSubject` false inside a subject's own page. */
export function RuleRows({ entries, labels, anonymous, onEdit, showSubject = true, readOnlyReason }: {
  entries: readonly AclEntry[]
  labels: Labels
  anonymous: boolean
  onEdit: ((entry: AclEntry) => void) | null
  showSubject?: boolean
  /** Why a row cannot be changed by this caller (a group holding `manage` for a manager), or null. */
  readOnlyReason?: (entry: AclEntry) => string | null
}) {
  const { t } = useAiwsText()
  const ui = useContext(WorkspaceUiContext)
  if (entries.length === 0) return <div className="aiws-muted" data-testid="aiws-perm-rules-empty">{t('aiworkspace.perm.rules.none')}</div>
  return (
    <div className={`aiws-perm-rules${showSubject ? '' : ' is-one-subject'}`} role="table" aria-label={t('aiworkspace.perm.rules.title')}>
      <div className="aiws-perm-rule is-head" role="row">
        {showSubject && <span role="columnheader">{t('aiworkspace.perm.subject')}</span>}
        <span role="columnheader">{t('aiworkspace.perm.resource')}</span>
        <span role="columnheader">{t('aiworkspace.perm.allowed')}</span>
        <span role="columnheader">{t('aiworkspace.perm.denied')}</span>
        <span role="columnheader" />
      </div>
      {entries.map((e) => {
        const entity = labels.entity(e.resource.entity_id)
        const reason = readOnlyReason?.(e) ?? null
        return (
          <div key={`${e.subject} ${e.resource.entity_id ?? ''}`} className={`aiws-perm-rule${e.deny.length ? ' has-deny' : ''}`} role="row" data-testid="aiws-perm-rule" data-subject={e.subject} data-resource={e.resource.path} data-entity-id={e.resource.entity_id ?? ''}>
            {showSubject && (
              <span className="aiws-perm-rule-subject" role="cell">
                <SubjectIcon subject={e.subject} /><span className="aiws-perm-name">{labels.subject(e.subject)}</span>
                {subjectKind(e.subject) === 'publication' && <span className="aiws-chip" data-testid="aiws-perm-rule-publication">{t('aiworkspace.perm.publicationChip')}</span>}
                {e.subject === GUEST && !anonymous && <span className="aiws-chip aiws-chip-warn">{t('aiworkspace.perm.guestInactive')}</span>}
              </span>
            )}
            <span className="aiws-perm-rule-resource" role="cell">
              {e.resource.entity_id && ui
                ? <button type="button" className="aiws-link aiws-perm-resource" data-testid="aiws-perm-rule-locate" title={t('aiworkspace.perm.locate')} onClick={() => ui.openEntity(e.resource.entity_id as string)}><TypeIcon entity={entity} /><span>{e.resource.path}</span></button>
                : <span className="aiws-perm-resource"><TypeIcon entity={entity} /><span>{e.resource.path}</span></span>}
            </span>
            <span className="aiws-perm-rule-allow" role="cell" data-testid="aiws-perm-rule-allow"><span className="aiws-perm-cell-label">{t('aiworkspace.perm.allowed')}</span><CapTags allow={e.allow} empty="—" /></span>
            <span className="aiws-perm-rule-deny" role="cell" data-testid="aiws-perm-rule-deny"><span className="aiws-perm-cell-label">{t('aiworkspace.perm.denied')}</span><CapTags allow={[]} deny={e.deny} empty="—" /></span>
            <span role="cell" className="aiws-perm-rule-actions">
              {onEdit && !reason && <button type="button" className="aiws-tool" aria-label={t('aiworkspace.perm.rule.edit')} data-testid="aiws-perm-rule-edit" onClick={() => onEdit(e)}><Pencil size={16} /></button>}
              {onEdit && reason && <span className="aiws-muted" title={reason} data-testid="aiws-perm-rule-readonly">{t('aiworkspace.perm.ownerOnlyShort')}</span>}
            </span>
          </div>
        )
      })}
    </div>
  )
}

export function RulesPanel({ all, entryTargets, owner }: { all: AclEntryList; entryTargets: ReadonlySet<string>; owner: boolean }) {
  const { t } = useAiwsText()
  const labels = useLabels()
  const [subject, setSubject] = useState('')
  const [prefix, setPrefix] = useState('')
  const [denyOnly, setDenyOnly] = useState(false)
  const [editing, setEditing] = useState<{ target: RuleTarget; fixed: boolean } | null>(null)
  const managing = groupsHoldingManage(all.entries)
  const subjects = [...new Set(all.entries.map((e) => e.subject))].sort()
  const p = prefix.trim()
  const shown = sortEntries(all.entries.filter((e) => (!subject || e.subject === subject)
    && (!p || e.resource.path === p || e.resource.path.startsWith(p.endsWith('/') ? p : `${p}/`) || (p !== '/' && e.resource.path.startsWith(p)))
    && (!denyOnly || e.deny.length > 0)))
  const readOnly = (e: AclEntry) => (!owner && (e.allow.includes('manage') || e.deny.includes('manage') || managing.has(e.subject)) ? t('aiworkspace.perm.ownerOnlyManage') : null)
  return (
    <section className="aiws-perm-section" data-testid="aiws-perm-rules">
      <div className="aiws-perm-section-head">
        <h3>{t('aiworkspace.perm.rules.title')}</h3>
        <span className="aiws-grow" />
        <button type="button" className="is-primary" data-testid="aiws-perm-rule-add" onClick={() => setEditing({ target: {}, fixed: false })}><Plus size={16} aria-hidden="true" />{t('aiworkspace.perm.rule.add')}</button>
      </div>
      <div className="aiws-perm-filters">
        <select aria-label={t('aiworkspace.perm.rules.bySubject')} data-testid="aiws-perm-filter-subject" value={subject} onChange={(event) => setSubject(event.target.value)}>
          <option value="">{t('aiworkspace.perm.rules.allSubjects')}</option>
          {subjects.map((s) => <option key={s} value={s}>{labels.subject(s)}</option>)}
        </select>
        <input aria-label={t('aiworkspace.perm.rules.byPath')} placeholder={t('aiworkspace.perm.rules.byPath')} data-testid="aiws-perm-filter-path" value={prefix} onChange={(event) => setPrefix(event.target.value)} />
        <label className="aiws-perm-check"><input type="checkbox" data-testid="aiws-perm-filter-deny" checked={denyOnly} onChange={(event) => setDenyOnly(event.target.checked)} />{t('aiworkspace.perm.rules.denyOnly')}</label>
      </div>
      <RuleRows entries={shown} labels={labels} anonymous={all.anonymous} readOnlyReason={readOnly}
        onEdit={(e) => setEditing({ target: { subject: e.subject, resource: e.resource.entity_id ?? '/', initial: { allow: e.allow, deny: e.deny } }, fixed: true })} />
      {editing && <RuleEditor target={editing.target} fixedSubject={editing.fixed} fixedResource={editing.fixed} entries={all.entries} entryTargets={entryTargets} onClose={() => setEditing(null)} />}
    </section>
  )
}
