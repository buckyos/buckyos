import { buckyos } from 'buckyos'
import { isMockRuntime } from '../runtime.ts'
import { fetchUserDetail } from './user_mgr.ts'

export interface CurrentAccount {
  user_id: string
  user_name: string
  user_type: string
}

/** The signed-in account; the mock runtime answers with its in-browser control panel's account. */
export async function fetchCurrentAccount(): Promise<CurrentAccount | null> {
  if (isMockRuntime()) {
    const { mockCurrentAccount } = await import('./control_panel_mock.ts')
    return mockCurrentAccount()
  }
  const info = await buckyos.getAccountInfo()
  if (!info?.user_id) return null
  let userType = info.user_type ?? ''
  if (!userType) {
    const { data } = await fetchUserDetail({ userId: info.user_id })
    userType = String(data?.user_type ?? '')
  }
  return { user_id: info.user_id, user_name: info.user_name || info.user_id, user_type: userType }
}

/** Limited users (and guests) cannot create Agents. */
export function isLimitedUserType(userType: string | undefined | null): boolean {
  const normalized = String(userType ?? '').toLowerCase()
  return normalized === 'limited' || normalized === 'guest'
}
