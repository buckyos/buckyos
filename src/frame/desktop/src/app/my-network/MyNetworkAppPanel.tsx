/* ── My Network – app panel entry point ── */

import type { AppContentLoaderProps } from '../types'
import { MyNetworkShell } from './components/MyNetworkShell'
import { myNetworkLaunchSchema } from './launch'

export function MyNetworkAppPanel({ launch }: AppContentLoaderProps) {
  const parsed = myNetworkLaunchSchema.safeParse(launch?.payload)
  return <MyNetworkShell key={launch?.requestId ?? 'default'} launch={parsed.success ? parsed.data : undefined} />
}
