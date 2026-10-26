import { useCallback, useEffect, useRef, useState } from 'react'
import { Alert, Button, CircularProgress } from '@mui/material'
import { RotateCcw } from 'lucide-react'
import { changeUserState, fetchUserList, type UserInfo } from '../../../../api/user_mgr'
import { useSudoByPassword } from '../../../../components/sudo'
import { useI18n } from '../../../../i18n/provider'
import { errorMessage } from '../../../agent-setup/model'
import { useSelf, useUsersAgentsStore, useViewerIsAdmin } from '../../hooks/use-users-agents-store'

export function DeletedUsersPage({ onNotice }: { onNotice: (message: string) => void }) {
  const { t } = useI18n()
  const isAdmin = useViewerIsAdmin()
  const self = useSelf()
  const store = useUsersAgentsStore()
  const requestSudo = useSudoByPassword()
  const pending = useRef(false)
  const [users, setUsers] = useState<UserInfo[]>([])
  const [busy, setBusy] = useState(false)
  const [loaded, setLoaded] = useState(false)
  const [error, setError] = useState<string | null>(null)
  const [committed, setCommitted] = useState<string | null>(null)
  const [uncertain, setUncertain] = useState<string | null>(null)

  const load = useCallback(async () => {
    const { data, error: failure } = await fetchUserList({ includeDeleted: true })
    if (!data) throw failure ?? new Error('user.list failed')
    return data.users.filter((user) => user.state === 'deleted' && user.user_type !== 'root' && user.user_id !== self.id)
  }, [self.id])

  useEffect(() => {
    if (!isAdmin) return
    let cancelled = false
    void load().then((next) => {
      if (!cancelled) { setUsers(next); setLoaded(true) }
    }).catch((failure) => {
      if (!cancelled) setError(t('usersAgents.deleted.loadFailed', undefined, { reason: errorMessage(failure) }))
    })
    return () => { cancelled = true }
  }, [isAdmin, load, t])

  const refresh = async (restoredId: string | null = committed) => {
    try {
      const [next, snapshot] = await Promise.all([load(), store.reload()])
      if (restoredId && (!snapshot.localUsers.some((user) => user.id === restoredId) || next.some((user) => user.user_id === restoredId))) {
        throw new Error(t('usersAgents.lifecycle.notRestored'))
      }
      setUsers(next)
      setLoaded(true)
      setCommitted(null)
      setError(null)
    } catch (failure) {
      setError(t(restoredId ? 'usersAgents.lifecycle.restoreReloadFailed' : 'usersAgents.deleted.loadFailed', undefined, { reason: errorMessage(failure) }))
    }
  }

  const reconcileRestore = async (userId: string, failure?: unknown) => {
    try {
      const result = await fetchUserList({ includeDeleted: true })
      if (!result.data) throw result.error ?? new Error('user.list failed')
      const current = result.data.users.find((user) => user.user_id === userId)
      if (!current) throw new Error(t('usersAgents.lifecycle.accountMissing'))
      setUncertain(null)
      if (current.state === 'active') {
        setCommitted(userId)
        onNotice(t('usersAgents.lifecycle.responseLost'))
        await refresh(userId)
      } else {
        setError(t('usersAgents.deleted.restoreFailed', undefined, { reason: failure ? errorMessage(failure) : t('usersAgents.lifecycle.notRestored') }))
      }
    } catch (readError) {
      setUncertain(userId)
      setError(t('usersAgents.lifecycle.restoreUncertain', undefined, { reason: errorMessage(failure ?? readError) }))
    }
  }

  const restore = async (user: UserInfo) => {
    if (pending.current || committed || uncertain || !isAdmin) return
    if (!window.confirm(t('usersAgents.deleted.restoreConfirm', undefined, { name: user.show_name || user.user_id }))) return
    pending.current = true
    setBusy(true)
    setError(null)
    let mutationStarted = false
    try {
      const grant = await requestSudo({
        aud: 'system-config',
        appid: 'control-panel',
        username: self.id,
        title: t('usersAgents.deleted.restore'),
        description: t('usersAgents.deleted.restoreConfirm', undefined, { name: user.show_name || user.user_id }),
        reason: `Restore user ${user.user_id}`,
        confirmLabel: t('usersAgents.deleted.restore'),
      })
      if (!grant) return
      mutationStarted = true
      const { data, error: failure } = await changeUserState({ userId: user.user_id, state: 'active' }, { sessionToken: grant.sessionToken })
      if (!data?.ok || data.state !== 'active' || data.user_id !== user.user_id) {
        await reconcileRestore(user.user_id, failure ?? new Error('Invalid user.change_state response'))
        return
      }
      setCommitted(user.user_id)
      if (data.warning || !data.rbac_refreshed) onNotice(data.warning ?? t('usersAgents.lifecycle.rbacPending'))
      await refresh(user.user_id)
    } catch (failure) {
      if (mutationStarted) await reconcileRestore(user.user_id, failure)
      else setError(t('usersAgents.deleted.restoreFailed', undefined, { reason: errorMessage(failure) }))
    } finally {
      pending.current = false
      setBusy(false)
    }
  }

  const retryRefresh = async () => {
    if (pending.current) return
    pending.current = true
    setBusy(true)
    try { if (uncertain) await reconcileRestore(uncertain); else await refresh() } finally { pending.current = false; setBusy(false) }
  }

  if (!isAdmin) return <Alert severity="info">{t('usersAgents.newUser.error.permission')}</Alert>

  return (
    <section className="space-y-4" data-testid="deleted-users-page">
      <div className="flex items-center justify-between gap-3">
        <h3 className="font-display text-sm font-semibold" style={{ color: 'var(--cp-text)' }}>{t('usersAgents.deleted.title')}</h3>
        <Button size="small" disabled={busy} onClick={() => void retryRefresh()}>{t('usersAgents.lifecycle.refresh')}</Button>
      </div>
      <Alert severity="info">{t('usersAgents.deleted.hint')}</Alert>
      {error ? <Alert severity={committed || uncertain ? 'warning' : 'error'}>{error}</Alert> : null}
      {!loaded && !error ? <CircularProgress size={20} /> : null}
      {loaded && users.length === 0 ? <p className="text-sm" style={{ color: 'var(--cp-muted)' }}>{t('usersAgents.deleted.empty')}</p> : null}
      {users.map((user) => (
        <div key={user.user_id} className="rounded-[16px] border p-4 space-y-2" style={{ borderColor: 'var(--cp-border)' }} data-testid="deleted-user" data-user-id={user.user_id}>
          <div className="font-medium text-sm" style={{ color: 'var(--cp-text)' }}>{user.show_name || user.user_id}</div>
          <div className="text-xs break-all" style={{ color: 'var(--cp-muted)' }}>{user.user_id} · {user.user_type}{user.did ? ` · ${user.did}` : ''}</div>
          <Button size="small" disabled={busy || Boolean(committed) || Boolean(uncertain)} startIcon={busy ? <CircularProgress size={14} color="inherit" /> : <RotateCcw size={14} />} onClick={() => void restore(user)}>
            {t('usersAgents.deleted.restore')}
          </Button>
        </div>
      ))}
    </section>
  )
}
