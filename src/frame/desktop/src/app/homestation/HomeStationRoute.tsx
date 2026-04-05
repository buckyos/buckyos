import { HomeStationView } from './HomeStationView'

export function HomeStationRoute() {
  return (
    <main className="homestation-root min-h-dvh bg-[color:var(--cp-bg)] p-0 md:p-5">
      <div
        className="mx-auto h-dvh w-full overflow-hidden md:h-[calc(100dvh-2.5rem)] md:max-w-[1600px] md:rounded-[28px] md:border md:shadow-[var(--cp-window-shadow)]"
        style={{
          borderColor: 'var(--cp-border)',
          background:
            'linear-gradient(180deg, color-mix(in srgb, var(--cp-surface) 96%, transparent), color-mix(in srgb, var(--cp-surface-2) 94%, transparent))',
          backdropFilter: 'blur(20px)',
        }}
      >
        <HomeStationView />
      </div>
    </main>
  )
}
