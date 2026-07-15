import { useEffect, useMemo, useRef, useState } from 'react'
import { zodResolver } from '@hookform/resolvers/zod'
import { FileInput, RotateCcw, Save, Send, Trash2, Upload } from 'lucide-react'
import { useForm } from 'react-hook-form'
import { useNavigate } from 'react-router-dom'
import { JsonViewer } from '../../components/json-viewer/JsonViewer'
import { StatusBadge } from '../../components/status/StatusBadge'
import { importPlanInputSchema, type ImportPlanInput } from '../../datamodel/schemas'
import { useI18n } from '../../i18n/provider'
import { useProviderMetadataStore } from '../../state/useProviderMetadataStore'
import { useShellContext } from '../pageUtils'

const samplePlan = `# OpenRouter metadata update
actions:
  - action: upsert_provider
  - action: disable_provider
  - action: upsert_model_param_rule
  - action: delete_model_param_rule
  - action: include_models
  - action: exclude_models
  - action: set_model_nick
  - action: upsert_variant
  - action: upsert_version_rule
  - action: set_logical_mounts
  - action: upsert_logical_directory
  - action: delete_logical_directory
  - action: move_logical_directory
  - action: upsert_api_type
  - action: set_api_types
  - action: delete_api_type
  - action: upsert_capability
  - action: set_capabilities
  - action: delete_capability`

