import { useMediaQuery } from '@mui/material'
import { mobileStatusBarMode, shellStatusBarHeight } from '../../desktop/shell'
import { MessageHubView } from './MessageHubView'
import { messageHubLaunchSchema } from './launch'
import type { AppContentLoaderProps } from '../types'

export function MessageHubAppPanel({ launch, app, layoutState }: AppContentLoaderProps) {
  const isMobile = useMediaQuery('(max-width: 768px)')
  const topInset = isMobile && mobileStatusBarMode(app) === 'compact' ? `calc(var(--sat, 0px) + ${layoutState.deadZone.top + shellStatusBarHeight('mobile', app)}px)` : undefined
  const payload = messageHubLaunchSchema.safeParse(launch?.payload)
  // An invalid launch payload must not fall back to another identity's data:
  // it opens an observer context with an empty owner, which the store denies.
  const contextRequest = payload.success ? payload.data.context : launch ? { ownerDid: '', mode: 'observe' as const } : undefined
  return <div className="h-full" style={{ paddingTop: topInset }}><MessageHubView key={launch?.requestId} initialEntityId={payload.success ? payload.data.entityId : null} contextRequest={contextRequest} /></div>
}
