/* ── PeoplePicker: choose people from the owner's contacts and this Zone, or a DID found by search ── */

import { Button, Checkbox, Radio } from '@mui/material'
import { RefreshCw, Search, UserPlus, X } from 'lucide-react'
import { useEffect, useMemo, useState } from 'react'
import { useI18n } from '../../i18n/provider'
import { networkActions } from './cache'
import { useLookup, useNetworkResource } from './hooks'
import { KindLabel, ListSkeleton, PersonRow, ReachabilityChip, SectionTitle, StateMessage } from './Person'
import { personName, pickedPerson } from './personPresentation'
import { RelationBadge } from './RelationBadge'
import { networkErrorText, relationOfCandidate, tagKind, tagLabel } from './relation'
import type { PeoplePickerParams, PeoplePickerResult, PickedPerson } from './schemas'
import type { Contact, LookupCandidate } from './types'

const SECTION_ORDER = ['relation', 'zone', 'resolved'] as const

function restrictedCandidate(item: NonNullable<PeoplePickerParams['candidates']>[number], contacts: readonly Contact[]): LookupCandidate {
  const contact = contacts.find(entry => entry.did === item.did)
  return {
    did: item.did,
    kind: item.kind ?? 'user',
    section: 'relation',
    name: item.name || contact?.name,
    avatar: contact?.avatar,
    reachability: { state: 'unknown' },
    relation: contact
      ? { is_contact: true, friend_state: contact.friend_state, access_level: contact.access_level, tags: contact.tags }
      : { is_contact: false },
  }
}

/**
 * `embedded`: part of a host form (no footer); the selection is reported through `onChange`.
 * Otherwise the picker is a dialog body that ends with `onDone`.
 */
