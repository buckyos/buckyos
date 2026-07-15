import { useState } from 'react'
import { Outlet } from 'react-router-dom'
import { AlertTriangle } from 'lucide-react'
import { ErrorView, LoadingView } from '../components/empty-state/StateView'
import { StatusBadge } from '../components/status/StatusBadge'
import { useI18n } from '../i18n/provider'
import { useProviderMetadataStore } from '../state/useProviderMetadataStore'
import { InspectorPanel, type InspectorState } from './InspectorPanel'
import { MobileNav } from './MobileNav'
import { Sidebar } from './Sidebar'
import { TopBar } from './TopBar'

export interface ShellOutletContext {
  setInspector: (inspector: InspectorState | null) => void
}

export function CloudConsoleShell() {
  const { workspace, reload } = useProviderMetadataStore()
  const { t } = useI18n()
  const [inspector, setInspector] = useState<InspectorState | null>(null)

  if (workspace.status === 'loading' || workspace.status === 'idle') {
    return (
      <main className="min-h-dvh bg-[color:var(--cp-bg)]">
        <LoadingView />
      </main>
    )
  }

  if (workspace.status === 'error') {
    return (
      <main className="min-h-dvh bg-[color:var(--cp-bg)] p-4">
        <ErrorView retry={reload} />
      </main>
    )
  }

  const data = workspace.data!

  return (
    <main className="flex h-dvh flex-col overflow-hidden bg-[color:var(--cp-bg)]">
      <TopBar />
      {(data.tech_source.stale || data.warnings.length > 0) && (
        <div
          className="flex min-h-10 items-center gap-2 border-b border-[color:var(--cp-border)] bg-[color:color-mix(in_srgb,var(--cp-warning)_12%,var(--cp-surface))] px-4 text-xs text-[color:var(--cp-text)] md:px-5"
          data-testid="workspace-status-banner"
        >
          <AlertTriangle className="shrink-0 text-[color:var(--cp-warning)]" size={15} />
          <StatusBadge tone={data.tech_source.stale ? 'warning' : 'accent'}>
            {data.tech_source.stale ? t('status.stale', 'Stale') : t('status.warning', 'Warning')}
          </StatusBadge>
          <span className="min-w-0 truncate">
            {data.tech_source.stale
              ? t('state.stale', 'Technical source cache is stale; previous cache remains available.')
              : t('state.warningSummary', '{{count}} diagnostics need review before publish.', { count: data.warnings.length })}
          </span>
        </div>
      )}
      <div className="flex min-h-0 flex-1">
        <Sidebar />
        <section className="shell-scrollbar min-w-0 flex-1 overflow-auto">
          <div className="mx-auto w-full max-w-7xl p-3 md:p-5">
            <Outlet context={{ setInspector } satisfies ShellOutletContext} />
          </div>
        </section>
        <InspectorPanel inspector={inspector} />
      </div>
      <MobileNav />
    </main>
  )
}
