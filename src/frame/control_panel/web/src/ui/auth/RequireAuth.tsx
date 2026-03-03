import type { ReactNode } from 'react'
import { Navigate, Outlet, useLocation } from 'react-router-dom'

import { hasStoredSession } from '@/auth/session'

type RequireAuthProps = {
  children?: ReactNode
}

const RequireAuth = ({ children }: RequireAuthProps) => {
  const location = useLocation()

  if (hasStoredSession()) {
    return children ? <>{children}</> : <Outlet />
  }

  const redirect = `${location.pathname}${location.search}${location.hash}`
  return <Navigate to={`/login?redirect=${encodeURIComponent(redirect)}`} replace />
}

export default RequireAuth
