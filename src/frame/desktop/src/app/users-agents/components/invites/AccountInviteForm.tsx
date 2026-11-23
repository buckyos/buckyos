import { useState, type FormEvent } from 'react'
import { Alert, Button, Checkbox, FormControlLabel, TextField } from '@mui/material'
import { createUserInvite } from '../../../../api/user_mgr'
import { useI18n } from '../../../../i18n/provider'
import { useUserGroups, useUsersAgentsStore } from '../../hooks/use-users-agents-store'

export function AccountInviteForm({ onClose, initialDid = '', initialEmail = '', existingUser }: {
  onClose: () => void
  initialDid?: string
  initialEmail?: string
  existingUser?: string
}) {
  const { t } = useI18n()
  const store = useUsersAgentsStore()
  const directory = useUserGroups()
  const [source, setSource] = useState(initialEmail || existingUser ? 'google' : 'did')
  const [did, setDid] = useState(initialDid)
  const [email, setEmail] = useState(initialEmail)
  const [user, setUser] = useState(existingUser || '')
  const [type, setType] = useState<'user' | 'limited'>('user')
  const [groups, setGroups] = useState<string[]>([])
  const [days, setDays] = useState('7')
  const [url, setUrl] = useState('')
  const [busy, setBusy] = useState(false)
  const [error, setError] = useState('')
  const [copied, setCopied] = useState(false)
  const submit = async (event: FormEvent) => {
    event.preventDefault()
    if (busy) return
    setBusy(true)
    setError('')
    try {
      const { data, error } = await createUserInvite({
        userId: user.trim(), userType: source === 'google' ? 'user' : type, groups, ttlSecs: Number(days) * 86400,
        ...(source === 'google' ? { googleEmail: email.trim(), bindExisting: !!existingUser } : { targetDid: did.trim() }),
      })
      if (!data?.invite_url) throw error || new Error(t('sso.userInvite.issueFailed', 'Could not issue this invitation.'))
      setUrl(data.invite_url)
      void store.reload().catch(() => undefined)
    } catch (err) {
      setError(err instanceof Error ? err.message : String(err))
    } finally { setBusy(false) }
  }
  if (url) return <div className="space-y-3">
    <Alert severity="success">{t('sso.userInvite.share', 'Send this invitation link to the recipient.')}</Alert>
    <TextField fullWidth value={url} label={t('sso.userInvite.link', 'Invitation link')} slotProps={{input:{readOnly:true}}} />
    <Button onClick={async () => { try { await navigator.clipboard.writeText(url); setCopied(true) } catch { setError(t('sso.userInvite.copyManually', 'Select the link to copy it.')) } }}>{copied ? t('sso.oneTimeLink.copied','Copied') : t('sso.oneTimeLink.copy','Copy link')}</Button>
    {error && <Alert severity="info">{error}</Alert>}<Button onClick={onClose}>{t('sso.oneTimeLink.done','Done')}</Button>
  </div>
  return <form className="space-y-3 p-4" onSubmit={(e) => void submit(e)}>
    <h3>{existingUser ? t('sso.userInvite.bindGoogle','Bind Google to this account') : t('sso.userInvite.issue','Invite a user to this Zone')}</h3>
    <TextField fullWidth required disabled={!!existingUser} value={user} onChange={(e) => setUser(e.target.value)} label={t('usersAgents.newUser.username')} />
    {!existingUser && <TextField select fullWidth value={source} SelectProps={{native:true}} onChange={e => { setSource(e.target.value); if (e.target.value === 'google') setType('user') }} label={t('sso.userInvite.credential','Login credential')}>
      <option value="did">BuckyOS DID</option><option value="google">Google</option>
    </TextField>}
    {source === 'google' ? <TextField fullWidth type="email" required label={t('sso.userInvite.email','Google email')} value={email} onChange={e => setEmail(e.target.value)} />
      : <TextField fullWidth required label={t('sso.userInvite.did','Recipient DID')} value={did} onChange={e => setDid(e.target.value)} />}
    {!existingUser && <>
      <TextField select fullWidth value={type} SelectProps={{native:true}} label={t('usersAgents.newUser.userType')} onChange={e => setType(e.target.value as 'user' | 'limited')}>
        <option value="user">{t('usersAgents.role.user')}</option>
        {source !== 'google' && <option value="limited">{t('usersAgents.role.limited')}</option>}
      </TextField>
      {directory?.groups?.length ? <fieldset><legend>{t('usersAgents.newUser.groups')}</legend>{directory.groups.map(group =>
        <FormControlLabel key={group.id} label={group.name} control={<Checkbox checked={groups.includes(group.id)} onChange={e =>
          setGroups(e.target.checked ? [...groups,group.id] : groups.filter(id => id !== group.id))} />} />)}</fieldset> : null}
    </>}
    <TextField required fullWidth type="number" slotProps={{htmlInput:{min:1,max:365}}} label={t('sso.userInvite.days','Valid for (days)')} value={days} onChange={e => setDays(e.target.value)} />
    {error && <Alert severity="error">{error}</Alert>}
    <Button type="submit" variant="contained" disabled={busy}>{t('sso.userInvite.makeLink','Create invitation link')}</Button>
    <Button disabled={busy} onClick={onClose}>{t('sso.common.cancel','Cancel')}</Button>
  </form>
}
