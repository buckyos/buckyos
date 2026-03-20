import type { ReactNode } from 'react'
import { useCallback, useEffect, useMemo, useState } from 'react'

import { ensureAuthRuntime, ensureSessionToken, loginWithPassword, signOutSession } from './authManager'
import { AuthContext, type AuthContextValue, type AuthStatus } from './authContext'

type AuthProviderProps = {
  children: ReactNode
}

const AuthProvider = ({ children }: AuthProviderProps) => {
  const [status, setStatus] = useState<AuthStatus>('loading')
  const [initError, setInitError] = useState<string | null>(null)

  const refreshAuth = useCallback(async () => {
    setInitError(null)
    setStatus('loading')

    try {
      await ensureAuthRuntime()
      const token = await ensureSessionToken()
      setStatus(token ? 'authenticated' : 'unauthenticated')
    } catch (error) {
      setStatus('unauthenticated')
      const message = error instanceof Error ? error.message : String(error)
      setInitError(message || 'Failed to initialize authentication runtime')
    }
  }, [])

  useEffect(() => {
    void refreshAuth()
  }, [refreshAuth])

  const signInWithPasswordAction = useCallback(async (username: string, password: string, redirectUrl?: string | null) => {
    setInitError(null)
    await ensureAuthRuntime()
    await loginWithPassword(username, password, redirectUrl)
    setStatus('authenticated')
  }, [])

  const signOutAction = useCallback(async () => {
    await signOutSession()
    setStatus('unauthenticated')
  }, [])

  const value = useMemo<AuthContextValue>(
    () => ({
      status,
      initError,
      refreshAuth,
      signInWithPassword: signInWithPasswordAction,
      signOut: signOutAction,
    }),
    [initError, refreshAuth, signInWithPasswordAction, signOutAction, status],
  )

  return <AuthContext.Provider value={value}>{children}</AuthContext.Provider>
}

export default AuthProvider
