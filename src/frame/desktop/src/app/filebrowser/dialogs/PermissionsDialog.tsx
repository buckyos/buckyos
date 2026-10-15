import { Button, Checkbox, Dialog, DialogActions, DialogContent, DialogTitle, FormControlLabel, IconButton } from '@mui/material'
import { Globe, Lock, Trash2, User, Users } from 'lucide-react'
import { useEffect, useMemo, useState } from 'react'
import { useI18n } from '../../../i18n/provider'
import { PUBLIC_PRINCIPAL, ZONE_USERS_PRINCIPAL, permissionSource } from '../data/permissions'
import type { AclGrant, AclView, GrantLevel, PrincipalOption } from '../data/permissions'
import { toUiError } from '../data/state'
import { levelLabel, principalLabel } from './permissionLabels'

export interface PermissionsRequest {
  /** DFS path of the file or folder. */
  path: string
  name: string
  onSaved?: () => void
}

const GRANT_LEVELS: GrantLevel[] = ['read', 'write', 'manage']

function PrincipalIcon({ principal }: { principal: string }) {
  if (principal === PUBLIC_PRINCIPAL) return <Globe size={16} aria-hidden />
  if (principal === ZONE_USERS_PRINCIPAL) return <Users size={16} aria-hidden />
  return <User size={16} aria-hidden />
}

const selectClass = 'rounded border bg-[color:var(--cp-surface)] p-1.5 text-sm'

