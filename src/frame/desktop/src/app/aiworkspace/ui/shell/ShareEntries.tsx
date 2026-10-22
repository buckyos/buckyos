/* Entry links of the share panel (hybrid SSO contract §8.2–§8.3, SSO-13): a manager offers ways in — Guest, an
 * account of this Zone, self-registration, DID sign-in — and the access a visitor gets. The link goes to the Zone's
 * entry page (`/sso/entry?service=aiworkspace&entry=<id>`), which signs the visitor in and joins the entry. Ways the
 * Zone switched off stay on the entry but are marked; the Control Panel says which (`auth.entry.describe`), and when
 * it cannot be asked the marks are simply missing. */

import { useCallback, useState } from 'react'
import type { EntryMethod } from 'buckyos'
import { describeEntry } from '../../../../api/sso'
import { unwrap, type AiwsClient } from '../../api/client'
import { describeError } from '../../api/session'
import type { ShareEntry } from '../../api/types'
import { entryUrl } from '../../links'
import { useLoad } from '../../state/hooks'
import { useAiwsText } from '../../useAiwsText'
import { PRESETS } from '../sources/presets'

type Methods = ShareEntry['methods']

const METHODS: readonly { key: EntryMethod; label: string }[] = [
  { key: 'guest', label: 'aiworkspace.shell.share.method.guest' },
  { key: 'password', label: 'aiworkspace.shell.share.method.password' },
  { key: 'self_register', label: 'aiworkspace.shell.share.method.selfRegister' },
  { key: 'did_login', label: 'aiworkspace.shell.share.method.didLogin' },
]

const ENTRY_PRESETS = PRESETS.filter((p) => !p.caps.includes('manage'))

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
  const { t } = useAiwsText()
  const [tick, setTick] = useState(0)
  const [methods, setMethods] = useState<Methods>({ guest: true, password: true, self_register: false, did_login: false })
  const [preset, setPreset] = useState('commenter')
  const [busy, setBusy] = useState(false)
  const [message, setMessage] = useState<{ kind: 'info' | 'error'; text: string } | null>(null)
  const load = useCallback(async () => {
    const entries = unwrap(await client.wsListEntries({ workspace_id: workspaceId })).entries.filter((e) => e.state === 'active')
    return { entries, disabled: await zoneDisabled(entries) }
  }, [client, workspaceId])
  const listed = useLoad(load, tick)
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
  const create = () => run('aiworkspace.shell.share.entryCreateFailed', async () => {
    const caps = ENTRY_PRESETS.find((p) => p.id === preset)?.caps ?? ['read']
    const { entry } = unwrap(await client.wsCreateEntry({ workspace_id: workspaceId }, { capabilities: caps, methods }))
    return (await copyLink(entryUrl(entry.entry_id))) ? t('aiworkspace.shell.share.entryCreated') : t('aiworkspace.shell.share.entryCreatedManual')
  })
  const copyEntry = (entry: ShareEntry) => {
    void copyLink(entryUrl(entry.entry_id)).then((copied) => setMessage({ kind: 'info', text: copied ? t('aiworkspace.shell.share.entryCopied') : t('aiworkspace.shell.share.copyManually') }))
  }
  const close = (entry: ShareEntry) => run('aiworkspace.shell.share.entryCloseFailed', async () => {
    unwrap(await client.wsCloseEntry({ workspace_id: workspaceId }, entry.entry_id))
    return null
  })
  const presetLabel = (caps: string[]) => ENTRY_PRESETS.find((p) => p.caps.length === caps.length && p.caps.every((c) => caps.includes(c)))?.label ?? caps.join(', ')
  return (
    <div className="aiws-share-entries" data-testid="aiws-share-entries">
      <div className="aiws-muted">{t('aiworkspace.shell.share.entriesHint')}</div>
      <form className="aiws-share-entry-form" data-testid="aiws-share-entry-form" onSubmit={(event) => { event.preventDefault(); if (!busy && offered) create() }}>
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
          <label>{t('aiworkspace.shell.share.entryAccess')}
            <select value={preset} disabled={busy} data-testid="aiws-share-entry-preset" onChange={(event) => setPreset(event.target.value)}>
              {ENTRY_PRESETS.map((p) => <option key={p.id} value={p.id}>{p.label}</option>)}
            </select>
          </label>
          <button type="submit" className="is-primary" data-testid="aiws-share-entry-create" disabled={busy || !offered}>{t('aiworkspace.shell.share.entryCreate')}</button>
        </div>
      </form>
      {message && <div className={message.kind === 'error' ? 'aiws-error' : 'aiws-muted'} role={message.kind === 'error' ? 'alert' : 'status'} data-testid="aiws-share-entry-message">{message.text}</div>}
      <div className="aiws-share-entry-list">
        <div className="aiws-panel-title">{t('aiworkspace.shell.share.entryExisting')}</div>
        {listed.error && <div className="aiws-error" role="alert">{listed.error}</div>}
        {listed.data?.entries.length === 0 && <div className="aiws-muted" data-testid="aiws-share-entry-none">{t('aiworkspace.shell.share.entryNone')}</div>}
        {listed.data?.entries.map((entry) => {
          const off = listed.data?.disabled[entry.entry_id] ?? []
          return (
            <div key={entry.entry_id} className="aiws-share-entry" data-testid="aiws-share-entry" data-entry-id={entry.entry_id}>
              <div className="aiws-share-entry-header">
                <div className="aiws-share-entry-methods">
                  {METHODS.filter((m) => entry.methods[m.key]).map((m) => (
                    <span key={m.key} className={`aiws-chip${off.includes(m.key) ? ' aiws-chip-warn' : ''}`} data-testid={`aiws-share-entry-method-${m.key}`} data-zone-disabled={off.includes(m.key) ? 'true' : undefined}>
                      {t(m.label)}{off.includes(m.key) && <> · {t('aiworkspace.shell.share.zoneDisabled')}</>}
                    </span>
                  ))}
                  <span className="aiws-muted">{presetLabel(entry.capabilities)}</span>
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