export function PeoplePicker({ params, onDone, onChange, embedded = false }: { params: PeoplePickerParams; onDone?: (result: PeoplePickerResult) => void; onChange?: (people: PickedPerson[]) => void; embedded?: boolean }) {
  const { t } = useI18n()
  const [input, setInput] = useState('')
  const [tag, setTag] = useState<string | null>(params.filter.tag ?? null)
  const [selected, setSelected] = useState<Map<string, PickedPerson>>(() => new Map(params.initialSelection.map(did => [did, { did, kind: 'unknown', name: personName({ did }), isContact: false }])))
  const [saving, setSaving] = useState<string | null>(null)
  const [saveError, setSaveError] = useState<string | null>(null)
  const restricted = params.candidates
  const lookup = useLookup(input, { kinds: params.filter.kinds, limit: 200, enabled: !restricted })
  const contacts = useNetworkResource('contacts')
  const single = params.mode === 'single'
  const excluded = useMemo(() => new Map(params.filter.exclude.map(item => [item.did, item.reason])), [params.filter.exclude])
  const limit = single ? 1 : params.limit

  const tags = useMemo(() => {
    const all = new Set<string>()
    for (const contact of contacts.data ?? []) for (const value of contact.tags ?? []) all.add(value)
    const order = { user: 0, app: 1, system: 2 } as const
    return [...all].sort((a, b) => order[tagKind(a)] - order[tagKind(b)] || a.localeCompare(b))
  }, [contacts.data])

  const candidates = useMemo(() => {
    const needle = input.trim().toLowerCase()
    const items = restricted
      ? restricted.map(item => restrictedCandidate(item, contacts.data ?? [])).filter(item => !needle || [item.name, item.did].some(value => value?.toLowerCase().includes(needle)))
      : lookup.result?.candidates ?? []
    return items.filter((item) => {
      if (params.filter.kinds?.length && !params.filter.kinds.includes(item.kind)) return false
      if (params.filter.excludeBlocked && item.relation.access_level === 'block') return false
      if (tag && !(item.relation.tags ?? []).includes(tag)) return false
      return true
    })
  }, [lookup.result, restricted, contacts.data, input, params.filter.kinds, params.filter.excludeBlocked, tag])

  const known = useMemo(() => new Map(candidates.map(item => [item.did, pickedPerson(item)])), [candidates])
  const chosen = useMemo(() => [...selected.values()].map(person => person.kind === 'unknown' ? known.get(person.did) ?? person : person), [selected, known])
  const chosenKey = chosen.map(person => `${person.did}:${person.name}`).join('|')
  useEffect(() => { onChange?.(chosen) }, [chosenKey]) // eslint-disable-line react-hooks/exhaustive-deps
  const count = chosen.length
  const full = limit !== undefined && count >= limit

  const toggle = (candidate: LookupCandidate) => {
    if (single && !embedded) {
      onDone?.({ action: 'selected', people: [pickedPerson(candidate)] })
      return
    }
    setSelected((current) => {
      if (single) return new Map([[candidate.did, pickedPerson(candidate)]])
      const next = new Map(current)
      if (next.has(candidate.did)) next.delete(candidate.did)
      else next.set(candidate.did, pickedPerson(candidate))
      return next
    })
  }

  const saveAndSelect = async (candidate: LookupCandidate) => {
    setSaving(candidate.did)
    setSaveError(null)
    try {
      await networkActions.saveContact(candidate.did, candidate.name)
      const person = { ...pickedPerson(candidate), isContact: true }
      if (single && !embedded) onDone?.({ action: 'selected', people: [person] })
      else setSelected(current => single ? new Map([[candidate.did, person]]) : new Map(current).set(candidate.did, person))
    } catch (error) {
      setSaveError(networkErrorText(t, error))
    } finally {
      setSaving(null)
    }
  }

  const renderRow = (candidate: LookupCandidate) => {
    const reason = excluded.get(candidate.did)
    const checked = selected.has(candidate.did)
    const needsSave = !candidate.relation.is_contact && candidate.section === 'resolved' && params.allowSaveContact
    const disabled = Boolean(reason) || (!checked && full && !single)
    const control = single
      ? <Radio size="small" checked={checked} disabled={disabled} tabIndex={-1} inputProps={{ 'aria-hidden': true }} />
      : <Checkbox size="small" checked={checked} disabled={disabled} tabIndex={-1} inputProps={{ 'aria-hidden': true }} />
    return (
      <PersonRow
        key={`${candidate.section}:${candidate.did}`}
        testId="people-picker-row"
        person={candidate}
        selected={checked}
        disabled={disabled}
        leading={needsSave ? undefined : control}
        onClick={needsSave || disabled ? undefined : () => toggle(candidate)}
        secondary={<><KindLabel kind={candidate.kind} />{candidate.reachability.zone ? ` · ${candidate.reachability.zone}` : ''}</>}
        trailing={
          reason ? <span className="text-xs text-[color:var(--cp-muted)]">{reason}</span>
            : needsSave ? (
              <>
                <ReachabilityChip reachability={candidate.reachability} />
                <Button size="small" variant="outlined" startIcon={<UserPlus size={13} />} disabled={saving !== null || (full && !single)} onClick={() => void saveAndSelect(candidate)}>
                  {t(saving === candidate.did ? 'network.saving' : 'network.picker.saveAndSelect')}
                </Button>
              </>
            )
            : candidate.kind !== 'group' ? <RelationBadge relation={relationOfCandidate(candidate)} compact /> : null
        }
      />
    )
  }

  return (
    <div className="flex min-h-0 flex-col gap-3" data-testid="people-picker">
      <label className="flex min-h-11 items-center gap-2 rounded-xl border border-[color:var(--cp-border)] bg-[color:var(--cp-bg)] px-3">
        <Search size={15} className="shrink-0 text-[color:var(--cp-muted)]" aria-hidden />
        <input
          type="search"
          value={input}
          autoFocus={!embedded}
          onChange={event => setInput(event.target.value)}
          placeholder={t('network.picker.placeholder')}
          aria-label={t('network.picker.search')}
          className="min-w-0 flex-1 border-none bg-transparent text-sm text-[color:var(--cp-text)] outline-none"
        />
      </label>

      {tags.length > 0 ? (
        <div className="flex flex-wrap gap-1.5" role="group" aria-label={t('network.picker.tags')}>
          {[null, ...tags].map(value => {
            const active = tag === value
            return (
              <button
                key={value ?? '*'}
                type="button"
                aria-pressed={active}
                onClick={() => setTag(value)}
                className="min-h-8 rounded-full border px-3 text-[12px]"
                style={active
                  ? { background: 'color-mix(in srgb, var(--cp-accent-soft) 22%, var(--cp-surface))', borderColor: 'color-mix(in srgb, var(--cp-accent) 26%, var(--cp-border))', color: 'var(--cp-text)' }
                  : { borderColor: 'var(--cp-border)', color: 'var(--cp-muted)' }}
              >
                {value === null ? t('network.picker.allTags') : tagLabel(t, value)}
              </button>
            )
          })}
        </div>
      ) : null}

      {!single && chosen.length > 0 ? (
        <ul className="flex flex-wrap gap-1.5" aria-label={t('network.picker.selected')} data-testid="people-picker-selected">
          {chosen.map(person => (
            <li key={person.did}>
              <button
                type="button"
                className="flex min-h-8 items-center gap-1 rounded-full py-1 pl-3 pr-2 text-[13px]"
                style={{ background: 'color-mix(in srgb, var(--cp-accent-soft) 20%, var(--cp-surface))', color: 'var(--cp-text)' }}
                aria-label={t('network.picker.unselect', undefined, { name: person.name })}
                onClick={() => setSelected((current) => {
                  const next = new Map(current)
                  next.delete(person.did)
                  return next
                })}
              >
                <span className="max-w-[10rem] truncate">{person.name}</span><X size={13} aria-hidden />
              </button>
            </li>
          ))}
        </ul>
      ) : null}

      <div className="desktop-scrollbar max-h-[38vh] min-h-[8rem] overflow-y-auto">
        {(restricted ? contacts.status === 'loading' && !contacts.data : lookup.status === 'loading' && !lookup.result) ? <ListSkeleton /> : null}
        {lookup.status === 'error' ? (
          <StateMessage tone="danger" action={<Button variant="text" size="small" startIcon={<RefreshCw size={13} />} onClick={lookup.retry}>{t('network.retry')}</Button>}>
            {networkErrorText(t, lookup.error)}
          </StateMessage>
        ) : null}
        {lookup.result?.error ? <StateMessage tone={lookup.result.error.code === 'not-found' ? 'muted' : 'danger'}>{t(`network.lookup.${lookup.result.error.code}`)}</StateMessage> : null}
        {(restricted ? !(contacts.status === 'loading' && !contacts.data) : lookup.status === 'ready' && !lookup.result?.error) && candidates.length === 0 ? (
          <StateMessage testId="people-picker-empty">{input.trim() || tag ? t('network.find.noMatches') : t('network.picker.noContacts')}</StateMessage>
        ) : null}
        {saveError ? <StateMessage tone="danger">{saveError}</StateMessage> : null}
        {SECTION_ORDER.map((section) => {
          const items = candidates.filter(item => item.section === section)
          if (items.length === 0) return null
          return (
            <section key={section}>
              <SectionTitle>{t(`network.section.${section}`)}</SectionTitle>
              <div className="space-y-0.5">{items.map(renderRow)}</div>
            </section>
          )
        })}
      </div>

      {embedded ? null : !single ? (
        <div className="flex flex-wrap items-center justify-end gap-2 border-t border-[color:color-mix(in_srgb,var(--cp-border)_74%,transparent)] pt-3">
          <span className="mr-auto text-xs text-[color:var(--cp-muted)]" aria-live="polite">
            {limit ? t('network.picker.countOf', undefined, { count, limit }) : t('network.picker.count', undefined, { count })}
          </span>
          <Button variant="text" onClick={() => onDone?.({ action: 'cancelled' })}>{t('network.cancel')}</Button>
          <Button variant="contained" disabled={count === 0} onClick={() => onDone?.({ action: 'selected', people: chosen })}>
            {params.confirmLabel ?? t('network.picker.confirm')}
          </Button>
        </div>
      ) : (
        <div className="flex justify-end">
          <Button variant="text" onClick={() => onDone?.({ action: 'cancelled' })}>{t('network.cancel')}</Button>
        </div>
      )}
    </div>
  )
}
