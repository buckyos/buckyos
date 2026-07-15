import { zodResolver } from '@hookform/resolvers/zod'
import { Cable, RefreshCw, Wifi } from 'lucide-react'
import { useEffect } from 'react'
import { useForm } from 'react-hook-form'
import { StatusBadge } from '../../components/status/StatusBadge'
import { techSourceInputSchema, type TechSourceInput } from '../../datamodel/schemas'
import { useI18n } from '../../i18n/provider'
import { useProviderMetadataStore } from '../../state/useProviderMetadataStore'
import { formatDate } from '../pageUtils'

const inputClass = 'h-10 w-full rounded-md border border-[color:var(--cp-border)] bg-[color:var(--cp-surface)] px-3 text-sm text-[color:var(--cp-text)]'

export function TechSourcePage() {
  const { t } = useI18n()
  const { workspace, configureTechSource, testTechSource, syncSource } = useProviderMetadataStore()
  const data = workspace.data!
  const source = data.tech_source
  const form = useForm<TechSourceInput>({
    resolver: zodResolver(techSourceInputSchema),
    defaultValues: {
      service_url: source.service_url,
    },
  })

  useEffect(() => {
    form.reset({ service_url: source.service_url })
  }, [form, source.service_url])

  return (
    <div className="space-y-4" data-testid="tech-source-page">
      <header className="flex flex-wrap items-center justify-between gap-3">
        <div>
          <h1 className="text-2xl font-bold">{t('techSource.title', 'Tech Source')}</h1>
          <p className="mt-1 text-sm text-[color:var(--cp-muted)]">{t('techSource.subtitle', 'Configure the technical parameter sync source without calling a real backend.')}</p>
        </div>
        <StatusBadge tone={source.stale ? 'warning' : 'success'}>
          {source.stale ? t('status.stale', 'Stale') : t('techSource.cacheUsable', 'Cache usable')}
        </StatusBadge>
      </header>

      <form
        className="shell-card grid gap-3 p-4 lg:grid-cols-[minmax(0,1fr)_auto_auto_auto]"
        onSubmit={form.handleSubmit(configureTechSource)}
      >
        <label className="flex min-w-0 flex-col gap-1 text-xs font-semibold text-[color:var(--cp-muted)]">
          {t('techSource.serviceUrl', 'Technical parameter service URL')}
          <input className={inputClass} {...form.register('service_url')} />
          {form.formState.errors.service_url && <span className="text-[color:var(--cp-danger)]">{t('techSource.invalidUrl', 'Enter a valid service URL')}</span>}
        </label>
        <button className="inline-flex h-10 items-center justify-center gap-2 self-end rounded-md border border-[color:var(--cp-border)] px-3 text-sm font-semibold" type="submit">
          <Cable size={16} />
          {t('action.saveDraft', 'Save draft')}
        </button>
        <button className="inline-flex h-10 items-center justify-center gap-2 self-end rounded-md border border-[color:var(--cp-border)] px-3 text-sm font-semibold" onClick={testTechSource} type="button">
          <Wifi size={16} />
          {t('techSource.testConnection', 'Test connection')}
        </button>
        <button className="inline-flex h-10 items-center justify-center gap-2 self-end rounded-md bg-[color:var(--cp-accent)] px-3 text-sm font-semibold text-white" onClick={syncSource} type="button">
          <RefreshCw size={16} />
          {t('techSource.manualSync', 'Manual sync')}
        </button>
      </form>

      <section className="grid gap-4 md:grid-cols-2 xl:grid-cols-4">
        <Metric label={t('techSource.serviceUrl', 'Technical parameter service URL')} value={source.service_url} />
        <Metric label={t('top.sourceRevision', 'Source revision')} value={source.source_revision} />
        <Metric label={t('top.opsRevision', 'Operations revision')} value={source.ops_revision} />
        <Metric label={t('techSource.cacheRevision', 'Cache revision')} value={source.cache_revision} />
        <Metric label={t('techSource.lastSuccess', 'Last success')} value={source.last_success_at ? formatDate(source.last_success_at) : '-'} />
      </section>

      <section className="shell-card p-4">
        <h2 className="text-sm font-bold">{t('techSource.syncState', 'Sync state')}</h2>
        <div className="mt-3 grid gap-3 text-sm md:grid-cols-3">
          <div>
            <div className="text-xs font-semibold text-[color:var(--cp-muted)]">{t('techSource.lastAttempt', 'Last attempt')}</div>
            <div className="mt-1">{source.last_sync_at ? formatDate(source.last_sync_at) : '-'}</div>
          </div>
          <div>
            <div className="text-xs font-semibold text-[color:var(--cp-muted)]">{t('techSource.lastError', 'Last error')}</div>
            <div className="mt-1">{source.last_error ?? '-'}</div>
          </div>
          <div>
            <div className="text-xs font-semibold text-[color:var(--cp-muted)]">{t('techSource.failureCache', 'Failure fallback')}</div>
            <div className="mt-1">{t('techSource.failureCacheDetail', 'Previous cache remains available when sync fails.')}</div>
          </div>
        </div>
      </section>
    </div>
  )
}

function Metric({ label, value }: { label: string; value: string }) {
  return (
    <div className="shell-card p-4">
      <div className="text-xs font-semibold text-[color:var(--cp-muted)]">{label}</div>
      <div className="mt-2 break-all font-mono text-sm font-bold">{value}</div>
    </div>
  )
}
