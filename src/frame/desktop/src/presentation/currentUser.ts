/* ── The signed-in user as every surface shows them (taskbar, AI Workspace, …) ── */

import { useEffect, useState } from 'react'
import { fetchCurrentAccount } from '../api/account'
import { fetchUserDetail } from '../api/user_mgr'
import { onOwnProfileChanged } from '../app/agent-setup/events'
import { presentUser, textValue } from './model'
import { presentationStore } from './store'

export interface CurrentUser {
  userId: string
  displayName: string
  userType: string
  did?: string
  avatarUrl?: string
}

async function loadCurrentUser(): Promise<CurrentUser | null> {
  const account = await fetchCurrentAccount().catch(() => null)
  if (!account) return null
  const { data: detail } = await fetchUserDetail({ userId: account.user_id })
  const user = presentUser({
    userId: account.user_id,
    did: textValue(detail?.local_profile?.did) ?? textValue(detail?.did_document?.id),
    profile: detail?.profile,
    document: detail?.did_document,
  })
  presentationStore.put(user)
  return {
    userId: account.user_id,
    displayName: user.title,
    userType: account.user_type,
    did: user.did,
    avatarUrl: user.icons[0],
  }
}

/** The signed-in user as the shell shows them; null until loaded or when signed out. Reloads after a profile save. */
export function useCurrentUser() {
  const [user, setUser] = useState<CurrentUser | null>(null)
  useEffect(() => {
    let cancelled = false
    const load = () => {
      void loadCurrentUser().then((next) => {
        if (!cancelled) setUser(next)
      })
    }
    load()
    const stop = onOwnProfileChanged(load)
    return () => {
      cancelled = true
      stop()
    }
  }, [])
  return user
}
