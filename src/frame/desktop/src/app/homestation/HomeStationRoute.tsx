import { useEffect, useMemo } from 'react'
import { useLocation } from 'react-router-dom'
import { pageLaunch } from '../standalone/pageLaunch'
import { StandalonePage } from '../standalone/StandalonePage'
import { HomeStationView } from './HomeStationView'
import { homeStationLaunchSchema } from './launch'

export function HomeStationRoute() {
  const location = useLocation()
  // eslint-disable-next-line react-hooks/exhaustive-deps -- every arrival at an address is a launch
  const launch = useMemo(() => pageLaunch('homestation', location), [location.key])
  // A reload does not start the same post again.
  useEffect(() => {
    if (!launch) return
    const url = new URL(window.location.href)
    for (const name of ['new', 'compose']) url.searchParams.delete(name)
    window.history.replaceState(window.history.state, '', url)
  }, [launch])
  const parsed = homeStationLaunchSchema.safeParse(launch?.payload)
  return (
    <StandalonePage appId="homestation" ownHeader routed>
      {() => <div className="min-h-0 flex-1 overflow-hidden"><HomeStationView key={launch?.requestId ?? 'default'} launch={parsed.success ? parsed.data : undefined} /></div>}
    </StandalonePage>
  )
}
