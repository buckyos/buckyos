import { useMemo, useState } from 'react'
import { useNavigate, useParams, useSearchParams } from 'react-router-dom'
import { signOutToLogin } from '../../auth/signOut'
import { useI18n } from '../../i18n/provider'
import { useDocumentTitle } from '../../presentation/documentHead'
import { AIWorkspaceAppPanel, type TabHost } from './AIWorkspaceAppPanel'
import { targetOf, workspacePath } from './links'

export function AIWorkspaceRoute() {
  const { workspaceId = null } = useParams()
  const [searchParams, setSearchParams] = useSearchParams()
  const navigate = useNavigate()
  const search = searchParams.toString()
  const target = useMemo(() => targetOf(new URLSearchParams(search)), [search])

  const { t } = useI18n()
  const appName = t('apps.aiworkspace')
  const [shownTitle, setShownTitle] = useState<string | null>(null)
  useDocumentTitle(shownTitle ? `${shownTitle} · ${appName}` : appName)

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
    onShown: (shown) => setShownTitle(shown?.title ?? null),
    home: () => navigate('/'),
    signOut: signOutToLogin,
  }

  return (
    <main className="h-dvh w-full overflow-hidden bg-[color:var(--cp-bg)]" data-testid="aiws-tab">
      <AIWorkspaceAppPanel tab={tab} />
    </main>
  )
}
