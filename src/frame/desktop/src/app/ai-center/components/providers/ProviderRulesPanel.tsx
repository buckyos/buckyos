import './ProviderRulesPanel.css'
import { useEffect, useMemo, useRef, useState } from 'react'
import { Check, Code2, Copy, List, Pencil } from 'lucide-react'
import { useI18n } from '../../../../i18n/provider'
import { useAICCStore } from '../../hooks/use-aicc-store'
import type { ProviderView } from '../../../../api/aicc_mgr'
import { LongField } from '../shared/LongField'
import { ModelRuleEditor } from './ModelRuleEditor'
import {
  applyModelRuleEdits, createCustomRulesDraft, disabledRuleModels, setRuleModelEnabled, modelRuleDraft, paginateRuleModels,
  parseProviderRulesFile, providerRulesFile, ruleObject, ruleArray, rulesJson, ruleModelDefinition, validateRuleEditor,
  type CustomRulesDraft, type ModelRuleDraft, type ProviderRuleOptions,
} from '../../../../api/custom_provider_rules'

const EMPTY_OPTIONS: ProviderRuleOptions = { models: [], tasks: [], adapters: [] }

function CompactRuleList({ values, empty, onMore }: { values: string[]; empty: string; onMore: () => void }) {
  const { t } = useI18n()
  return values.length ? <ul className="flex min-w-0 flex-col gap-1 text-xs">
    {values.slice(0, 2).map((value) => <li key={value} title={value} className="truncate">{value}</li>)}
    {values.length > 2 && <li><button type="button" onClick={onMore} className="font-medium" style={{ color: 'var(--cp-accent)' }}>
      {t('aiCenter.rules.moreItems', '+{{count}} more', { count: values.length - 2 })}
    </button></li>}
  </ul> : <span className="text-xs" style={{ color: 'var(--cp-muted)' }}>{empty}</span>
}

function parseObject(source: string) { try { return ruleObject(JSON.parse(source)) } catch { return {} } }
function parseArray(source: string) { try { return ruleArray(JSON.parse(source)) } catch { return [] } }