/** Common ACL editor: built-in and inherited rows are read-only, the item's own rows are editable. */
export function PermissionsDialog({ request, onClose }: { request: PermissionsRequest; onClose: () => void }) {
  const { t } = useI18n()
  const source = permissionSource()
  const [acl, setAcl] = useState<AclView | null>(null)
  const [draft, setDraft] = useState<{ inherit: boolean; entries: AclGrant[] } | null>(null)
  const [users, setUsers] = useState<PrincipalOption[]>([])
  const [error, setError] = useState<string | null>(null)
  const [saving, setSaving] = useState(false)
  const [reload, setReload] = useState(0)
  const [adding, setAdding] = useState<{ text: string; level: GrantLevel }>({ text: '', level: 'read' })

  useEffect(() => {
    if (!source) return
    let alive = true
    source.getAcl(request.path).then((view) => {
      if (!alive) return
      setAcl(view)
      setDraft({ inherit: view.inherit, entries: view.entries.map((entry) => ({ ...entry })) })
      setError(null)
    }).catch((err: unknown) => { if (alive) setError(toUiError(err).fallback) })
    source.listUsers().then((list) => { if (alive) setUsers(list) }).catch(() => {})
    return () => { alive = false }
  }, [source, request.path, reload])

  const dirty = useMemo(() => !!acl && !!draft && (draft.inherit !== acl.inherit || JSON.stringify(draft.entries) !== JSON.stringify(acl.entries)), [acl, draft])
  const listed = new Set(draft?.entries.map((entry) => entry.principal))
  const candidates: PrincipalOption[] = [
    ...users.filter((user) => !listed.has(user.principal) && !acl?.builtin.some((rule) => rule.principal === user.principal)),
    ...[ZONE_USERS_PRINCIPAL, PUBLIC_PRINCIPAL].filter((principal) => !listed.has(principal)).map((principal) => ({ principal, label: principalLabel(t, principal, users) })),
  ]
  // Free text falls back to a user id, so access can be granted even when the user directory is unavailable.
  const addingPrincipal = (() => {
    const text = adding.text.trim()
    if (!text) return null
    const lower = text.toLowerCase()
    const known = candidates.find((option) => [option.principal, option.label, option.detail].some((value) => value?.toLowerCase() === lower))
    if (known) return known.principal
    const id = text.replace(/^user:/, '')
    return /^[A-Za-z0-9._-]{1,128}$/.test(id) && !listed.has(`user:${id}`) ? `user:${id}` : null
  })()
  const hasPublic = draft?.entries.some((entry) => entry.principal === PUBLIC_PRINCIPAL) || (draft?.inherit && acl?.inherited.some((entry) => entry.principal === PUBLIC_PRINCIPAL))

  const update = (entries: AclGrant[]) => draft && setDraft({ ...draft, entries })
  const add = () => {
    if (!draft || !addingPrincipal) return
    update([...draft.entries, { principal: addingPrincipal, level: addingPrincipal === PUBLIC_PRINCIPAL ? 'read' : adding.level }])
    setAdding({ text: '', level: 'read' })
  }
  const save = async () => {
    if (!source || !acl || !draft) return
    setSaving(true)
    setError(null)
    try {
      await source.setAcl(request.path, draft, acl.revision)
      request.onSaved?.()
      onClose()
    } catch (err) {
      setError(toUiError(err).fallback)
      setReload((n) => n + 1)
    } finally {
      setSaving(false)
    }
  }

  const levelOptions = (principal: string) => (principal === PUBLIC_PRINCIPAL ? ['read' as GrantLevel] : GRANT_LEVELS)
  const loading = !acl && !error

  return <Dialog open onClose={onClose} maxWidth="sm" fullWidth data-testid="permissions-dialog">
    <DialogTitle>{t('filebrowser.permissions.title', 'Permissions')} · {request.name}</DialogTitle>
    <DialogContent>
      <p className="break-all font-mono text-xs text-[color:var(--cp-muted)]">{request.path}</p>
      {!source && <p className="mt-3">{t('filebrowser.permissions.unavailable', 'This location does not support permissions.')}</p>}
      {loading && source && <p className="mt-3 text-sm" data-testid="permissions-loading">{t('filebrowser.permissions.loading', 'Loading permissions…')}</p>}
      {acl && <p className="mt-2 text-sm" data-testid="permissions-own-access">
        {t('filebrowser.permissions.yourAccess', 'Your access')}: <strong>{levelLabel(t, acl.accessLevel)}</strong>
      </p>}
      {acl && !acl.canManage && <p className="mt-3 text-sm">{t('filebrowser.permissions.cannotManage', 'Only people with Full control can see and change who has access.')}</p>}
      {acl?.canManage && draft && <>
        <FormControlLabel
          className="mt-2"
          control={<Checkbox checked={draft.inherit} onChange={(event) => setDraft({ ...draft, inherit: event.target.checked })} inputProps={{ 'aria-label': t('filebrowser.permissions.inherit', 'Inherit permissions from the parent folder') }} data-testid="acl-inherit" />}
          label={t('filebrowser.permissions.inherit', 'Inherit permissions from the parent folder')}
        />
        {!draft.inherit && <p className="text-xs text-[color:var(--cp-muted)]">{t('filebrowser.permissions.inheritOff', 'Only the entries below apply here and in subfolders that inherit from this item.')}</p>}
        <h3 className="mb-1 mt-3 text-sm font-semibold">{t('filebrowser.permissions.whoHasAccess', 'Who has access')}</h3>
        <table className="w-full text-sm" data-testid="acl-table">
          <tbody>
            {acl.builtin.map((rule) => <tr key={`builtin-${rule.principal}-${rule.reason}`} className="border-t border-[color:var(--cp-border)]" data-testid={`acl-builtin-${rule.principal}`}>
              <td className="py-2 pr-2"><span className="inline-flex items-center gap-2"><PrincipalIcon principal={rule.principal} />{principalLabel(t, rule.principal, users)}</span></td>
              <td className="py-2 pr-2">{levelLabel(t, rule.level)}</td>
              <td className="py-2 text-xs text-[color:var(--cp-muted)]"><span className="inline-flex items-center gap-1"><Lock size={12} aria-hidden />{rule.reason === 'zone_owner' ? t('filebrowser.permissions.zoneOwner', 'Zone owner') : t('filebrowser.permissions.homeOwner', 'Owner of this home folder')}</span></td>
              <td />
            </tr>)}
            {draft.inherit && acl.inherited.map((entry, index) => <tr key={`inherited-${index}`} className="border-t border-[color:var(--cp-border)] text-[color:var(--cp-muted)]" data-testid={`acl-inherited-${entry.principal}`}>
              <td className="py-2 pr-2"><span className="inline-flex items-center gap-2"><PrincipalIcon principal={entry.principal} />{principalLabel(t, entry.principal, users)}</span></td>
              <td className="py-2 pr-2">{levelLabel(t, entry.level)}</td>
              <td className="break-all py-2 text-xs">{t('filebrowser.permissions.inheritedFrom', 'Inherited from {{path}}', { path: entry.from })}</td>
              <td />
            </tr>)}
            {draft.entries.map((entry, index) => <tr key={entry.principal} className="border-t border-[color:var(--cp-border)]" data-testid={`acl-row-${entry.principal}`}>
              <td className="py-2 pr-2"><span className="inline-flex items-center gap-2"><PrincipalIcon principal={entry.principal} />{principalLabel(t, entry.principal, users)}</span></td>
              <td className="py-2 pr-2">
                <select
                  aria-label={t('filebrowser.permissions.levelFor', 'Access for {{name}}', { name: principalLabel(t, entry.principal, users) })}
                  className={selectClass}
                  value={entry.level}
                  data-testid={`acl-level-${entry.principal}`}
                  onChange={(event) => update(draft.entries.map((item, i) => (i === index ? { ...item, level: event.target.value as GrantLevel } : item)))}
                >
                  {levelOptions(entry.principal).map((level) => <option key={level} value={level}>{levelLabel(t, level)}</option>)}
                </select>
              </td>
              <td />
              <td className="w-10 py-1 text-right">
                <IconButton size="small" aria-label={t('filebrowser.permissions.remove', 'Remove {{name}}', { name: principalLabel(t, entry.principal, users) })} data-testid={`acl-remove-${entry.principal}`} onClick={() => update(draft.entries.filter((_, i) => i !== index))}><Trash2 size={16} /></IconButton>
              </td>
            </tr>)}
            {acl.builtin.length + draft.entries.length + (draft.inherit ? acl.inherited.length : 0) === 0 && <tr><td colSpan={4} className="py-2 text-[color:var(--cp-muted)]">{t('filebrowser.permissions.nobody', 'Nobody else has access.')}</td></tr>}
          </tbody>
        </table>
        <div className="mt-3 flex flex-wrap items-center gap-2">
          <input
            aria-label={t('filebrowser.permissions.addWho', 'Add a person or group')}
            placeholder={t('filebrowser.permissions.choose', 'User ID, All zone users or Public…')}
            className={`${selectClass} min-w-0 flex-1 bg-transparent`}
            list="acl-principal-options"
            value={adding.text}
            data-testid="acl-add-principal"
            onChange={(event) => setAdding({ ...adding, text: event.target.value })}
            onKeyDown={(event) => { if (event.key === 'Enter') { event.preventDefault(); add() } }}
          />
          <datalist id="acl-principal-options">
            {candidates.map((option) => <option key={option.principal} value={option.detail ?? option.label}>{option.label}</option>)}
          </datalist>
          <select aria-label={t('filebrowser.permissions.addLevel', 'Access to grant')} className={selectClass} value={addingPrincipal === PUBLIC_PRINCIPAL ? 'read' : adding.level} data-testid="acl-add-level" disabled={addingPrincipal === PUBLIC_PRINCIPAL} onChange={(event) => setAdding({ ...adding, level: event.target.value as GrantLevel })}>
            {levelOptions(addingPrincipal ?? '').map((level) => <option key={level} value={level}>{levelLabel(t, level)}</option>)}
          </select>
          <Button size="small" variant="outlined" disabled={!addingPrincipal} onClick={add} data-testid="acl-add">{t('filebrowser.permissions.add', 'Add')}</Button>
        </div>
        {hasPublic && <p className="mt-3 rounded border border-[color:var(--cp-warning)] p-2 text-xs" data-testid="acl-public-warning">{t('filebrowser.permissions.publicWarning', 'Anyone can open this without signing in, including anyone who gets a link to it.')}</p>}
      </>}
      {error && <p className="mt-3 text-sm text-[color:var(--cp-danger)]" role="alert">{error}</p>}
    </DialogContent>
    <DialogActions>
      {acl?.canManage
        ? <><Button onClick={onClose}>{t('common.cancel', 'Cancel')}</Button><Button variant="contained" disabled={!dirty || saving} onClick={() => void save()} data-testid="acl-save">{saving ? t('filebrowser.permissions.saving', 'Saving…') : t('common.save', 'Save')}</Button></>
        : <Button onClick={onClose}>{t('common.close', 'Close')}</Button>}
    </DialogActions>
  </Dialog>
}
