/* The groups of the Workspace (§8.2): the fixed groups first, with what they mean and that they stay; then the logical
 * groups (title, id, description, members, rules), created, renamed, described and deleted here. A group's page lists
 * its members by where they come from and its rules by resource path. A group holding `manage` and its members are the
 * owner's to change (R17); a group is never a member of another. */

import { useState } from 'react'
import { ChevronRight, Plus, Trash2, UserPlus } from 'lucide-react'
import { describeError } from '../../api/session'
import type { AclEntry, AclEntryList, AclGroup, AclMember, ShareEntry } from '../../api/types'
import { useAiwsText } from '../../useAiwsText'
import { useShell } from '../shell/shellContext'
import { useAcl } from './aclData'
import { MemberIcon, SubjectIcon } from './icons'
import { useLabels, type Labels } from './labels'
import { sortEntries } from './logic'
import { ALL, CREATOR, GUEST, OWNER, groupsHoldingManage, isGroupId, memberProblem } from './model'
import { RuleEditor, type RuleTarget } from './RuleEditor'
import { RuleRows } from './RulesPanel'

const FIXED = [OWNER, ALL, GUEST, CREATOR]

export interface GroupsData {
  all: AclEntryList
  groups: AclGroup[]
  entries: ShareEntry[]
  owner: boolean
}

export function GroupList({ data, selected, onSelect }: { data: GroupsData; selected: string | null; onSelect: (subject: string) => void }) {
  const { t } = useAiwsText()
  const shell = useShell()
  const labels = useLabels()
  const [creating, setCreating] = useState(false)
  const rules = (subject: string) => data.all.entries.filter((e) => e.subject === subject).length
  return (
    <div className="aiws-perm-group-list" data-testid="aiws-perm-groups">
      <div className="aiws-perm-subhead">{t('aiworkspace.perm.fixedGroups')}</div>
      {FIXED.map((s) => (
        <div key={s} className={`aiws-perm-group-row${selected === s ? ' is-selected' : ''}`} data-testid="aiws-perm-group" data-subject={s}>
          <button type="button" className="aiws-perm-group-main" aria-current={selected === s ? 'true' : undefined} onClick={() => onSelect(s)}>
            <SubjectIcon subject={s} />
            <span className="aiws-perm-group-text">
              <b>{labels.subject(s)}</b> <code className="aiws-muted">{s}</code>
              <span className="aiws-muted aiws-perm-group-desc">{t(`aiworkspace.perm.fixedDesc.${s.slice(1)}`)}</span>
            </span>
            <span className="aiws-perm-group-counts aiws-muted">{s === OWNER ? t('aiworkspace.perm.everything') : t('aiworkspace.perm.rulesCount', { count: rules(s) })}</span>
            <ChevronRight size={16} aria-hidden="true" className="aiws-perm-chevron" />
          </button>
          {s === GUEST && !data.all.anonymous && (
            <div className="aiws-perm-group-note" data-testid="aiws-perm-guest-inactive">
              <span className="aiws-chip aiws-chip-warn">{t('aiworkspace.perm.guestInactive')}</span>
              {data.all.complete && <button type="button" className="aiws-link" data-testid="aiws-perm-guest-enable" onClick={() => shell.openDialog({ kind: 'share' })}>{t('aiworkspace.perm.guestEnable')}</button>}
            </div>
          )}
        </div>
      ))}
      <div className="aiws-perm-subhead">
        {t('aiworkspace.perm.logicalGroups')}
        <span className="aiws-grow" />
        <button type="button" data-testid="aiws-perm-group-new" onClick={() => setCreating(true)}><Plus size={16} aria-hidden="true" />{t('aiworkspace.perm.group.new')}</button>
      </div>
      {creating && <GroupForm onDone={(id) => { setCreating(false); if (id) onSelect(`@${id}`) }} />}
      {data.groups.length === 0 && !creating && <div className="aiws-muted">{t('aiworkspace.perm.group.none')}</div>}
      {data.groups.map((g) => (
        <div key={g.subject} className={`aiws-perm-group-row${selected === g.subject ? ' is-selected' : ''}`} data-testid="aiws-perm-group" data-subject={g.subject}>
          <button type="button" className="aiws-perm-group-main" aria-current={selected === g.subject ? 'true' : undefined} onClick={() => onSelect(g.subject)}>
            <SubjectIcon subject={g.subject} />
            <span className="aiws-perm-group-text">
              <b>{g.title || g.group_id}</b> <code className="aiws-muted">@{g.group_id}</code>
              {g.description && <span className="aiws-muted aiws-perm-group-desc">{g.description}</span>}
            </span>
            <span className="aiws-perm-group-counts aiws-muted" data-testid="aiws-perm-group-counts">{t('aiworkspace.perm.membersCount', { count: g.members?.length ?? 0 })} · {t('aiworkspace.perm.rulesCount', { count: rules(g.subject) })}</span>
            <ChevronRight size={16} aria-hidden="true" className="aiws-perm-chevron" />
          </button>
        </div>
      ))}
    </div>
  )
}

