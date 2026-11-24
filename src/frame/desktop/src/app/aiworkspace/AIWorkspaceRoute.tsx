/* `/workspace` on the Zone host: the signed-in user's workspace list in a tab of its own (the phone's entry). A
 * workspace itself lives on its own origin (T4-8): picking one goes there, signed in through the Control Panel's
 * launch (launch.ts). */

import { useState } from 'react'
import { useNavigate } from 'react-router-dom'
import { signOutToLogin } from '../../auth/signOut'
import { useI18n } from '../../i18n/provider'
import { StandalonePage } from '../standalone/StandalonePage'
import { AIWorkspaceAppPanel, type TabHost } from './AIWorkspaceAppPanel'
import { openWorkspaceById } from './launch'

export function AIWorkspaceRoute() {
  const navigate = useNavigate()
  const { t } = useI18n()
  const appName = t('apps.aiworkspace')
  const [shownTitle, setShownTitle] = useState<string | null>(null)

  const tab: TabHost = {
    workspaceId: null,
    target: null,
    navigate: (id) => { if (id) void openWorkspaceById(id) },
    consumeTarget: () => undefined,
    onShown: (shown) => setShownTitle(shown?.title ?? null),
    home: () => navigate('/'),
    signOut: signOutToLogin,
  }

  return (
    <StandalonePage appId="aiworkspace" ownHeader routed title={shownTitle ? `${shownTitle} · ${appName}` : undefined}>
      {() => <div className="min-h-0 flex-1 overflow-hidden" data-testid="aiws-tab"><AIWorkspaceAppPanel tab={tab} /></div>}
    </StandalonePage>
  )
}
