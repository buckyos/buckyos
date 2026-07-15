import { useEffect, useMemo, useState } from 'react'
import { zodResolver } from '@hookform/resolvers/zod'
import { AlertTriangle, CheckCircle2, GitCompare, RefreshCcw, Rocket, ShieldCheck, SlidersHorizontal } from 'lucide-react'
import { useForm } from 'react-hook-form'
import { useNavigate, useSearchParams } from 'react-router-dom'
import { JsonViewer } from '../../components/json-viewer/JsonViewer'
import { StatusBadge } from '../../components/status/StatusBadge'
import { buildPublishPreview, previewSelectionRuleHits } from '../../datamodel/selectors'
import { summarizePendingChanges } from '../../datamodel/diff'
import { publishWizardInputSchema, type PublishWizardInput } from '../../datamodel/schemas'
import { useI18n } from '../../i18n/provider'
import { useProviderMetadataStore } from '../../state/useProviderMetadataStore'
import { useShellContext } from '../pageUtils'

export function PublishPage() {
  const { t } = useI18n()
  const {
    workspace,
    publishPreview,
    simulateConflict,
    markTechSourceStale,
    refreshPublishBase,
    completePublish,
  } = useProviderMetadataStore()
  const { setInspector } = useShellContext()
  const navigate = useNavigate()
  const [searchParams] = useSearchParams()
  const data = workspace.data!
  const [publishError, setPublishError] = useState('')
  const wizardProviderKey = searchParams.get('source') === 'wizard' ? searchParams.get('provider') ?? '' : ''
  const fallbackPreview = useMemo(() => buildPublishPreview(data), [data])
  const preview = publishPreview ?? fallbackPreview
  const diff = summarizePendingChanges(data.pending_changes)
  const wizardPendingChanges = useMemo(() => {
    if (!wizardProviderKey) {
      return []
    }
    return data.pending_changes.filter((change) => change.target_key === wizardProviderKey || change.change_key.includes(wizardProviderKey))
  }, [data.pending_changes, wizardProviderKey])
  const selectionHitCount = useMemo(() => {
    return data.provider_model_rules.reduce((count, rule) => count + previewSelectionRuleHits(data, rule).length, 0)
  }, [data])
  const keyRiskCount = preview.warnings.filter((warning) => warning.message_key.includes('key') || warning.message_key.includes('Duplicate')).length
  const activeRevision = data.edit_session?.service_role === 'ops' ? data.ops_revision : data.published_revision
  const hasRevisionConflict = Boolean(data.revision_conflict || (data.edit_session && data.edit_session.base_revision !== activeRevision))
  const form = useForm<PublishWizardInput>({
    resolver: zodResolver(publishWizardInputSchema),
    defaultValues: {
      release_note: 'Publish imported provider metadata update',
      confirm_key_risk: false,
      confirm_stale_publish: false,
      confirm_final_publish: false,
    },
  })

  useEffect(() => {
    setInspector({
      title: t('publish.title', 'Publish Preview'),
      subtitle: `${preview.pending_change_count} pending changes`,
      status: preview.warnings.length ? t('status.warning', 'Warning') : t('status.published', 'Published'),
      json: preview.published_json[0] ?? preview.published_json,
    })
  }, [preview, setInspector, t])

  async function handlePublish(values: PublishWizardInput) {
    setPublishError('')
    try {
      await completePublish(values)
      navigate('/change-logs')
    } catch (error) {
      setPublishError(error instanceof Error ? error.message : String(error))
    }
  }

  return (
    <div className="space-y-4" data-testid="publish-page">
      <header className="flex items-center justify-between gap-3">
        <div>
          <h1 className="text-2xl font-bold">{t('publish.title', 'Publish Preview')}</h1>
          <p className="mt-1 text-sm text-[color:var(--cp-muted)]">{t('publish.happyPath', 'Edit action ready for publish preview')}</p>
        </div>
        <StatusBadge tone="warning">{preview.revision}</StatusBadge>
      </header>

      <section className="shell-card p-4 md:hidden">
        <StatusBadge tone="warning">{t('mobile.desktopOnly', 'Desktop only')}</StatusBadge>
        <p className="mt-3 text-sm leading-6 text-[color:var(--cp-muted)]">
          {t('mobile.publishUnavailable', 'Publishing is available on desktop only.')}
        </p>
      </section>

      {wizardProviderKey && (
        <section className="shell-card border border-[color:var(--cp-accent)] p-4">
          <div className="flex flex-wrap items-center justify-between gap-3">
            <div>
              <StatusBadge tone="success">{t('status.saved', 'Saved')}</StatusBadge>
              <h2 className="mt-2 text-sm font-bold">{t('wizard.previewReady', 'Provider wizard changes are ready for publish preview')}</h2>
              <p className="mt-1 text-sm text-[color:var(--cp-muted)]">
                {wizardProviderKey} · {wizardPendingChanges.length} {t('publish.pendingChanges', 'pending changes')}
              </p>
            </div>
            <button className="rounded-md border border-[color:var(--cp-border)] px-3 py-2 text-xs font-bold" onClick={() => navigate(`/providers/wizard?provider=${encodeURIComponent(wizardProviderKey)}`)} type="button">
              {t('action.edit', 'Edit')}
            </button>
          </div>
          {wizardPendingChanges.length > 0 && (
            <div className="mt-3 grid gap-2 md:grid-cols-3">
              {wizardPendingChanges.map((change) => (
                <div className="rounded-md border border-[color:var(--cp-border)] p-2 text-sm" key={change.change_key}>
                  <StatusBadge tone={change.risk === 'blocked' ? 'danger' : change.risk === 'warning' ? 'warning' : 'success'}>{change.action}</StatusBadge>
                  <div className="mt-2 font-mono text-xs text-[color:var(--cp-muted)]">{change.target_type}</div>
                  <div className="mt-1">{change.summary}</div>
                </div>
              ))}
            </div>
          )}
        </section>
      )}

      <section className="hidden gap-4 md:grid lg:grid-cols-3">
        <Panel title={t('publish.diff', 'Diff summary')} icon={<GitCompare size={18} />}>
          <div className="grid grid-cols-2 gap-2 text-sm">
            <Fact label={t('summary.create', 'Create')} value={diff.create} />
            <Fact label={t('summary.update', 'Update')} value={diff.update} />
            <Fact label={t('summary.delete', 'Delete')} value={diff.delete} />
            <Fact label={t('summary.disable', 'Disable')} value={diff.disable} />
          </div>
        </Panel>
        <Panel title={t('publish.impact', 'Impact')} icon={<CheckCircle2 size={18} />}>
          <div className="space-y-2 text-sm">
            <Fact label={t('summary.providers', 'Providers')} value={preview.impact.providers} />
            <Fact label={t('summary.modelRules', 'Model rules')} value={preview.impact.model_rules} />
            <Fact label={t('publish.hitModels', 'Matched models')} value={selectionHitCount} />
            <Fact label={t('summary.dictionaries', 'Dictionaries')} value={preview.impact.dictionaries} />
          </div>
        </Panel>
        <Panel title={t('publish.risks', 'Risk checks')} icon={<AlertTriangle size={18} />}>
          <div className="space-y-2">
            <div className="rounded-md border border-[color:var(--cp-border)] p-2 text-sm">
              <StatusBadge tone={keyRiskCount ? 'warning' : 'success'}>{keyRiskCount}</StatusBadge>
              <div className="mt-2">{t('publish.keyRisks', 'Key field risk area')}</div>
            </div>
            {preview.warnings.map((warning) => (
              <button
                className="block w-full rounded-md border border-[color:var(--cp-border)] p-2 text-left text-sm hover:border-[color:var(--cp-accent)]"
                key={warning.warning_key}
                onClick={() => navigate(getWarningPath(warning.target_type))}
              >
                <StatusBadge tone={warning.severity === 'blocked' ? 'danger' : 'warning'}>{warning.severity}</StatusBadge>
                <div className="mt-2">{t(warning.message_key, warning.message_key)}</div>
                {warning.detail && <div className="mt-1 font-mono text-xs text-[color:var(--cp-muted)]">{warning.detail}</div>}
              </button>
            ))}
          </div>
        </Panel>
      </section>

      <section className="hidden gap-4 md:grid lg:grid-cols-3">
        <Panel title={t('publish.opsCounts', 'Operations publish counts')} icon={<SlidersHorizontal size={18} />}>
          <div className="space-y-2 text-sm">
            <Fact label={t('publish.visibleProviders', 'Visible providers')} value={preview.ops_impact.visible_providers} />
            <Fact label={t('publish.visibleModels', 'Visible models')} value={preview.ops_impact.visible_models} />
            <Fact label={t('publish.overlayHits', 'Overlay hits')} value={preview.ops_impact.overlay_hits} />
          </div>
        </Panel>
        <Panel title={t('publish.disabledByOps', 'Removed by operations overlay')} icon={<AlertTriangle size={18} />}>
          <div className="space-y-2 text-sm">
            <Fact label={t('publish.disabledProviders', 'Disabled providers')} value={preview.ops_impact.disabled_providers} />
            <Fact label={t('publish.disabledModels', 'Disabled models')} value={preview.ops_impact.disabled_models} />
            <Fact label={t('publish.discardedFields', 'Discarded technical fields')} value={preview.ops_impact.discarded_technical_fields} />
          </div>
        </Panel>
        <Panel title={t('publish.staleConfirm', 'Stale publish confirmation')} icon={<RefreshCcw size={18} />}>
          <div className="space-y-3 text-sm">
            <StatusBadge tone={preview.ops_impact.source_stale ? 'warning' : 'success'}>
              {preview.ops_impact.source_stale ? t('status.stale', 'Stale') : t('techSource.cacheUsable', 'Cache usable')}
            </StatusBadge>
            <div className="text-[color:var(--cp-muted)]">{t('publish.staleDetail', 'Operations publish can proceed with a stale technical source only after explicit confirmation.')}</div>
            <button className="inline-flex items-center gap-2 rounded-md border border-[color:var(--cp-border)] px-3 py-2 text-xs font-bold" onClick={markTechSourceStale} type="button">
              <AlertTriangle size={14} />
              {t('publish.simulateStale', 'Simulate stale source')}
            </button>
          </div>
        </Panel>
      </section>

      <section className="hidden gap-4 md:grid xl:grid-cols-[minmax(0,1fr)_380px]">
        <div className="shell-card p-4">
          <div className="mb-4 flex items-center gap-2 text-sm font-bold">
            <ShieldCheck size={18} className="text-[color:var(--cp-accent)]" />
            {t('publish.wizard', 'Publish Wizard')}
          </div>
          {hasRevisionConflict && (
            <div className="mb-4 rounded-md border border-[color:var(--cp-danger)] p-3 text-sm" data-testid="revision-conflict-banner">
              <StatusBadge tone="danger">{t('publish.conflict', 'Base revision expired')}</StatusBadge>
              <div className="mt-2 text-[color:var(--cp-muted)]">
                {t('publish.conflictDetail', 'Direct publish is blocked until the preview is refreshed against the latest revision.')}
              </div>
              <button className="mt-3 inline-flex items-center gap-2 rounded-md bg-[color:var(--cp-accent)] px-3 py-2 text-xs font-bold text-white" onClick={refreshPublishBase}>
                <RefreshCcw size={14} />
                {t('publish.refreshPreview', 'Refresh preview')}
              </button>
            </div>
          )}
          <div className="grid gap-3 md:grid-cols-2">
            <Checklist title={t('publish.schemaValidation', 'Schema validation')} status={preview.warnings.some((warning) => warning.severity === 'blocked') ? 'blocked' : 'ready'} />
            <Checklist title={t('publish.keyRiskConfirm', 'Key field risk confirmation')} status={keyRiskCount ? 'warning' : 'ready'} />
            <Checklist title={t('publish.impactReview', 'Impact review')} status={preview.pending_change_count ? 'ready' : 'blocked'} />
            <Checklist title={t('publish.testAdvice', 'Test suggestions')} status="ready" />
          </div>
          <div className="mt-4 rounded-md border border-[color:var(--cp-border)] p-3 text-sm">
            <div className="font-bold">{t('publish.testAdvice', 'Test suggestions')}</div>
            <ul className="mt-2 list-disc space-y-1 pl-5 text-[color:var(--cp-muted)]">
              <li>{t('publish.testImport', 'Verify imported selectors still hit expected source models.')}</li>
              <li>{t('publish.testJson', 'Compare client driver metadata models, patterns, defaults, variants, and version rules before rollout.')}</li>
              <li>{t('publish.testClient', 'Run one client resolver smoke check after publish.')}</li>
            </ul>
          </div>
        </div>

        <form className="shell-card space-y-3 p-4" onSubmit={form.handleSubmit(handlePublish)}>
          <h2 className="text-sm font-bold">{t('publish.finalConfirm', 'Final confirmation')}</h2>
          <label className="block text-sm font-bold" htmlFor="release-note">{t('publish.releaseNote', 'Publish note')}</label>
          <textarea
            id="release-note"
            className="min-h-24 w-full rounded-md border border-[color:var(--cp-border)] bg-[color:var(--cp-surface)] px-3 py-2 text-sm"
            {...form.register('release_note')}
          />
          <label className="flex items-start gap-2 text-sm">
            <input className="mt-1" type="checkbox" {...form.register('confirm_key_risk')} />
            <span>{t('publish.confirmKeyRisk', 'I reviewed key field risks and affected references.')}</span>
          </label>
          <label className="flex items-start gap-2 text-sm">
            <input className="mt-1" type="checkbox" {...form.register('confirm_stale_publish')} />
            <span>{t('publish.confirmStale', 'I reviewed stale source status and accept publishing from the current cache if needed.')}</span>
          </label>
          <label className="flex items-start gap-2 text-sm">
            <input className="mt-1" type="checkbox" {...form.register('confirm_final_publish')} />
            <span>{t('publish.confirmFinal', 'I understand this writes a mock change log and clears pending changes.')}</span>
          </label>
          <button
            className="inline-flex w-full items-center justify-center gap-2 rounded-md bg-[color:var(--cp-accent)] px-3 py-2 text-sm font-bold text-white disabled:cursor-not-allowed disabled:opacity-50"
            disabled={hasRevisionConflict || preview.pending_change_count === 0}
            type="submit"
          >
            <Rocket size={16} />
            {t('action.publish', 'Publish')}
          </button>
          <button className="inline-flex w-full items-center justify-center gap-2 rounded-md border border-[color:var(--cp-border)] px-3 py-2 text-sm font-bold" type="button" onClick={simulateConflict}>
            <AlertTriangle size={16} />
            {t('publish.simulateConflict', 'Simulate revision conflict')}
          </button>
          {(form.formState.errors.release_note || form.formState.errors.confirm_key_risk || form.formState.errors.confirm_final_publish) && (
            <p className="text-sm text-[color:var(--cp-danger)]">{t('publish.confirmRequired', 'Publish note and both confirmations are required.')}</p>
          )}
          {publishError && <p className="text-sm text-[color:var(--cp-danger)]">{publishError}</p>}
        </form>
      </section>

      <section className="shell-card hidden p-4 md:block">
        <h2 className="mb-3 text-sm font-bold">{t('publish.json', 'Client driver metadata JSON')}</h2>
        <JsonViewer value={preview.published_json} filename="driver-metadata-preview.json" />
      </section>
    </div>
  )
}