/** A new group (`group` absent) or a group's title and description. Answers the id saved, or null when cancelled. */
function GroupForm({ group, onDone }: { group?: AclGroup; onDone: (id: string | null) => void }) {
  const { t } = useAiwsText()
  const { api, online } = useAcl()
  const [id, setId] = useState(group?.group_id ?? '')
  const [title, setTitle] = useState(group?.title ?? '')
  const [description, setDescription] = useState(group?.description ?? '')
  const [busy, setBusy] = useState(false)
  const [failure, setFailure] = useState<string | null>(null)
  const idOk = Boolean(group) || isGroupId(id)
  const save = async () => {
    setBusy(true)
    setFailure(null)
    try {
      await api.setGroup(id, title.trim() || id, description.trim())
      onDone(id)
    } catch (error) { setFailure(t('aiworkspace.perm.group.saveFailed', { error: describeError(error) })) } finally { setBusy(false) }
  }
  return (
    <form className="aiws-perm-form aiws-perm-group-form" data-testid="aiws-perm-group-form" onSubmit={(event) => { event.preventDefault(); if (idOk && online && !busy) void save() }}>
      {!group && (
        <label className="aiws-perm-field"><span>{t('aiworkspace.perm.group.id')}</span>
          <input data-testid="aiws-perm-group-id" value={id} placeholder="teachers" spellCheck={false} autoCapitalize="off" onChange={(event) => setId(event.target.value.trim())} aria-invalid={id && !idOk ? true : undefined} />
          <span className="aiws-muted">{id && !idOk ? <span className="aiws-perm-invalid">{t('aiworkspace.perm.group.idInvalid')}</span> : t('aiworkspace.perm.group.idHint')}</span>
        </label>
      )}
      <label className="aiws-perm-field"><span>{t('aiworkspace.perm.group.title')}</span>
        <input data-testid="aiws-perm-group-title" value={title} maxLength={128} onChange={(event) => setTitle(event.target.value)} />
      </label>
      <label className="aiws-perm-field"><span>{t('aiworkspace.perm.group.description')}</span>
        <textarea data-testid="aiws-perm-group-description" value={description} maxLength={2000} rows={2} onChange={(event) => setDescription(event.target.value)} />
      </label>
      {failure && <div className="aiws-error" role="alert">{failure}</div>}
      <div className="aiws-dialog-actions">
        <button type="button" onClick={() => onDone(null)}>{t('aiworkspace.common.cancel')}</button>
        <button type="submit" className="is-primary" data-testid="aiws-perm-group-save" disabled={!idOk || !online || busy}>{group ? t('aiworkspace.common.save') : t('aiworkspace.perm.group.create')}</button>
      </div>
    </form>
  )
}

function viaEntryLabel(t: (key: string, vars?: Record<string, string | number>) => string, entryId: string): string {
  return t('aiworkspace.perm.member.viaEntry', { entry: entryId.split('~').pop()?.slice(-6) ?? entryId })
}

function MemberRows({ members, labels, canChange, onRemove }: { members: AclMember[]; labels: Labels; canChange: boolean; onRemove: (member: string) => void }) {
  const { t } = useAiwsText()
  if (members.length === 0) return <div className="aiws-muted" data-testid="aiws-perm-members-empty">{t('aiworkspace.perm.member.none')}</div>
  return (
    <ul className="aiws-perm-members">
      {members.map((m) => {
        const kind = labels.memberKind(m.member)
        const name = labels.member(m.member)
        return (
          <li key={m.member} className="aiws-perm-member" data-testid="aiws-perm-member" data-member={m.member}>
            <MemberIcon kind={kind} />
            <span className="aiws-perm-name">{name}{name !== m.member && <span className="aiws-muted"> {m.member}</span>}</span>
            <span className="aiws-chip" data-testid="aiws-perm-member-kind">{m.via_entry ? viaEntryLabel(t, m.via_entry) : t(`aiworkspace.perm.memberKind.${kind}`)}</span>
            <span className="aiws-grow" />
            {canChange && <button type="button" className="aiws-tool" aria-label={t('aiworkspace.perm.member.remove', { name })} data-testid="aiws-perm-member-remove" onClick={() => onRemove(m.member)}><Trash2 size={16} /></button>}
          </li>
        )
      })}
    </ul>
  )
}

