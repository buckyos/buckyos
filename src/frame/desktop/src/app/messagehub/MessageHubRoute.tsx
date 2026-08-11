import { useSearchParams } from 'react-router-dom'
import { MessageHubView } from './MessageHubView'

export function MessageHubRoute() {
  const [searchParams] = useSearchParams()
  const entityId = searchParams.get('entityId') ?? 'agent-coder'

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
        <MessageHubView initialEntityId={entityId} />
      </div>
    </main>
  )
}
