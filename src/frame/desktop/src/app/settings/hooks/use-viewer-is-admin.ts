import { useEffect, useState } from 'react'
import { fetchCurrentAccount } from '../../../api/account'
import { isPrivilegedUserType } from '../../../api/user_type'

/** Whether the signed-in account administers the Zone; `null` until known. */
export function useViewerIsAdmin(): boolean | null {
  const [isAdmin, setIsAdmin] = useState<boolean | null>(null)
  useEffect(() => {
    let live = true
    void fetchCurrentAccount()
      .catch(() => null)
      .then((account) => { if (live) setIsAdmin(isPrivilegedUserType(account?.user_type)) })
    return () => { live = false }
  }, [])
  return isAdmin
}
