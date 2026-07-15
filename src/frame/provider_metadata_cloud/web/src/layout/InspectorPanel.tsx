import { JsonViewer } from '../components/json-viewer/JsonViewer'
import { StatusBadge } from '../components/status/StatusBadge'
import { useI18n } from '../i18n/provider'

export interface InspectorState {
  title: string
  subtitle?: string
  status?: string
  json?: unknown
}

export function InspectorPanel({ inspector }: { inspector: InspectorState | null }) {
  const { t } = useI18n()
  return (
    <aside className="hidden w-80 shrink-0 border-l border-[color:var(--cp-border)] bg-[color:color-mix(in_srgb,var(--cp-surface)_82%,transparent)] p-4 lg:block">
      <div className="mb-4 flex items-center justify-between">
        <h2 className="text-sm font-bold">{t('inspector.title', 'Inspector')}</h2>
        {inspector?.status ? <StatusBadge tone="accent">{inspector.status}</StatusBadge> : null}
      </div>
      {inspector ? (
        <div className="space-y-4">
          <div>
            <div className="text-base font-bold">{inspector.title}</div>
            {inspector.subtitle ? <p className="mt-1 text-xs leading-5 text-[color:var(--cp-muted)]">{inspector.subtitle}</p> : null}
          </div>
          <div>
            <div className="mb-2 text-xs font-bold uppercase text-[color:var(--cp-muted)]">{t('inspector.json', 'Published JSON')}</div>
            <JsonViewer value={inspector.json ?? inspector} />
          </div>
        </div>
      ) : (
        <p className="text-sm leading-6 text-[color:var(--cp-muted)]">{t('inspector.none', 'Select a provider or model row to inspect details.')}</p>
      )}
    </aside>
  )
}
