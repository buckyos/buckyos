import { createContext } from 'react'

export type AuthStatus = 'loading' | 'authenticated' | 'unauthenticated'

export type AuthContextValue = {
  status: AuthStatus
  initError: string | null
  refreshAuth: () => Promise<void>
  signInWithPassword: (username: string, password: string, redirectUrl?: string | null) => Promise<void>
  signOut: () => Promise<void>
}

export const AuthContext = createContext<AuthContextValue | null>(null)
