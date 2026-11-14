/* Choosing the subject of an entry (§8.3): the fixed groups, the logical groups, the people and Agents the Zone lists,
 * or a principal typed in (a user id, a DID). `group:` / `contact:` / `*` are members of a logical group, never a
 * subject (R7): typing one says so. */

import { useState } from 'react'
import { useAiwsText } from '../../useAiwsText'
import type { Labels } from './labels'
import { subjectProblem } from './logic'
import { ALL, CREATOR, GUEST } from './model'

const OTHER = '__other'

export function SubjectPicker({ value, onChange, labels, people = true, groups = true, testId = 'aiws-perm-subject' }: {
  value: string
  onChange: (subject: string) => void
  labels: Labels
  /** Offer single callers (users, Agents, DIDs). */
  people?: boolean
  /** Offer the fixed and logical groups. */
  groups?: boolean
  testId?: string
}) {
  const { t } = useAiwsText()
  const persons = labels.directory.candidates.filter((c) => !c.subject.startsWith('group:'))
  const known = (groups && ([ALL, GUEST, CREATOR].includes(value) || labels.groups.some((g) => g.subject === value))) || (people && persons.some((p) => p.subject === value))
  const [other, setOther] = useState(() => value !== '' && !known)
  const selected = other ? OTHER : value
  const problem = other && value ? subjectProblem(value, labels.groups) : null
  return (
    <span className="aiws-perm-subject-picker">
      <select aria-label={t('aiworkspace.perm.subject')} data-testid={testId} value={selected}
        onChange={(event) => {
          if (event.target.value === OTHER) { setOther(true); onChange('') } else { setOther(false); onChange(event.target.value) }
        }}>
        <option value="" disabled>{t('aiworkspace.perm.subject.choose')}</option>
        {groups && (
          <optgroup label={t('aiworkspace.perm.fixedGroups')}>
            {[ALL, GUEST, CREATOR].map((s) => <option key={s} value={s}>{labels.subject(s)}</option>)}
          </optgroup>
        )}
        {groups && labels.groups.length > 0 && (
          <optgroup label={t('aiworkspace.perm.logicalGroups')}>
            {labels.groups.map((g) => <option key={g.subject} value={g.subject}>{g.title || g.group_id} (@{g.group_id})</option>)}
          </optgroup>
        )}
        {people && persons.length > 0 && (
          <optgroup label={t('aiworkspace.perm.people')}>
            {persons.map((p) => <option key={p.subject} value={p.subject}>{labels.member(p.subject)}{labels.member(p.subject) !== p.subject ? ` (${p.subject})` : ''}</option>)}
          </optgroup>
        )}
        {people && <option value={OTHER}>{t('aiworkspace.perm.subject.other')}</option>}
      </select>
      {other && (
        <input aria-label={t('aiworkspace.perm.subject.typed')} data-testid={`${testId}-input`} placeholder={t('aiworkspace.perm.subject.placeholder')} value={value}
          onChange={(event) => onChange(event.target.value.trim())} aria-invalid={problem ? true : undefined} />
      )}
      {problem && <span className="aiws-perm-invalid" role="status" data-testid={`${testId}-problem`}>{t(problem)}</span>}
    </span>
  )
}