function Checklist({ title, status }: { title: string; status: 'ready' | 'warning' | 'blocked' }) {
  return (
    <div className="rounded-md border border-[color:var(--cp-border)] p-3 text-sm">
      <StatusBadge tone={status === 'blocked' ? 'danger' : status === 'warning' ? 'warning' : 'success'}>{status}</StatusBadge>
      <div className="mt-2 font-semibold">{title}</div>
    </div>
  )
}

function getWarningPath(targetType: string) {
  if (targetType === 'provider') {
    return '/providers'
  }
  if (targetType === 'nick_rule') {
    return '/nick-rules'
  }
  if (targetType === 'logical_directory') {
    return '/logical-directory'
  }
  if (targetType === 'dictionary') {
    return '/dictionaries'
  }
  return '/models'
}

function Panel({ title, icon, children }: { title: string; icon: React.ReactNode; children: React.ReactNode }) {
  return (
    <div className="shell-card p-4">
      <div className="mb-3 flex items-center gap-2 text-sm font-bold">
        <span className="text-[color:var(--cp-accent)]">{icon}</span>
        {title}
      </div>
      {children}
    </div>
  )
}

function Fact({ label, value }: { label: string; value: number }) {
  return (
    <div className="flex items-center justify-between rounded-md border border-[color:var(--cp-border)] px-3 py-2">
      <span className="text-[color:var(--cp-muted)]">{label}</span>
      <span className="font-bold">{value}</span>
    </div>
  )
}
