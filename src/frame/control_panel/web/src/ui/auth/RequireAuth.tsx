import type { ReactNode } from 'react'
import { Navigate, Outlet, useLocation } from 'react-router-dom'

import { useAuth } from '@/auth/useAuth'

type RequireAuthProps = {
  children?: ReactNode
}

const RequireAuth = ({ children }: RequireAuthProps) => {
  const location = useLocation()
  const { status } = useAuth()

  if (status === 'loading') {
    return (
      <div className="flex min-h-screen items-center justify-center px-4 py-8 text-sm text-[var(--cp-muted)]">
        Verifying session...
      </div>
    )
  }

  if (status === 'authenticated') {
    return children ? <>{children}</> : <Outlet />
  }

  const redirect = `${location.pathname}${location.search}${location.hash}`
  return <Navigate to={`/login?redirect=${encodeURIComponent(redirect)}`} replace />
}

export default RequireAuth
