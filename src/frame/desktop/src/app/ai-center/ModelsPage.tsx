import type { ReactNode } from 'react'
import { Cpu, Download, Pause, Play, Store, Trash2 } from 'lucide-react'
import { useI18n } from '../../i18n/provider'
import { useLocalModels } from './hooks/use-aicc-store'
import { EmptyState } from './components/shared/EmptyState'
import { StatusBadge } from './components/shared/StatusBadge'
import { LongField } from './components/shared/LongField'
import type { LocalModel } from '../../api/aicc_mgr'

const SHOW_LOCAL_MODELS_PAGE = false

function formatSize(bytes?: number): string {
  if (!bytes) return '—'
  return `${(bytes / (1024 * 1024 * 1024)).toFixed(1)} GB`
}

function statusVariant(status: LocalModel['health']['status']): 'ok' | 'warning' | 'error' {
  if (status === 'available') return 'ok'
  if (status === 'degraded') return 'warning'
  return 'error'
}

export function ModelsPage() {
  if (SHOW_LOCAL_MODELS_PAGE) {
    return <LocalModelsPage />
  }

  return <ComingSoonModelsPage />
}

function ComingSoonModelsPage() {
  const { t } = useI18n()

  return (
    <div className="flex min-h-[420px] flex-col items-center justify-center px-4 py-16 text-center">
      <div
        className="mb-5 inline-flex h-14 w-14 items-center justify-center rounded-lg"
        style={{
          background: 'var(--cp-surface)',
          border: '1px solid var(--cp-border)',
          color: 'var(--cp-accent)',
        }}
      >
        <Cpu size={28} />
      </div>

      <p className="mb-2 text-xs font-semibold uppercase" style={{ color: 'var(--cp-accent)' }}>
        {t('aiCenter.models.kicker', 'Models')}
      </p>
      <h2 className="text-xl font-semibold" style={{ color: 'var(--cp-text)' }}>
        {t('aiCenter.models.comingSoon.title', 'Coming Soon')}
      </h2>
      <p className="mt-3 max-w-xl text-sm leading-6" style={{ color: 'var(--cp-muted)' }}>
        {t(
          'aiCenter.models.comingSoon.desc',
          'Local Provider and local LLM Engine support is planned for a future release. Installed local model inventory is not available in this version.'
        )}
      </p>
    </div>
  )
}

function LocalModelsPage() {
  const { t } = useI18n()
  const models = useLocalModels()

  if (models.length === 0) {
    return (
      <EmptyState
        icon={<Store size={48} />}
        title={t('aiCenter.models.empty.title', 'No Local Models Installed')}
        description={t('aiCenter.models.empty.desc', 'Local models are local_inference provider instances. Install them from the Store; they will join the same routing tree as cloud models.')}
        action={{
          label: t('aiCenter.models.empty.cta', 'Go to Model Store'),
          onClick: () => console.log('navigate to model store'),
        }}
      />
    )
  }

  return (
    <div className="flex flex-col gap-5">
      <div className="flex flex-col gap-1">
        <h2 className="text-lg font-semibold" style={{ color: 'var(--cp-text)' }}>
          {t('aiCenter.models.title', 'Local Models')}
        </h2>
        <p className="text-sm" style={{ color: 'var(--cp-muted)' }}>
          {t('aiCenter.models.subtitle', 'Installed local models are displayed as @local exact models and participate in the same logical routing tree.')}
        </p>
      </div>

      <div className="flex justify-end">
        <button
          type="button"
          className="inline-flex items-center gap-2 rounded-lg px-3 py-2 text-sm font-medium"
          style={{ background: 'var(--cp-accent)', color: '#fff' }}
          onClick={() => console.log('navigate to model store')}
        >
          <Download size={16} />
          {t('aiCenter.models.install', 'Install Model')}
        </button>
      </div>

      <div className="grid grid-cols-1 gap-3">
        {models.map((m) => (
          <LocalModelCard key={m.id} model={m} />
        ))}
      </div>
    </div>
  )
}

function LocalModelCard({ model }: { model: LocalModel }) {
  return (
    <article className="rounded-xl p-4" style={{ background: 'var(--cp-surface)', border: '1px solid var(--cp-border)' }}>
      <div className="flex flex-col md:flex-row md:items-start md:justify-between gap-3">
        <div className="flex items-start gap-3 min-w-0">
          <div className="p-2 rounded-lg" style={{ background: 'var(--cp-bg)', color: 'var(--cp-accent)' }}>
            <Cpu size={18} />
          </div>
          <div className="min-w-0">
            <div className="flex flex-wrap items-center gap-2">
              <h3 className="text-sm font-semibold" style={{ color: 'var(--cp-text)' }}>{model.name}</h3>
              <StatusBadge status={statusVariant(model.health.status)} label={model.health.status} />
            </div>
            <LongField value={model.exact_model} className="mt-1 text-xs" mono tone="muted" expandable />
            <div className="flex flex-wrap gap-2 mt-2">
              {model.logical_mounts.map((mount) => (
                <span key={mount} className="text-[11px] rounded-md px-2 py-1" style={{ background: 'var(--cp-bg)', color: 'var(--cp-text)' }}>
                  <LongField value={mount} copyable={false} />
                </span>
              ))}
            </div>
          </div>
        </div>
        <div className="flex items-center gap-2">
          <IconButton icon={model.status === 'ready' ? <Pause size={14} /> : <Play size={14} />} label={model.status === 'ready' ? 'Disable' : 'Enable'} />
          <IconButton icon={<Trash2 size={14} />} label="Delete" danger />
        </div>
      </div>

      <div className="grid grid-cols-2 md:grid-cols-5 gap-3 mt-4 text-xs">
        <Fact label="Size" value={formatSize(model.size_bytes)} />
        <Fact label="API Type" value={model.api_types.join(', ')} />
        <Fact label="Context" value={model.capabilities.max_context_tokens?.toLocaleString() ?? '—'} />
        <Fact label="Latency" value={`${model.health.p50_latency_ms ?? '—'}ms p50`} />
        <Fact label="Last Used" value={model.last_used_at?.slice(0, 10) ?? 'Never'} />
      </div>
    </article>
  )
}

function Fact({ label, value }: { label: string; value: string }) {
  return (
    <div>
      <div style={{ color: 'var(--cp-muted)' }}>{label}</div>
      <LongField value={value} className="font-medium" copyable={false} expandable />
    </div>
  )
}

function IconButton({ icon, label, danger }: { icon: ReactNode; label: string; danger?: boolean }) {
  return (
    <button
      type="button"
      title={label}
      className="inline-flex h-8 w-8 items-center justify-center rounded-lg"
      style={{
        border: '1px solid var(--cp-border)',
        color: danger ? 'var(--cp-danger)' : 'var(--cp-text)',
      }}
    >
      {icon}
    </button>
  )
}
