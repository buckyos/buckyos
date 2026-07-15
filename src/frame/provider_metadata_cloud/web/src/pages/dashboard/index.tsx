import { AlertTriangle, CheckCircle2, Database, GitPullRequest, History } from 'lucide-react'
import { Link } from 'react-router-dom'
import { StatusBadge } from '../../components/status/StatusBadge'
import { buildPublishPreview } from '../../datamodel/selectors'
import { summarizePendingChanges } from '../../datamodel/diff'
import { useI18n } from '../../i18n/provider'
import { useProviderMetadataStore } from '../../state/useProviderMetadataStore'
import { formatDate } from '../pageUtils'

export function DashboardPage() {
  const { t } = useI18n()
  const { workspace, serviceRole } = useProviderMetadataStore()
  const data = workspace.data!
  const preview = buildPublishPreview(data)
  const diff = summarizePendingChanges(data.pending_changes)
  const visibleLogs = data.change_logs.filter((log) => log.service_role === serviceRole).slice(0, 4)

  return (
    <div className="space-y-5" data-testid="dashboard-page">
      <div className="flex flex-col gap-3 md:flex-row md:items-end md:justify-between">
        <div>
          <h1 className="text-2xl font-bold">{t('dashboard.title', 'Dashboard')}</h1>
          <p className="mt-1 text-sm text-[color:var(--cp-muted)]">
            {serviceRole === 'tech' ? t('role.tech', 'Technical parameters') : t('role.ops', 'Operations parameters')}
          </p>
        </div>
        <StatusBadge tone="success">{t('top.mock', 'Mock data')}</StatusBadge>
      </div>

      <section className="grid gap-3 sm:grid-cols-2 xl:grid-cols-5">
        <Metric icon={<Database size={18} />} label={t('dashboard.providers', 'Providers')} value={data.providers.length} />
        <Metric icon={<GitPullRequest size={18} />} label={t('dashboard.models', 'Model rules')} value={data.model_param_rules.length} />
        <Metric icon={<AlertTriangle size={18} />} label={t('dashboard.warnings', 'Warnings')} value={data.warnings.length} tone="warning" />
        <Metric icon={<History size={18} />} label={t('dashboard.pending', 'Pending changes')} value={data.pending_changes.length} tone="accent" />
        <Metric icon={<CheckCircle2 size={18} />} label={t('dashboard.rules', 'Selection rules')} value={data.provider_model_rules.length} />
      </section>

      <section className="grid gap-4 xl:grid-cols-[1fr_1fr]">
        <div className="shell-card p-4">
          <h2 className="mb-3 text-sm font-bold">{t('dashboard.publishReadiness', 'Publish readiness')}</h2>
          <div className="grid gap-3 sm:grid-cols-4">
            <Readiness label={t('summary.create', 'Create')} value={diff.create} />
            <Readiness label={t('summary.update', 'Update')} value={diff.update} />
            <Readiness label={t('summary.disable', 'Disable')} value={diff.disable} />
            <Readiness label={t('status.blocked', 'Blocked')} value={data.warnings.filter((warning) => warning.severity === 'blocked').length} />
          </div>
          <div className="mt-4 rounded-md border border-[color:var(--cp-border)] p-3 text-sm text-[color:var(--cp-muted)]">
            {t('publish.impact', 'Impact')}: {preview.impact.providers} providers, {preview.impact.model_rules} model rules
          </div>
          <Link className="mt-4 hidden rounded-md bg-[color:var(--cp-accent)] px-3 py-2 text-sm font-bold text-white md:inline-flex" to="/publish">
            {t('action.previewPublish', 'Preview publish')}
          </Link>
          <div className="mt-4 rounded-md border border-[color:var(--cp-border)] px-3 py-2 text-sm font-semibold text-[color:var(--cp-muted)] md:hidden">
            {t('mobile.publishUnavailable', 'Publishing is available on desktop only.')}
          </div>
        </div>
        <div className="shell-card p-4">
          <h2 className="mb-3 text-sm font-bold">{t('dashboard.recentChanges', 'Recent changes')}</h2>
          <div className="space-y-3">
            {visibleLogs.map((log) => (
              <div className="rounded-md border border-[color:var(--cp-border)] p-3" key={log.change_id}>
                <div className="flex items-center justify-between gap-2">
                  <span className="text-sm font-semibold">{log.summary}</span>
                  <StatusBadge>{log.to_revision}</StatusBadge>
                </div>
                <div className="mt-1 text-xs text-[color:var(--cp-muted)]">{log.operator_id} · {formatDate(log.created_at)}</div>
              </div>
            ))}
          </div>
        </div>
      </section>
    </div>
  )
}

function Metric({
  icon,
  label,
  value,
  tone = 'neutral',
}: {
  icon: React.ReactNode
  label: string
  value: number
  tone?: 'neutral' | 'warning' | 'accent'
}) {
  const color = tone === 'warning' ? 'var(--cp-warning)' : tone === 'accent' ? 'var(--cp-accent)' : 'var(--cp-ink-accent)'
  return (
    <div className="shell-card p-4">
      <div className="mb-4 flex h-9 w-9 items-center justify-center rounded-md border border-[color:var(--cp-border)]" style={{ color }}>
        {icon}
      </div>
      <div className="text-2xl font-bold">{value}</div>
      <div className="mt-1 text-xs font-semibold text-[color:var(--cp-muted)]">{label}</div>
    </div>
  )
}

function Readiness({ label, value }: { label: string; value: number }) {
  return (
    <div className="rounded-md border border-[color:var(--cp-border)] p-3">
      <div className="text-xl font-bold">{value}</div>
      <div className="text-xs text-[color:var(--cp-muted)]">{label}</div>
    </div>
  )
}
