import { useNavigate, useSearchParams } from 'react-router-dom'
import { messageHubRouteContext } from './launch'
import { MessageHubView } from './MessageHubView'

export function MessageHubRoute() {
  const [searchParams, setSearchParams] = useSearchParams()
  const navigate = useNavigate()
  const entityId = searchParams.get('entityId')
  const sessionId = searchParams.get('sessionId')
  const exitObserver = () => setSearchParams(previous => {
    const next = new URLSearchParams(previous)
    next.delete('ownerDid'); next.delete('mode'); next.delete('entityId'); next.delete('sessionId')
    return next
  }, { replace: true })

  return (
    <main className="min-h-dvh bg-[color:var(--cp-bg)] p-0 md:p-5">
      <div
        className="mx-auto h-dvh w-full overflow-hidden md:h-[calc(100dvh-2.5rem)] md:max-w-[1600px] md:rounded-[28px] md:border md:shadow-[var(--cp-window-shadow)]"
        style={{
          borderColor: 'var(--cp-border)',
          background:
            'linear-gradient(180deg, color-mix(in srgb, var(--cp-surface) 96%, transparent), color-mix(in srgb, var(--cp-surface-2) 94%, transparent))',
          backdropFilter: 'blur(20px)',
        }}
      >
        <MessageHubView initialEntityId={entityId} initialSessionId={sessionId} contextRequest={messageHubRouteContext(searchParams)} onHome={() => navigate('/')} onExitObserver={exitObserver} />
      </div>
    </main>
  )
}
