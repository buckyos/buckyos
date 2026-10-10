/* ── Agent setup app panel: the shared Add Agent wizard (PRD §3.3) ── */

import { useMediaQuery } from '@mui/material'
import { WindowDialogProvider } from '../../desktop/windows/dialogs'
import type { AppContentLoaderProps } from '../types'
import { AgentSetupWizard } from './AgentSetupWizard'
import { agentSetupLaunchSchema } from './launch'

export function AgentSetupAppPanel({ launch, windowId }: AppContentLoaderProps) {
  const mobile = useMediaQuery('(max-width: 767px)')
  const parsed = agentSetupLaunchSchema.safeParse(launch?.payload)
  const source = parsed.success ? parsed.data.source : 'users_agents'
  const agentId = parsed.success ? parsed.data.agent_id : undefined
  // Its own dialog layer keeps the wizard mounted (and its draft in memory) while a dialog is open.
  return (
    <WindowDialogProvider permissions={{ fullscreen: false }} surface={mobile ? 'mobile' : 'desktop'}>
      <AgentSetupWizard
        key={`${launch?.requestId ?? 'default'}:${source}:${agentId ?? ''}`}
        source={source}
        agentId={agentId}
        windowId={windowId}
      />
    </WindowDialogProvider>
  )
}
