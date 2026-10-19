import { HomeStationView } from './HomeStationView'
import { homeStationLaunchSchema } from './launch'
import type { AppContentLoaderProps } from '../types'

export function HomeStationAppPanel({ launch }: AppContentLoaderProps) {
  const parsed = homeStationLaunchSchema.safeParse(launch?.payload)
  return <HomeStationView key={launch?.requestId ?? 'default'} compose={parsed.success ? parsed.data.compose : undefined} />
}
