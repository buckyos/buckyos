/* Entry links of the share panel (hybrid SSO contract §8.2–§8.3, SSO-13; 权限需求 R8): a manager offers ways in — Guest, an
 * account of this Zone, self-registration, DID sign-in — and the logical group whoever comes through joins (an existing
 * group, or one made from a preset), with an optional expiry and a limit on how many may join (UI 框架 PRD §8.5). The
 * link goes to the Zone's entry page (`/sso/entry?service=aiworkspace&entry=<id>`), which signs the visitor in and joins
 * the entry. Ways the Zone switched off stay on the entry but are marked; the Control Panel says which
 * (`auth.entry.describe`), and when it cannot be asked the marks are simply missing. */

import { useCallback, useState } from 'react'
import type { EntryMethod } from 'buckyos'
import { describeEntry } from '../../../../api/sso'
import { unwrap, type AiwsClient } from '../../api/client'
import { describeError } from '../../api/session'
import type { Capability, ShareEntry } from '../../api/types'
import { entryUrl } from '../../links'
import { useLoad } from '../../state/hooks'
import { useAiwsText } from '../../useAiwsText'
import { useAcl, useAclRead } from '../permissions/aclData'
import { CAPS } from '../permissions/model'
import { PRESETS } from '../sources/presets'

type Methods = ShareEntry['methods']

const METHODS: readonly { key: EntryMethod; label: string }[] = [
  { key: 'guest', label: 'aiworkspace.shell.share.method.guest' },
  { key: 'password', label: 'aiworkspace.shell.share.method.password' },
  { key: 'self_register', label: 'aiworkspace.shell.share.method.selfRegister' },
  { key: 'did_login', label: 'aiworkspace.shell.share.method.didLogin' },
]

/** An entry's group never holds `manage` or `run` (R8, R18). */
const ENTRY_PRESETS = PRESETS.filter((p) => !p.caps.includes('manage') && !p.caps.includes('run'))
const EXPIRY_DAYS = [1, 7, 30] as const

/** The group a preset's entries lead into: `share-<caps>`, allowed those capabilities on `/` when it is made. */
function presetGroup(caps: readonly Capability[]): string {
  return `share-${CAPS.filter((c) => caps.includes(c)).join('-')}`
}

/** The ways in the Zone switched off, per entry; an entry the Control Panel could not describe has none listed. */
async function zoneDisabled(entries: ShareEntry[]): Promise<Record<string, EntryMethod[]>> {
  const out: Record<string, EntryMethod[]> = {}
  await Promise.all(entries.map(async (entry) => {
    const { data } = await describeEntry('aiworkspace', entry.entry_id)
    if (data?.state === 'active') out[entry.entry_id] = data.zone_disabled ?? []
  }))
  return out
}

/** Copy `link`; false when the browser refuses (the link stays visible to copy by hand). */
async function copyLink(link: string): Promise<boolean> {
  try { await navigator.clipboard.writeText(link); return true } catch { return false }
}

