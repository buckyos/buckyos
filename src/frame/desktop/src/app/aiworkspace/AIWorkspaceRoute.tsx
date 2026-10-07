import { useEffect, useMemo, useState } from 'react'
import { useNavigate, useParams, useSearchParams } from 'react-router-dom'
import { signOutToLogin } from '../../auth/signOut'
import { AIWorkspaceAppPanel, type TabHost } from './AIWorkspaceAppPanel'
import { targetOf, workspacePath } from './links'

export function AIWorkspaceRoute() {
  const { workspaceId = null } = useParams()
  const [searchParams, setSearchParams] = useSearchParams()
  const navigate = useNavigate()
  const search = searchParams.toString()
  const target = useMemo(() => targetOf(new URLSearchParams(search)), [search])

  const [previousTitle] = useState(() => document.title)
  useEffect(() => () => { document.title = previousTitle }, [previousTitle])

  const tab: TabHost = {
    workspaceId,
    target,
    navigate: (id, options) => navigate(workspacePath(id), { replace: options?.replace }),
    consumeTarget: () => setSearchParams((previous) => {
      const next = new URLSearchParams(previous)
      next.delete('surface')
      next.delete('block')
      return next
    }, { replace: true }),
    onShown: (shown) => { document.title = shown ? `${shown.title} - AI Workspace` : 'AI Workspace' },
    home: () => navigate('/'),
    signOut: signOutToLogin,
  }

  return (
    <main className="h-dvh w-full overflow-hidden bg-[color:var(--cp-bg)]" data-testid="aiws-tab">
      <AIWorkspaceAppPanel tab={tab} />
    </main>
  )
}
