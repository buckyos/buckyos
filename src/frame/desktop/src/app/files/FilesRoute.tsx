import { useSearchParams } from 'react-router-dom'
import { useI18n } from '../../i18n/provider'
import { useThemeMode } from '../../theme/provider'
import { FilesView } from './FilesView'

export function FilesRoute() {
  const [searchParams] = useSearchParams()
  const { locale } = useI18n()
  const { themeMode } = useThemeMode()
  const path = searchParams.get('path') ?? '/Desktop'

  return (
    <main className="min-h-dvh bg-[color:var(--cp-bg)] px-0 py-0 md:px-5 md:py-5">
      <div
        className="mx-auto h-dvh w-full overflow-hidden md:h-[calc(100dvh-2.5rem)] md:max-w-[1480px] md:rounded-[28px] md:border md:shadow-[var(--cp-window-shadow)]"
        style={{
          borderColor: 'var(--cp-border)',
          background:
            'linear-gradient(180deg, color-mix(in srgb, var(--cp-surface) 96%, transparent), color-mix(in srgb, var(--cp-surface-2) 94%, transparent))',
          backdropFilter: 'blur(20px)',
        }}
      >
        <FilesView
          key={`${path}:${themeMode}:${locale}`}
          embedded={false}
          initialPath={path}
          locale={locale}
          runtimeContainer="browser"
          themeMode={themeMode}
        />
      </div>
    </main>
  )
}