function MemberAdd({ groupId, labels, existing }: { groupId: string; labels: Labels; existing: readonly AclMember[] }) {
  const { t } = useAiwsText()
  const { api, online } = useAcl()
  const [choice, setChoice] = useState('')
  const [typed, setTyped] = useState('')
  const [busy, setBusy] = useState(false)
  const [failure, setFailure] = useState<string | null>(null)
  const member = (choice === '__typed' ? typed : choice).trim()
  const problem = choice === '__typed' && typed ? memberProblem(typed.trim()) : null
  const taken = existing.some((m) => m.member === member)
  const people = labels.directory.candidates.filter((c) => !c.subject.startsWith('group:') && !existing.some((m) => m.member === c.subject))
  const sysGroups = labels.directory.candidates.filter((c) => c.subject.startsWith('group:') && !existing.some((m) => m.member === c.subject))
  const add = async () => {
    setBusy(true)
    setFailure(null)
    try {
      await api.members(groupId, [member])
      setChoice('')
      setTyped('')
    } catch (error) { setFailure(t('aiworkspace.perm.member.addFailed', { error: describeError(error) })) } finally { setBusy(false) }
  }
  return (
    <form className="aiws-perm-member-add" data-testid="aiws-perm-member-add" onSubmit={(event) => { event.preventDefault(); if (member && !problem && !taken && online && !busy) void add() }}>
      <select aria-label={t('aiworkspace.perm.member.add')} data-testid="aiws-perm-member-choice" value={choice} onChange={(event) => setChoice(event.target.value)}>
        <option value="">{t('aiworkspace.perm.member.choose')}</option>
        {people.length > 0 && <optgroup label={t('aiworkspace.perm.people')}>{people.map((p) => <option key={p.subject} value={p.subject}>{labels.member(p.subject)}{labels.member(p.subject) !== p.subject ? ` (${p.subject})` : ''}</option>)}</optgroup>}
        {sysGroups.length > 0 && <optgroup label={t('aiworkspace.perm.memberKind.sysgroup')}>{sysGroups.map((g) => <option key={g.subject} value={g.subject}>{labels.member(g.subject)}</option>)}</optgroup>}
        {!existing.some((m) => m.member === '*') && <option value="*">{t('aiworkspace.perm.member.zone')}</option>}
        <option value="__typed">{t('aiworkspace.perm.member.typed')}</option>
      </select>
      {choice === '__typed' && <input aria-label={t('aiworkspace.perm.member.typedLabel')} data-testid="aiws-perm-member-input" placeholder={t('aiworkspace.perm.member.placeholder')} value={typed} spellCheck={false} autoCapitalize="off" onChange={(event) => setTyped(event.target.value)} />}
      <button type="submit" data-testid="aiws-perm-member-submit" disabled={!member || Boolean(problem) || taken || !online || busy}><UserPlus size={16} aria-hidden="true" />{t('aiworkspace.perm.member.addButton')}</button>
      {problem && <span className="aiws-perm-invalid" role="status" data-testid="aiws-perm-member-problem">{t(`aiworkspace.perm.member.problem.${problem}`)}</span>}
      {choice === '__typed' && !problem && <span className="aiws-muted">{t('aiworkspace.perm.member.typedHint')}</span>}
      {failure && <div className="aiws-error" role="alert" data-testid="aiws-perm-member-error">{failure}</div>}
    </form>
  )
}