export function ShareEntries({ client, workspaceId }: { client: AiwsClient; workspaceId: string }) {
  const { t, dateTime } = useAiwsText()
  const { api } = useAcl()
  const [tick, setTick] = useState(0)
  const [methods, setMethods] = useState<Methods>({ guest: true, password: true, self_register: false, did_login: false })
  const [target, setTarget] = useState('commenter')
  const [expiry, setExpiry] = useState('')
  const [maxJoins, setMaxJoins] = useState('')
  const [removeMembers, setRemoveMembers] = useState(false)
  const [busy, setBusy] = useState(false)
  const [message, setMessage] = useState<{ kind: 'info' | 'error'; text: string } | null>(null)
  const load = useCallback(async () => {
    const entries = unwrap(await client.wsListEntries({ workspace_id: workspaceId })).entries.filter((e) => e.state === 'active')
    return { entries, disabled: await zoneDisabled(entries) }
  }, [client, workspaceId])
  const listed = useLoad(load, tick)
  const groups = useAclRead('groups', api.groups)
  const all = useAclRead('all', api.all)
  // a group allowed `manage` or `run` anywhere cannot be an entry's (R8)
  const unfit = new Set((all.data?.entries ?? []).filter((e) => e.allow.includes('manage') || e.allow.includes('run')).map((e) => e.subject))
  const choices = (groups.data?.groups ?? []).filter((g) => !unfit.has(g.subject))
  const titleOf = (groupId: string) => groups.data?.groups.find((g) => g.group_id === groupId)?.title || groupId
  const run = (failedKey: string, work: () => Promise<string | null>) => {
    if (busy) return
    setBusy(true)
    setMessage(null)
    work()
      .then((text) => { if (text) setMessage({ kind: 'info', text }) })
      .catch((error: unknown) => setMessage({ kind: 'error', text: t(failedKey, { error: describeError(error) }) }))
      .finally(() => { setBusy(false); setTick((n) => n + 1) })
  }
  const offered = METHODS.some((m) => methods[m.key])
  const max = maxJoins.trim() === '' ? null : Number(maxJoins)
  const maxOk = max === null || (Number.isInteger(max) && max >= 1 && max <= 100_000)
  const create = () => run('aiworkspace.shell.share.entryCreateFailed', async () => {
    let groupId: string
    if (target.startsWith('@')) groupId = target.slice(1)
    else {
      // a preset's group is made once; its entries are then the group's own, edited in the permission editor
      const preset = ENTRY_PRESETS.find((p) => p.id === target) ?? ENTRY_PRESETS[0]
      groupId = presetGroup(preset.caps)
      if (!(groups.data?.groups ?? []).some((g) => g.group_id === groupId)) {
        await api.setGroup(groupId, t('aiworkspace.perm.share.presetGroupTitle', { label: preset.label }))
        await api.set([{ subject: `@${groupId}`, resource: '/', allow: preset.caps }])
      }
    }
    const days = Number(expiry)
    const expires_at = expiry ? Math.floor(Date.now() / 1000) + days * 86400 : undefined
    const entry = unwrap(await client.wsCreateEntry({ workspace_id: workspaceId }, { group_id: groupId, methods, ...(expires_at ? { expires_at } : {}), ...(max !== null ? { max_joins: max } : {}) })).entry
    return (await copyLink(entryUrl(entry.entry_id))) ? t('aiworkspace.shell.share.entryCreated') : t('aiworkspace.shell.share.entryCreatedManual')
  })
  const copyEntry = (entry: ShareEntry) => {
    void copyLink(entryUrl(entry.entry_id)).then((copied) => setMessage({ kind: 'info', text: copied ? t('aiworkspace.shell.share.entryCopied') : t('aiworkspace.shell.share.copyManually') }))
  }
  const close = (entry: ShareEntry) => run('aiworkspace.shell.share.entryCloseFailed', async () => {
    const closed = unwrap(await client.wsCloseEntry({ workspace_id: workspaceId }, entry.entry_id, removeMembers))
    return removeMembers ? t('aiworkspace.perm.share.closedRemoved', { count: closed.removed_members }) : null
  })
  return (
    <div className="aiws-share-entries" data-testid="aiws-share-entries">
      <div className="aiws-muted">{t('aiworkspace.shell.share.entriesHint')}</div>
      <form className="aiws-share-entry-form" data-testid="aiws-share-entry-form" onSubmit={(event) => { event.preventDefault(); if (!busy && offered && maxOk) create() }}>
        <fieldset className="aiws-share-entry-options" disabled={busy}>
          <legend>{t('aiworkspace.shell.share.entryMethods')}</legend>
          {METHODS.map((m) => (
            <label key={m.key} className="aiws-share-entry-option">
              <input type="checkbox" checked={methods[m.key]} data-testid={`aiws-share-entry-${m.key}`} onChange={(event) => setMethods({ ...methods, [m.key]: event.target.checked })} />
              <span>{t(m.label)}</span>
            </label>
          ))}
        </fieldset>
        {!offered && <div className="aiws-muted">{t('aiworkspace.shell.share.entryMethodRequired')}</div>}
        <div className="aiws-share-entry-create-row">
          <label>{t('aiworkspace.perm.share.target')}
            <select value={target} disabled={busy} data-testid="aiws-share-entry-preset" onChange={(event) => setTarget(event.target.value)}>
              <optgroup label={t('aiworkspace.perm.share.presetGroups')}>
                {ENTRY_PRESETS.map((p) => <option key={p.id} value={p.id}>{p.label}</option>)}
              </optgroup>
              {choices.length > 0 && (
                <optgroup label={t('aiworkspace.perm.logicalGroups')}>
                  {choices.map((g) => <option key={g.subject} value={g.subject}>{g.title || g.group_id} (@{g.group_id})</option>)}
                </optgroup>
              )}
            </select>
          </label>
          <label>{t('aiworkspace.perm.share.expires')}
            <select value={expiry} disabled={busy} data-testid="aiws-share-entry-expires" onChange={(event) => setExpiry(event.target.value)}>
              <option value="">{t('aiworkspace.perm.share.never')}</option>
              {EXPIRY_DAYS.map((d) => <option key={d} value={String(d)}>{t('aiworkspace.perm.share.days', { count: d })}</option>)}
            </select>
          </label>
          <label>{t('aiworkspace.perm.share.maxJoins')}
            <input type="number" min={1} max={100000} inputMode="numeric" value={maxJoins} disabled={busy} placeholder={t('aiworkspace.perm.share.unlimited')} data-testid="aiws-share-entry-max" aria-invalid={maxOk ? undefined : true} onChange={(event) => setMaxJoins(event.target.value)} />
          </label>
          <button type="submit" className="is-primary" data-testid="aiws-share-entry-create" disabled={busy || !offered || !maxOk}>{t('aiworkspace.shell.share.entryCreate')}</button>
        </div>
        <div className="aiws-muted">{t('aiworkspace.perm.share.targetHint')}</div>
      </form>
      {message && <div className={message.kind === 'error' ? 'aiws-error' : 'aiws-muted'} role={message.kind === 'error' ? 'alert' : 'status'} data-testid="aiws-share-entry-message">{message.text}</div>}
      <div className="aiws-share-entry-list">
        <div className="aiws-panel-title">{t('aiworkspace.shell.share.entryExisting')}</div>
        {listed.error && <div className="aiws-error" role="alert">{listed.error}</div>}
        {listed.data?.entries.length === 0 && <div className="aiws-muted" data-testid="aiws-share-entry-none">{t('aiworkspace.shell.share.entryNone')}</div>}
        {(listed.data?.entries.length ?? 0) > 0 && (
          <label className="aiws-share-entry-option"><input type="checkbox" checked={removeMembers} data-testid="aiws-share-entry-close-remove" onChange={(event) => setRemoveMembers(event.target.checked)} /><span>{t('aiworkspace.perm.share.closeRemoves')}</span></label>
        )}
        {listed.data?.entries.map((entry) => {
          const off = listed.data?.disabled[entry.entry_id] ?? []
          return (
            <div key={entry.entry_id} className="aiws-share-entry" data-testid="aiws-share-entry" data-entry-id={entry.entry_id} data-group={entry.group_id}>
              <div className="aiws-share-entry-header">
                <div className="aiws-share-entry-methods">
                  {METHODS.filter((m) => entry.methods[m.key]).map((m) => (
                    <span key={m.key} className={`aiws-chip${off.includes(m.key) ? ' aiws-chip-warn' : ''}`} data-testid={`aiws-share-entry-method-${m.key}`} data-zone-disabled={off.includes(m.key) ? 'true' : undefined}>
                      {t(m.label)}{off.includes(m.key) && <> · {t('aiworkspace.shell.share.zoneDisabled')}</>}
                    </span>
                  ))}
                  <span className="aiws-muted" data-testid="aiws-share-entry-group">{t('aiworkspace.perm.share.joins', { group: titleOf(entry.group_id) })}</span>
                  <span className="aiws-muted" data-testid="aiws-share-entry-limits">
                    {entry.expires_at ? t('aiworkspace.perm.share.until', { at: dateTime(entry.expires_at * 1000) }) : t('aiworkspace.perm.share.noExpiry')}
                    {' · '}{entry.max_joins ? t('aiworkspace.perm.share.joinedOf', { joined: entry.joined ?? 0, max: entry.max_joins }) : t('aiworkspace.perm.share.joined', { joined: entry.joined ?? 0 })}
                  </span>
                </div>
                <button type="button" data-testid="aiws-share-entry-close" disabled={busy} onClick={() => close(entry)}>{t('aiworkspace.shell.share.entryClose')}</button>
              </div>
              <div className="aiws-share-link-row">
                <input readOnly aria-label={t('aiworkspace.shell.share.entryLink')} value={entryUrl(entry.entry_id)} data-testid="aiws-share-entry-link" onFocus={(event) => event.target.select()} />
                <button type="button" data-testid="aiws-share-entry-copy" onClick={() => copyEntry(entry)}>{t('aiworkspace.shell.share.entryCopy')}</button>
              </div>
            </div>
          )
        })}
      </div>
    </div>
  )
}
