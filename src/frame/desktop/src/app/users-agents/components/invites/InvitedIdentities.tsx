import { useEffect, useState } from 'react'
import { Alert, Button, CircularProgress } from '@mui/material'
import { callControlPanel } from '../../../../api/user_mgr'
import { useI18n } from '../../../../i18n/provider'
import { AccountInviteForm } from './AccountInviteForm'

interface Identity {
  did: string
  email?: string
  user_id?: string
  account_did?: string
  account_state?: string
  sources: {invite_id:string; title:string; target:string; issuer:string; expires_at?:number; accepted_at?:number; state?:string}[]
}
export function InvitedIdentities() {
  const {t} = useI18n()
  const [users,setUsers] = useState<Identity[]>()
  const [error,setError] = useState('')
  const [selected,setSelected] = useState<Identity>()
  const [all,setAll] = useState(false)
  const [revision,setRevision] = useState(0)
  useEffect(() => {
    let active = true
    void callControlPanel<{users: Identity[]}>('user.invite.directory',{}).then(({data,error}) => {
      if (!active) return
      if (data) { setUsers(data.users); setError('') } else setError(String(error))
    })
    return () => {active = false}
  },[revision])
  if (selected) return <AccountInviteForm initialDid={selected.did} initialEmail={selected.email} onClose={() => {setSelected(undefined);setRevision(v => v+1)}} />
  return <div className="space-y-4 p-4">
    <h2>{t('sso.userInvite.directory','Invited identities')}</h2>
    <Button onClick={() => setAll(!all)}>{all ? t('sso.userInvite.filterExternal','Show users without Zone login') : t('sso.userInvite.showAll','Show all invited identities')}</Button>
    {error ? <Alert severity="error">{error}</Alert> : !users ? <CircularProgress size={20} /> : null}
    {users?.filter(user => all || !user.account_did).length === 0 && <p>{t('sso.userInvite.empty','No invitations have identified a recipient yet.')}</p>}
    {users?.filter(user => all || !user.account_did).map(user => <section key={user.did} className="space-y-2">
      <h3 className="break-all font-semibold">{user.did}</h3>
      <div>{user.user_id ? `${user.user_id} · ${user.account_state}${user.account_did ? " · "+user.account_did : ""}` : t('sso.userInvite.appOnly','App access only')}</div>
      <ul>{user.sources.map(src => <li key={src.invite_id} className="break-words">
        {src.title} · {src.target} · {src.issuer} · {src.state || (src.accepted_at ? t('sso.userInvite.acceptedState','Accepted') : t('sso.userInvite.offered','Invited'))}
        {src.expires_at ? ' · ' + new Date(src.expires_at * 1000).toLocaleString() : ''}
      </li>)}</ul>
      {!user.user_id && <Button onClick={() => setSelected(user)}>{t('sso.userInvite.allow','Allow login to this Zone')}</Button>}
    </section>)}
  </div>
}