export function GroupDetail({ subject, data, onDeleted }: { subject: string; data: GroupsData; onDeleted: () => void }) {
  const { t } = useAiwsText()
  const labels = useLabels()
  const { api, online } = useAcl()
  const [editing, setEditing] = useState(false)
  const [rule, setRule] = useState<{ target: RuleTarget; fixed: boolean } | null>(null)
  const [confirmDelete, setConfirmDelete] = useState(false)
  const [failure, setFailure] = useState<string | null>(null)
  const group = data.groups.find((g) => g.subject === subject)
  const fixed = FIXED.includes(subject)
  const holdsManage = groupsHoldingManage(data.all.entries).has(subject)
  const ownerOnly = holdsManage && !data.owner
  const entries: AclEntry[] = sortEntries(data.all.entries.filter((e) => e.subject === subject))
  const entryTargets = new Set(data.entries.filter((e) => e.state === 'active').map((e) => `@${e.group_id}`))
  const act = async (work: () => Promise<unknown>, failed: string) => {
    setFailure(null)
    try { await work() } catch (error) { setFailure(t(failed, { error: describeError(error) })) }
  }
  if (!fixed && !group) return <div className="aiws-muted" data-testid="aiws-perm-group-gone">{t('aiworkspace.perm.group.gone')}</div>
  return (
    <div className="aiws-perm-group-detail" data-testid="aiws-perm-group-detail" data-subject={subject}>
      <div className="aiws-perm-section-head">
        <SubjectIcon subject={subject} size={20} />
        <h3>{labels.subject(subject)}</h3>
        <code className="aiws-muted">{subject}</code>
        <span className="aiws-grow" />
        {group && !ownerOnly && !editing && <button type="button" data-testid="aiws-perm-group-edit" onClick={() => setEditing(true)}>{t('aiworkspace.common.edit')}</button>}
        {group && !ownerOnly && <button type="button" data-testid="aiws-perm-group-delete" disabled={!online} onClick={() => setConfirmDelete(true)}><Trash2 size={16} aria-hidden="true" />{t('aiworkspace.common.delete')}</button>}
      </div>
      {fixed ? <p className="aiws-muted">{t(`aiworkspace.perm.fixedDesc.${subject.slice(1)}`)} {t('aiworkspace.perm.fixedStays')}</p> : group?.description && !editing && <p className="aiws-muted">{group.description}</p>}
      {subject === GUEST && !data.all.anonymous && <div className="aiws-warning">{t('aiworkspace.perm.guestInactiveLong')}</div>}
      {ownerOnly && <div className="aiws-warning" data-testid="aiws-perm-group-owner-only">{t('aiworkspace.perm.group.ownerOnly')}</div>}
      {editing && group && <GroupForm group={group} onDone={() => setEditing(false)} />}
      {confirmDelete && group && (
        <div className="aiws-warning" data-testid="aiws-perm-group-delete-confirm">
          {t('aiworkspace.perm.group.deleteConfirm', { name: group.title || group.group_id, rules: entries.length, members: group.members?.length ?? 0 })}
          <div className="aiws-dialog-actions">
            <button type="button" onClick={() => setConfirmDelete(false)}>{t('aiworkspace.common.cancel')}</button>
            <button type="button" className="is-danger" data-testid="aiws-perm-group-delete-yes" disabled={!online} onClick={() => { void act(async () => { await api.deleteGroup(group.group_id); onDeleted() }, 'aiworkspace.perm.group.deleteFailed') }}>{t('aiworkspace.common.delete')}</button>
          </div>
        </div>
      )}
      {failure && <div className="aiws-error" role="alert">{failure}</div>}
      {group && (
        <section className="aiws-perm-section" data-testid="aiws-perm-members">
          <div className="aiws-perm-subhead">{t('aiworkspace.perm.members')}</div>
          <MemberRows members={group.members ?? []} labels={labels} canChange={!ownerOnly && online}
            onRemove={(m) => { void act(() => api.members(group.group_id, [], [m]), 'aiworkspace.perm.member.removeFailed') }} />
          {!ownerOnly && <MemberAdd groupId={group.group_id} labels={labels} existing={group.members ?? []} />}
          <div className="aiws-muted">{t('aiworkspace.perm.member.noNesting')}</div>
        </section>
      )}
      {fixed && subject !== OWNER && <p className="aiws-muted">{t('aiworkspace.perm.fixedMembers')}</p>}
      {subject !== OWNER && (
        <section className="aiws-perm-section" data-testid="aiws-perm-group-rules">
          <div className="aiws-perm-subhead">
            {t('aiworkspace.perm.groupRules')}
            <span className="aiws-grow" />
            {!ownerOnly && <button type="button" data-testid="aiws-perm-group-rule-add" onClick={() => setRule({ target: { subject }, fixed: false })}><Plus size={16} aria-hidden="true" />{t('aiworkspace.perm.rule.add')}</button>}
          </div>
          <RuleRows entries={entries} labels={labels} anonymous={data.all.anonymous} showSubject={false}
            readOnlyReason={(e) => (ownerOnly || (!data.owner && (e.allow.includes('manage') || e.deny.includes('manage'))) ? t('aiworkspace.perm.ownerOnlyManage') : null)}
            onEdit={(e) => setRule({ target: { subject, resource: e.resource.entity_id ?? '/', initial: { allow: e.allow, deny: e.deny } }, fixed: true })} />
        </section>
      )}
      {rule && <RuleEditor target={rule.target} fixedSubject fixedResource={rule.fixed} entries={data.all.entries} entryTargets={entryTargets} onClose={() => setRule(null)} />}
    </div>
  )
}