export function ProviderRulesPanel({ provider, active: visible = true }: { provider: ProviderView; active?: boolean }) {
  const { t } = useI18n()
  const store = useAICCStore()
  const current = useMemo(() => createCustomRulesDraft(provider.config), [provider.config])
  const [draft, setDraft] = useState(current)
  const [editing, setEditing] = useState(false)
  const [editRevision, setEditRevision] = useState(store.getSnapshot().settingsRevision)
  const [saving, setSaving] = useState(false)
  const [error, setError] = useState('')
  const [feedback, setFeedback] = useState('')
  const [query, setQuery] = useState('')
  const [page, setPage] = useState(0)
  const [pageSize, setPageSize] = useState(20)
  const [view, setView] = useState<'list' | 'json'>('list')
  const [selected, setSelected] = useState<string | null>(null)
  const [edits, setEdits] = useState<Record<string, Partial<ModelRuleDraft>>>({})
  const [sourceFile, setSourceFile] = useState('')
  const [options, setOptions] = useState<ProviderRuleOptions | null>(null)
  const [loading, setLoading] = useState(true)
  const [loadError, setLoadError] = useState('')
  const [attempt, setAttempt] = useState(0)
  const input = useRef<HTMLInputElement>(null)
  const active = editing ? draft : current
  const resolved = sourceFile ? {} : provider.config.resolved_model_rules ?? {}
  const definitions = options ?? EMPTY_OPTIONS
  const adapter = definitions.adapters.find((adapter) => adapter.id === provider.config.protocol_adapter_id)

  useEffect(() => {
    if (!visible || options) return
    let cancelled = false
    void store.fetchProviderRuleOptions().then((value) => {
      if (!cancelled) { setOptions(value); setLoading(false) }
    }).catch((cause: unknown) => {
      if (!cancelled) { setLoadError(cause instanceof Error ? cause.message : String(cause)); setLoading(false) }
    })
    return () => { cancelled = true }
  }, [visible, options, store, attempt])

  const retry = () => { setLoadError(''); setLoading(true); setAttempt((value) => value + 1) }
  const ids = useMemo(() => provider.config.discovered_model_ids?.length
    ? provider.config.discovered_model_ids
    : [...new Set([
      ...provider.status.discovered_models.map((model) => model.provider_model_id),
      ...((provider.config.instance_rules?.exclude_models ?? []) as string[]),
      ...(provider.inventory.filtered_capabilities ?? []).map((model) => model.provider_model_id),
    ])], [provider])
  const excluded = disabledRuleModels(active, resolved)
  const result = paginateRuleModels(ids, excluded, query, page, pageSize)
  const disabled = new Set(excluded)

  const materialize = (): CustomRulesDraft => parseProviderRulesFile(rulesJson(providerRulesFile(applyModelRuleEdits(active, edits, resolved))), active)
  let fullFile: CustomRulesDraft | null = null
  let previewError = ''
  try { fullFile = editing ? materialize() : active }
  catch (cause) { previewError = cause instanceof Error ? cause.message : String(cause) }
  const finishEditing = async () => {
    setError('')
    setSaving(true)
    try {
      if (!options) throw new Error(t('aiCenter.rules.optionsRequired', 'Load model and adapter definitions before applying changes.'))
      const file = materialize()
      validateRuleEditor(file, edits, resolved, options, provider.config.protocol_adapter_id)
      await store.updateProviderRules(provider, { rules_config_json: rulesJson(providerRulesFile(file)), rules_settings_revision: editRevision })
      setEditing(false); setEdits({}); setSourceFile(''); setSelected(null)
      setFeedback(t('aiCenter.rules.applied', 'Rules applied.'))
    } catch (cause) { setError(cause instanceof Error ? cause.message : String(cause)) }
    finally { setSaving(false) }
  }
  const exportFile = () => {
    try {
      const file = editing ? materialize() : active
      const url = URL.createObjectURL(new Blob([rulesJson(providerRulesFile(file))], { type: 'application/json' }))
      const anchor = document.createElement('a')
      anchor.href = url; anchor.download = `${provider.config.provider_instance_name}.provider.json`; anchor.click()
      setTimeout(() => URL.revokeObjectURL(url), 1000)
    } catch (cause) { setError(cause instanceof Error ? cause.message : String(cause)) }
  }
  const importFile = async (file: File) => {
    try {
      setError('')
      if (file.size > 5 * 1024 * 1024) throw new Error(t('aiCenter.rules.fileTooLarge', 'Rules file must be smaller than 5 MB.'))
      setDraft(parseProviderRulesFile(await file.text(), active)); setEdits({}); setSourceFile(file.name); setSelected(null); setPage(0)
    } catch (cause) { setError(cause instanceof Error ? cause.message : String(cause)) }
  }
  const updateModel = <K extends keyof ModelRuleDraft>(id: string, field: K, value: ModelRuleDraft[K]) => {
    setEdits((before) => ({ ...before, [id]: { ...before[id], [field]: value } })); setError('')
  }
  const empty = t('aiCenter.rules.notConfigured', 'Not configured')
  const header = 'px-3 py-3 text-xs font-medium'
  const cell = 'min-w-0 px-3 py-3 align-top'
  const selectedRow = selected ? { ...modelRuleDraft(active, selected, resolved[selected]), ...edits[selected] } : null

  return <section className="flex min-w-0 flex-col gap-4" aria-label={t('aiCenter.rules.title', 'Provider rules')}>
    <div className="flex flex-wrap items-center justify-between gap-3">
      <div role="group" aria-label={t('aiCenter.rules.previewMode', 'Configuration view')} className="flex rounded-lg border p-1" style={{ borderColor: 'var(--cp-border)' }}>
        {(['list', 'json'] as const).map((mode) => <button key={mode} type="button" aria-pressed={view === mode}
          onClick={() => setView(mode)} className="flex items-center gap-2 rounded-md px-3 py-2 text-xs font-medium"
          style={{ background: view === mode ? 'var(--cp-surface-2)' : 'transparent', color: view === mode ? 'var(--cp-accent)' : 'var(--cp-muted)' }}>
          {mode === 'list' ? <List size={15} /> : <Code2 size={15} />}
          {mode === 'list' ? t('aiCenter.rules.modelList', 'Model list') : t('aiCenter.rules.fullJson', 'Full configuration JSON')}
        </button>)}
      </div>
      <div className="flex flex-wrap gap-2">
        <button type="button" disabled={saving || loading || Boolean(loadError)} onClick={() => {
          if (editing) { void finishEditing(); return }
          setDraft(current); setEditRevision(store.getSnapshot().settingsRevision); setEdits({}); setError(''); setFeedback(''); setEditing(true)
        }} className="flex items-center gap-2 rounded-lg px-3 py-2 text-sm font-medium disabled:opacity-50"
          style={{ background: 'var(--cp-accent)', color: 'var(--cp-on-accent, white)' }}>
          {editing ? <Check size={16} /> : <Pencil size={16} />}
          {saving ? t('common.saving', 'Saving...') : editing ? t('aiCenter.rules.finish', 'Finish editing and apply') : t('aiCenter.rules.edit', 'Edit rules')}
        </button>
        {editing && <button type="button" disabled={saving} className="rounded-lg border px-3 py-2 text-sm"
          onClick={() => { setEditing(false); setEdits({}); setError(''); setSourceFile(''); setSelected(null) }}>{t('common.cancel', 'Cancel')}</button>}
        <button type="button" disabled={saving || !fullFile} onClick={exportFile} className="rounded-lg border px-3 py-2 text-sm">{t('aiCenter.rules.export', 'Export full JSON')}</button>
        {editing && <button type="button" disabled={saving} onClick={() => input.current?.click()} className="rounded-lg border px-3 py-2 text-sm">{t('aiCenter.rules.import', 'Import full JSON')}</button>}
      </div>
    </div>
    <input ref={input} type="file" accept=".json,application/json" className="hidden" aria-label={t('aiCenter.rules.import', 'Import full JSON')}
      disabled={!editing || saving} onChange={(event) => { const file = event.target.files?.[0]; if (file) void importFile(file); event.target.value = '' }} />
    <p className="text-xs" style={{ color: 'var(--cp-muted)' }}>{t('aiCenter.rules.structuredHelp', 'Configure one model at a time. Default operations are not saved as overrides. Thinking efforts are supported only when explicitly configured. Finish editing to apply the whole draft.')}</p>
    {loading && <p role="status" className="text-xs">{t('common.loading', 'Loading...')}</p>}
    {loadError && <p role="alert" style={{ color: 'var(--cp-danger)' }}>{loadError} <button type="button" onClick={retry} className="underline">{t('aiCenter.rules.retry', 'Retry')}</button></p>}
    {sourceFile && <p role="status" className="text-xs">{t('aiCenter.rules.imported', 'Imported, pending apply:')} {sourceFile}</p>}
    {error && <p role="alert" style={{ color: 'var(--cp-danger)' }}>{error}</p>}
    {feedback && <p role="status" className="text-xs">{feedback}</p>}
    {previewError && <p role="alert" className="text-xs" style={{ color: 'var(--cp-danger)' }}>{previewError}</p>}
    {view === 'json' ? <div className="flex min-w-0 flex-col gap-3 rounded-xl border p-4" style={{ borderColor: 'var(--cp-border)' }}>
      <div className="flex items-center justify-between gap-2">
        <span className="text-xs" style={{ color: 'var(--cp-muted)' }}>{editing ? t('aiCenter.rules.draftJson', 'Complete draft configuration') : t('aiCenter.rules.savedJson', 'Complete instance configuration')}</span>
        <button type="button" disabled={!fullFile} onClick={() => {
          if (fullFile) void navigator.clipboard.writeText(rulesJson(providerRulesFile(fullFile))).then(() => setFeedback(t('aiCenter.rules.copied', 'JSON copied.'))).catch((cause: unknown) => setError(String(cause)))
        }} className="flex items-center gap-1 rounded border px-3 py-1 text-xs"><Copy size={14} />{t('aiCenter.rules.copyJson', 'Copy JSON')}</button>
      </div>
      {fullFile && <pre aria-label={t('aiCenter.rules.fullJson', 'Full configuration JSON')} className="max-h-[65vh] overflow-auto whitespace-pre text-xs">{rulesJson(providerRulesFile(fullFile))}</pre>}
    </div> : <>
      <div className="flex items-center gap-3">
        <input type="search" aria-label={t('aiCenter.rules.search', 'Search model name')} placeholder={t('aiCenter.rules.search', 'Search model name')}
          value={query} onChange={(event) => { setQuery(event.target.value); setPage(0) }} className="min-w-0 flex-1 rounded-lg border p-2 text-sm" />
        <label className="text-xs">{t('aiCenter.rules.pageSize', 'Models per page')}<select className="ml-2 rounded border p-2" value={pageSize}
          onChange={(event) => { setPageSize(Number(event.target.value)); setPage(0) }}>{[10, 20, 50, 100].map((size) => <option key={size} value={size}>{size}</option>)}</select></label>
      </div>
      <div className="provider-rules-table overflow-x-auto rounded-xl border" style={{ borderColor: 'var(--cp-border)' }}>
        <table className="provider-rules-model-table w-full table-fixed text-left">
          <colgroup><col style={{ width: 54 }} /><col style={{ width: '20%' }} /><col style={{ width: '12%' }} /><col style={{ width: '18%' }} /><col style={{ width: '12%' }} /><col style={{ width: '12%' }} /><col style={{ width: 190 }} /></colgroup>
          <thead style={{ background: 'var(--cp-surface)' }}><tr>
            {[t('aiCenter.rules.enabled', 'Enabled'), t('aiCenter.rules.model', 'Model'), t('aiCenter.rules.routes', 'Task lists'), t('aiCenter.rules.operations', 'Protocol mappings'),
              t('aiCenter.rules.variants', 'Thinking level mappings'), t('aiCenter.rules.pricing', 'Pricing'), t('aiCenter.rules.actions', 'Actions')].map((label, index) => <th key={label} className={index === 6 ? `${header} sticky right-0` : header} style={{ color: 'var(--cp-muted)', background: 'var(--cp-surface)' }}>{label}</th>)}
          </tr></thead>
          <tbody>{result.models.map((id) => {
            const row = { ...modelRuleDraft(active, id, resolved[id]), ...edits[id] }
            const metadata = ruleModelDefinition(id, row, resolved[id] ?? {}, definitions)
            const mapping = parseObject(row.operations)
            const disabledApiTypes = (() => { try { return JSON.parse(row.disabled_api_types) as string[] } catch { return [] } })()
            const apiTypes = [...new Set([...(metadata?.apiTypes ?? Object.keys(ruleObject(resolved[id]?.operations))).filter((type) => adapter?.operations.some((operation) => operation.apiTypes.includes(type))), ...Object.keys(mapping)])]
            const protocol = apiTypes.map((type) => disabledApiTypes.includes(type) ? `${type} · ${t('aiCenter.rules.capabilityDisabled', 'disabled')}`
              : `${type} → ${mapping[type] ?? `${t('aiCenter.rules.default', 'default')} · ${adapter?.defaults[type] ?? '—'}`}`)
            const price = parseObject(row.pricing)
            const prices = price.currency ? [
              ...(price.input_token !== undefined ? [`${price.currency} · ${t('aiCenter.rules.inputRate', 'input')}: ${price.input_token}`] : []),
              ...(price.output_token !== undefined ? [`${price.currency} · ${t('aiCenter.rules.outputRate', 'output')}: ${price.output_token}`] : []),
              ...(price.amount !== undefined ? [`${price.currency} ${price.amount} / ${price.unit ?? 'request'}`] : []),
            ] : []
            return <tr key={id} data-testid="provider-rule-model" data-model={id} className="border-t" style={{ borderColor: 'var(--cp-border)' }}>
              <td className={`${cell} rule-enabled-cell`}>{editing ? <input type="checkbox" aria-label={`${t('aiCenter.rules.enabled', 'Enabled')} ${id}`} checked={!disabled.has(id)} disabled={saving}
                onChange={(event) => { const enabled = event.target.checked; setDraft((before) => setRuleModelEnabled(before, id, enabled, resolved)) }} />
                : <span className="text-xs" style={{ color: disabled.has(id) ? 'var(--cp-muted)' : 'var(--cp-success)' }}>{disabled.has(id) ? '—' : <Check size={15} />}</span>}</td>
              <td className={`${cell} rule-model-cell`}><LongField value={id} className="text-sm font-medium" copyable />
                <p className="mt-1 truncate text-xs" title={row.alias || metadata?.target} style={{ color: 'var(--cp-muted)' }}>{row.alias || metadata?.target || t('aiCenter.rules.unrecognized', 'Model identity is not resolved')}</p></td>
              <td className={cell} data-label={t('aiCenter.rules.routes', 'Task lists')}><CompactRuleList values={row.routes.split(',').map((value) => value.trim()).filter(Boolean)} empty={empty} onMore={() => setSelected(id)} /></td>
              <td className={cell} data-label={t('aiCenter.rules.operations', 'Protocol mappings')}><CompactRuleList values={protocol} empty={empty} onMore={() => setSelected(id)} /></td>
              <td className={cell} data-label={t('aiCenter.rules.variants', 'Thinking level mappings')}><CompactRuleList values={parseArray(row.variants).map((variant) => String(variant.variant).replace(/^reasoning-/, ''))} empty={empty} onMore={() => setSelected(id)} /></td>
              <td className={cell} data-label={t('aiCenter.rules.pricing', 'Pricing')}><CompactRuleList values={prices} empty={empty} onMore={() => setSelected(id)} /></td>
              <td className={`${cell} rule-action-cell sticky right-0`}><button type="button" onClick={() => setSelected(id)} className="flex items-center justify-center gap-2 whitespace-nowrap rounded-lg border px-3 py-2 text-xs font-semibold"
                style={{ color: 'var(--cp-accent)', borderColor: 'var(--cp-accent)', background: 'color-mix(in srgb, var(--cp-accent) 8%, transparent)' }}>
                <Pencil size={15} />{editing ? t('aiCenter.rules.configure', 'Configure') : t('aiCenter.rules.inspect', 'View parameters')}
              </button></td>
            </tr>
          })}</tbody>
        </table>
        {result.total === 0 && <p className="p-4 text-sm">{t('aiCenter.rules.empty', 'No discovered models match this search.')}</p>}
      </div>
      <div className="flex items-center justify-between text-xs"><span>{result.total} {t('aiCenter.rules.models', 'models')} · {result.page + 1} / {result.pages}</span>
        <div className="flex gap-3"><button type="button" disabled={result.page === 0} onClick={() => setPage(result.page - 1)}>{t('common.previous', 'Previous')}</button>
          <button type="button" disabled={result.page + 1 >= result.pages} onClick={() => setPage(result.page + 1)}>{t('common.next', 'Next')}</button></div>
      </div>
    </>}
    {visible && selected && selectedRow && <ModelRuleEditor id={selected} row={selectedRow} options={definitions} resolved={resolved[selected] ?? {}}
      adapterId={provider.config.protocol_adapter_id} editing={editing} saving={saving} loading={loading} loadError={loadError} onRetry={retry}
      onChange={(key, value) => updateModel(selected, key, value)} onClose={() => setSelected(null)} />}
  </section>
}