export function ImportPlanPage() {
  const { t } = useI18n()
  const navigate = useNavigate()
  const { setInspector } = useShellContext()
  const {
    workspace,
    importPlan,
    saveImportDraft,
    restoreImportDraft,
    discardImportDraft,
    runPublishPreview,
  } = useProviderMetadataStore()
  const data = workspace.data!
  const [message, setMessage] = useState('')
  const [actionPage, setActionPage] = useState(1)
  const fileInputRef = useRef<HTMLInputElement | null>(null)
  const form = useForm<ImportPlanInput>({
    resolver: zodResolver(importPlanInputSchema),
    defaultValues: {
      title: 'OpenRouter July update plan',
      text: samplePlan,
    },
  })
  const result = data.import_plan_result
  const blockedActions = useMemo(() => result?.actions.filter((action) => action.errors.length > 0) ?? [], [result])
  const actionPageSize = 25
  const actionRows = result?.actions ?? []
  const actionPageCount = Math.max(1, Math.ceil(actionRows.length / actionPageSize))
  const pagedActions = actionRows.slice((actionPage - 1) * actionPageSize, actionPage * actionPageSize)

  useEffect(() => {
    setInspector({
      title: t('import.title', 'Import Plan'),
      subtitle: result ? `${result.supported_count}/${result.action_count} actions supported` : t('import.noPlan', 'No imported plan'),
      status: blockedActions.length ? t('status.blocked', 'Blocked') : t('status.editing', 'Editing'),
      json: result ?? data.import_plan_draft?.parse_result ?? null,
    })
  }, [blockedActions.length, data.import_plan_draft?.parse_result, result, setInspector, t])

  async function handleApply(values: ImportPlanInput) {
    await importPlan(values)
    setMessage(t('import.applied', 'Actions were added to pending changes'))
  }

  async function handleSaveDraft() {
    const values = form.getValues()
    await saveImportDraft({ ...values, plan_id: 'import-plan-draft' })
    setMessage(t('import.draftSaved', 'Draft saved'))
  }

  async function handleRestoreDraft() {
    const draft = data.import_plan_draft
    await restoreImportDraft()
    if (draft) {
      form.reset({ title: draft.title, text: draft.text })
    }
    setMessage(t('import.draftRestored', 'Draft restored'))
  }

  async function handleDiscardDraft() {
    await discardImportDraft()
    setMessage(t('import.draftDiscarded', 'Draft discarded'))
  }

  async function handlePreviewPublish() {
    await runPublishPreview()
    navigate('/publish')
  }

  async function handleUploadFile(file: File | null) {
    if (!file) {
      return
    }
    if (!/\.(ya?ml|md)$/i.test(file.name)) {
      setMessage(t('import.unsupportedFile', 'Only .yaml, .yml, and .md files are supported.'))
      return
    }
    const text = await file.text()
    if (!text.trim()) {
      setMessage(t('import.emptyFile', 'The selected file is empty.'))
      return
    }
    form.setValue('title', file.name.replace(/\.(ya?ml|md)$/i, ''), { shouldValidate: true, shouldDirty: true })
    form.setValue('text', text, { shouldValidate: true, shouldDirty: true })
    setMessage(t('import.fileLoaded', 'File loaded into the import editor.'))
  }

  return (
    <div className="space-y-4" data-testid="import-plan-page">
      <header className="flex flex-col justify-between gap-3 lg:flex-row lg:items-center">
        <div>
          <h1 className="text-2xl font-bold">{t('import.title', 'Import Plan')}</h1>
          <p className="mt-1 text-sm text-[color:var(--cp-muted)]">
            {t('import.subtitle', 'Paste YAML or Markdown, parse supported actions, then dispatch them to pending changes.')}
          </p>
        </div>
        <div className="flex flex-wrap items-center gap-2">
          {data.import_plan_draft && <StatusBadge tone="warning">{t('import.draftAvailable', 'Draft available')}</StatusBadge>}
          {result && <StatusBadge tone={result.error_count ? 'danger' : 'success'}>{result.supported_count}/{result.action_count}</StatusBadge>}
        </div>
      </header>

      <section className="grid gap-4 xl:grid-cols-[minmax(0,1fr)_420px]">
        <form className="shell-card space-y-3 p-4" onSubmit={form.handleSubmit(handleApply)}>
          <label className="block text-sm font-bold" htmlFor="import-title">{t('import.planTitle', 'Plan title')}</label>
          <input
            id="import-title"
            className="w-full rounded-md border border-[color:var(--cp-border)] bg-[color:var(--cp-surface)] px-3 py-2 text-sm"
            {...form.register('title')}
          />
          <label className="block text-sm font-bold" htmlFor="import-text">{t('import.planText', 'YAML or Markdown')}</label>
          <textarea
            id="import-text"
            className="min-h-[360px] w-full rounded-md border border-[color:var(--cp-border)] bg-[color:var(--cp-surface)] px-3 py-2 font-mono text-xs leading-5"
            spellCheck={false}
            {...form.register('text')}
          />
          <div className="flex flex-wrap items-center gap-2">
            <input
              ref={fileInputRef}
              accept=".yaml,.yml,.md"
              className="hidden"
              onChange={(event) => {
                void handleUploadFile(event.target.files?.[0] ?? null)
                event.target.value = ''
              }}
              type="file"
            />
            <button className="inline-flex items-center gap-2 rounded-md border border-[color:var(--cp-border)] px-3 py-2 text-sm font-bold" type="button" onClick={() => fileInputRef.current?.click()}>
              <Upload size={16} />
              {t('import.uploadFile', 'Upload file')}
            </button>
            <button className="inline-flex items-center gap-2 rounded-md bg-[color:var(--cp-accent)] px-3 py-2 text-sm font-bold text-white" type="submit">
              <FileInput size={16} />
              {t('import.apply', 'Import update plan')}
            </button>
            <button className="inline-flex items-center gap-2 rounded-md border border-[color:var(--cp-border)] px-3 py-2 text-sm font-bold" type="button" onClick={handleSaveDraft}>
              <Save size={16} />
              {t('action.saveDraft', 'Save draft')}
            </button>
            <button className="inline-flex items-center gap-2 rounded-md border border-[color:var(--cp-border)] px-3 py-2 text-sm font-bold" disabled={!data.import_plan_draft} type="button" onClick={handleRestoreDraft}>
              <RotateCcw size={16} />
              {t('import.restoreDraft', 'Restore draft')}
            </button>
            <button className="inline-flex items-center gap-2 rounded-md border border-[color:var(--cp-border)] px-3 py-2 text-sm font-bold" disabled={!data.import_plan_draft && !result} type="button" onClick={handleDiscardDraft}>
              <Trash2 size={16} />
              {t('action.discard', 'Discard')}
            </button>
          </div>
          {(form.formState.errors.title || form.formState.errors.text) && (
            <p className="text-sm text-[color:var(--cp-danger)]">{t('import.validationError', 'Plan title and text are required before import.')}</p>
          )}
          {message && <p className="text-sm text-[color:var(--cp-muted)]">{message}</p>}
        </form>

        <aside className="space-y-4">
          <div className="shell-card p-4">
            <h2 className="mb-3 text-sm font-bold">{t('import.validation', 'Validation')}</h2>
            <div className="grid grid-cols-2 gap-2 text-sm">
              <Fact label={t('import.actions', 'Actions')} value={result?.action_count ?? 0} />
              <Fact label={t('import.supported', 'Supported')} value={result?.supported_count ?? 0} />
              <Fact label={t('import.errors', 'Errors')} value={result?.error_count ?? 0} />
              <Fact label={t('top.pending', 'Pending changes')} value={data.pending_changes.length} />
            </div>
            {blockedActions.length > 0 && (
              <div className="mt-3 rounded-md border border-[color:var(--cp-danger)] p-3 text-sm">
                <StatusBadge tone="danger">{t('status.blocked', 'Blocked')}</StatusBadge>
                <div className="mt-2">{blockedActions[0].errors[0]}</div>
              </div>
            )}
          </div>
          <div className="shell-card p-4">
            <h2 className="mb-3 text-sm font-bold">{t('import.publishGate', 'Publish gate')}</h2>
            <p className="mb-3 text-sm text-[color:var(--cp-muted)]">
              {t('import.noDirectPublish', 'Import Plan cannot publish directly. Review pending changes and publish from the wizard.')}
            </p>
            <button className="inline-flex w-full items-center justify-center gap-2 rounded-md bg-[color:var(--cp-accent)] px-3 py-2 text-sm font-bold text-white" onClick={handlePreviewPublish}>
              <Send size={16} />
              {t('action.previewPublish', 'Preview publish')}
            </button>
          </div>
        </aside>
      </section>

      <section className="shell-card overflow-hidden">
        <div className="border-b border-[color:var(--cp-border)] p-4">
          <h2 className="text-sm font-bold">{t('import.actionList', 'Action list')}</h2>
        </div>
        <div className="overflow-x-auto">
          <table className="min-w-full text-left text-sm">
            <thead className="bg-[color:var(--cp-surface-muted)] text-xs uppercase text-[color:var(--cp-muted)]">
              <tr>
                <th className="px-4 py-3">{t('import.action', 'Action')}</th>
                <th className="px-4 py-3">{t('table.summary', 'Summary')}</th>
                <th className="px-4 py-3">{t('rules.selector', 'Selector')}</th>
                <th className="px-4 py-3">{t('rules.hits', 'Hits')}</th>
                <th className="px-4 py-3">{t('import.impact', 'Impact')}</th>
                <th className="px-4 py-3">{t('publish.risks', 'Risk checks')}</th>
              </tr>
            </thead>
            <tbody>
              {pagedActions.map((action) => (
                <tr className="border-t border-[color:var(--cp-border)]" key={action.action_key}>
                  <td className="px-4 py-3 font-mono text-xs">{action.raw_action}</td>
                  <td className="px-4 py-3">
                    <div>{action.summary}</div>
                    <div className="mt-1 text-xs text-[color:var(--cp-muted)]">
                      {t('import.sourceRecord', 'Source')}: {action.source_record ?? '-'}
                    </div>
                    {action.match_type && (
                      <div className="mt-1 text-xs text-[color:var(--cp-muted)]">
                        match_type={action.match_type} priority={action.priority ?? '-'}
                      </div>
                    )}
                    {action.fallback_behavior && (
                      <div className="mt-1 text-xs text-[color:var(--cp-muted)]">
                        {t('import.fallback', 'Fallback')}: {action.fallback_behavior}
                      </div>
                    )}
                    {action.published_selector && (
                      <div className="mt-1 font-mono text-xs text-[color:var(--cp-muted)]">
                        {t('import.publishedSelector', 'Published selector')}: {action.published_selector}
                      </div>
                    )}
                    {action.errors.map((error) => <div className="mt-1 text-xs text-[color:var(--cp-danger)]" key={error}>{error}</div>)}
                  </td>
                  <td className="px-4 py-3 font-mono text-xs">{action.selector ?? '-'}</td>
                  <td className="px-4 py-3">
                    <div className="font-bold">{action.hit_count}</div>
                    <div className="text-xs text-[color:var(--cp-muted)]">{action.samples.join(', ') || '-'}</div>
                  </td>
                  <td className="px-4 py-3">
                    <div className="font-bold">{action.affected_count}</div>
                    <div className="text-xs text-[color:var(--cp-muted)]">{action.reference_samples.join(', ') || '-'}</div>
                    <div className="mt-2 space-y-1">
                      {action.field_changes.map((change) => (
                        <div className="rounded border border-[color:var(--cp-border)] px-2 py-1 font-mono text-[11px]" key={`${action.action_key}-${change.field}`}>
                          {change.field}: {change.before} {'->'} {change.after}
                        </div>
                      ))}
                    </div>
                  </td>
                  <td className="px-4 py-3">
                    <StatusBadge tone={action.risk === 'blocked' ? 'danger' : action.risk === 'warning' ? 'warning' : 'success'}>{action.risk}</StatusBadge>
                  </td>
                </tr>
              ))}
              {!result && (
                <tr>
                  <td className="px-4 py-8 text-center text-[color:var(--cp-muted)]" colSpan={6}>{t('import.empty', 'Import a plan to inspect actions.')}</td>
                </tr>
              )}
            </tbody>
          </table>
        </div>
        {actionRows.length > actionPageSize && (
          <div className="flex items-center justify-end gap-2 border-t border-[color:var(--cp-border)] px-4 py-3 text-sm text-[color:var(--cp-muted)]">
            <button className="rounded-md border border-[color:var(--cp-border)] px-3 py-1.5" disabled={actionPage <= 1} onClick={() => setActionPage((page) => Math.max(1, page - 1))} type="button">-</button>
            {t('pager.page', 'Page {{page}} of {{pages}}', { page: actionPage, pages: actionPageCount })}
            <button className="rounded-md border border-[color:var(--cp-border)] px-3 py-1.5" disabled={actionPage >= actionPageCount} onClick={() => setActionPage((page) => Math.min(actionPageCount, page + 1))} type="button">+</button>
          </div>
        )}
      </section>

      {result && (
        <section className="shell-card p-4">
          <h2 className="mb-3 text-sm font-bold">{t('inspector.json', 'Published JSON')}</h2>
          <JsonViewer value={result} />
        </section>
      )}
    </div>
  )
}

function Fact({ label, value }: { label: string; value: number }) {
  return (
    <div className="rounded-md border border-[color:var(--cp-border)] px-3 py-2">
      <div className="text-xs text-[color:var(--cp-muted)]">{label}</div>
      <div className="text-lg font-bold">{value}</div>
    </div>
  )
}
