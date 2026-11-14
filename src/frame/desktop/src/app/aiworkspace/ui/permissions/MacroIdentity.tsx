/* The identity a wish's macro runs with (权限需求 §4.5, R28; §8.4): by default its triggerer's permissions; bound to a
 * logical group, their union (the default when binding) or intersection with the group's. For the "权限" tab of a macro.
 * Binding is a manager's (`acl.bind_macro`) and goes into the operation record; whoever may change the macro holds the
 * group's permissions through it (Q10), so a group holding `manage` cannot be bound. This phase runs no macro with
 * more than its triggerer holds (Q12): a union adds nothing until a trusted executor exists.
 *
 * The service has no read of one wish's binding; it is taken from what the permission queries show (the by-entity
 * query of the wish lists it when the group has entries along the wish's path) and else from the operation record. */

import { useMemo, useState } from 'react'
import { describeError } from '../../api/session'
import type { AclGroup } from '../../api/types'
import { useAiwsText } from '../../useAiwsText'
import { useAcl, useAclRead, useWorkspaceCaps } from './aclData'
import { useLabels } from './labels'
import { bindingFromAudit, type Binding } from './logic'
import { groupsHoldingManage } from './model'

type Message = { kind: 'info' | 'error'; text: string } | null

export function MacroIdentity({ wishId }: { wishId: string }) {
  const { t } = useAiwsText()
  const labels = useLabels()
  const { api } = useAcl()
  const { capabilities } = useWorkspaceCaps()
  const manage = capabilities.includes('manage')
  const all = useAclRead(manage ? 'all' : null, api.all)
  const audit = useAclRead(manage ? 'audit' : null, () => api.audit())
  const view = useAclRead(manage ? `entity:${wishId}` : null, () => api.entity(wishId))
  const [message, setMessage] = useState<Message>(null)
  const known = useMemo<Binding | undefined>(() => {
    const listed = view.data?.macros.find((m) => m.wish.entity_id === wishId)
    if (listed) return { group_id: listed.group_id, mode: listed.mode }
    if (!audit.data) return undefined
    const b = bindingFromAudit(audit.data, wishId)
    return b && labels.groups.some((g) => g.group_id === b.group_id) ? b : b === undefined ? undefined : null
  }, [view.data, audit.data, wishId, labels.groups])
  if (!manage) return <div className="aiws-muted" data-testid="aiws-perm-macro-identity" data-manage="false">{t('aiworkspace.perm.macro.managersOnly')}</div>
  const holding = groupsHoldingManage(all.data?.entries ?? [])
  const choices = labels.groups.filter((g) => !holding.has(g.subject))
  return (
    <section className="aiws-perm-section aiws-perm-macro-identity" data-testid="aiws-perm-macro-identity" data-group={known?.group_id ?? ''} data-mode={known?.mode ?? ''}>
      <div className="aiws-perm-subhead">{t('aiworkspace.perm.macro.title')}</div>
      <p className="aiws-muted">{t('aiworkspace.perm.macro.intro')}</p>
      {known === undefined && audit.data && <div className="aiws-muted" data-testid="aiws-perm-macro-unknown">{t('aiworkspace.perm.macro.unknown')}</div>}
      <MacroForm key={known === undefined ? '?' : `${known?.group_id ?? ''}:${known?.mode ?? ''}`} wishId={wishId} known={known ?? null} choices={choices} onMessage={setMessage} />
      {message && <div className={message.kind === 'error' ? 'aiws-error' : 'aiws-muted'} role={message.kind === 'error' ? 'alert' : 'status'} data-testid="aiws-perm-macro-message">{message.text}</div>}
    </section>
  )
}

/** The binding's choices, starting from the one in force (re-mounted when it changes). */
function MacroForm({ wishId, known, choices, onMessage }: { wishId: string; known: Binding; choices: AclGroup[]; onMessage: (message: Message) => void }) {
  const { t } = useAiwsText()
  const { api, online } = useAcl()
  const [group, setGroup] = useState(known?.group_id ?? '')
  const [mode, setMode] = useState<'union' | 'intersection'>(known?.mode ?? 'union')
  const [busy, setBusy] = useState(false)
  const dirty = (known?.group_id ?? '') !== group || (group !== '' && (known?.mode ?? 'union') !== mode)
  const save = async () => {
    setBusy(true)
    onMessage(null)
    try {
      await api.bindMacro(wishId, group || null, mode)
      onMessage({ kind: 'info', text: group ? t('aiworkspace.perm.macro.bound') : t('aiworkspace.perm.macro.unboundDone') })
    } catch (error) { onMessage({ kind: 'error', text: t('aiworkspace.perm.macro.failed', { error: describeError(error) }) }) } finally { setBusy(false) }
  }
  return (
    <>
      <label className="aiws-perm-field"><span>{t('aiworkspace.perm.macro.group')}</span>
        <select data-testid="aiws-perm-macro-group" value={group} disabled={busy} onChange={(event) => setGroup(event.target.value)}>
          <option value="">{t('aiworkspace.perm.macro.none')}</option>
          {choices.map((g) => <option key={g.group_id} value={g.group_id}>{g.title || g.group_id} (@{g.group_id})</option>)}
        </select>
      </label>
      {group && (
        <fieldset className="aiws-perm-radios" disabled={busy}>
          <legend>{t('aiworkspace.perm.macro.relation')}</legend>
          {(['union', 'intersection'] as const).map((m) => (
            <label key={m}><input type="radio" name={`macro-mode-${wishId}`} data-testid={`aiws-perm-macro-mode-${m}`} checked={mode === m} onChange={() => setMode(m)} />{t(`aiworkspace.perm.macro.mode.${m}`)}<span className="aiws-muted"> {t(`aiworkspace.perm.macro.modeHint.${m}`)}</span></label>
          ))}
        </fieldset>
      )}
      <div className="aiws-warning">{t('aiworkspace.perm.macro.risk')}</div>
      <div className="aiws-dialog-actions">
        <button type="button" className="is-primary" data-testid="aiws-perm-macro-save" disabled={!online || busy || !dirty} onClick={() => { void save() }}>{t('aiworkspace.common.save')}</button>
      </div>
    </>
  )
}
