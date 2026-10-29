import { Autocomplete, Drawer, TextField } from '@mui/material'
import { X } from 'lucide-react'
import { useI18n } from '../../../../i18n/provider'
import {
  ruleArray, ruleObject, ruleModelDefinition, rulesJson, scopedRequestRules,
  type ModelRuleDraft, type ProviderRuleOptions, type RuleObject,
} from '../../../../api/custom_provider_rules'

function JsonRuleInput({ label, value, onChange, disabled, kind }: {
  label: string; value: string; onChange: (value: string) => void; disabled: boolean; kind: 'object' | 'array' | 'price'
}) {
  const { t } = useI18n()
  let invalid = false
  try {
    const parsed: unknown = JSON.parse(value)
    invalid = kind === 'array' ? !Array.isArray(parsed)
      : kind === 'price' && parsed === null ? false
        : parsed === null || typeof parsed !== 'object' || Array.isArray(parsed)
  } catch { invalid = true }
  return <label className="flex min-w-0 flex-col gap-2 text-xs">
    <span>{label}</span>
    <textarea aria-label={label} aria-invalid={invalid} rows={5} value={value} disabled={disabled}
      onChange={(event) => onChange(event.target.value)} className="w-full rounded-lg border p-3 font-mono text-xs"
      style={{ borderColor: invalid ? 'var(--cp-danger)' : 'var(--cp-border)' }} />
    {invalid && <span role="alert" style={{ color: 'var(--cp-danger)' }}>{t('aiCenter.rules.invalidJson', 'Enter valid JSON of the required type.')}</span>}
  </label>
}

function parsedObject(source: string): RuleObject {
  try { return ruleObject(JSON.parse(source)) } catch { return {} }
}
function parsedArray(source: string): RuleObject[] {
  try { return ruleArray(JSON.parse(source)) } catch { return [] }
}

export function ModelRuleEditor({ id, row, options, resolved, adapterId, editing, saving, loading, loadError, onRetry, onChange, onClose }: {
  id: string; row: ModelRuleDraft; options: ProviderRuleOptions; resolved: RuleObject; adapterId: string
  editing: boolean; saving: boolean; loading: boolean; loadError: string; onRetry: () => void
  onChange: <K extends keyof ModelRuleDraft>(key: K, value: ModelRuleDraft[K]) => void
  onClose: () => void
}) {
  const { t } = useI18n()
  const metadata = ruleModelDefinition(id, row, resolved, options)
  const adapter = options.adapters.find((item) => item.id === adapterId)
  const operations = parsedObject(row.operations)
  const variants = parsedArray(row.variants)
  const disabledTypes = new Set<string>((() => { try { return JSON.parse(row.disabled_api_types) as string[] } catch { return [] } })())
  const knownTypes = metadata?.apiTypes.filter((apiType) => adapter?.operations.some((operation) => operation.apiTypes.includes(apiType))) ?? []
  const apiTypes = [...new Set([...knownTypes, ...Object.keys(operations), ...disabledTypes])].sort()
  const tasks = row.routes.split(',').map((task) => task.trim()).filter(Boolean)
  const canUseTasks = metadata?.apiTypes.includes('llm') && !metadata.directOnly
  const efforts = metadata?.efforts.filter((effort) => effort !== 'native') ?? []
  const unsupportedVariants = variants.filter((variant) => !efforts.includes(String(variant.variant).replace(/^reasoning-/, '')) || variant.model_driver !== metadata?.driver)
  const readOnly = !editing || saving
  const controlsDisabled = readOnly || loading || Boolean(loadError)
  const section = 'flex flex-col gap-3 rounded-xl border p-4'
  const updateVariant = (effort: string, enabled: boolean) => {
    const name = `reasoning-${effort}`
    const next = variants.filter((variant) => variant.variant !== name)
    if (enabled && metadata) next.push({
      model_driver: metadata.driver, variant: name,
      match: { provider_model_id: id },
      provider_options: {},
    })
    onChange('variants', rulesJson(next))
  }

  return <Drawer open anchor="right" onClose={onClose}
    slotProps={{ paper: { sx: { width: 'min(760px, 90vw)', background: 'var(--cp-bg)', color: 'var(--cp-text)' } } }}>
    <div className="flex h-full min-h-0 flex-col" role="region" aria-label={t('aiCenter.rules.modelSettings', 'Model configuration')}>
      <div className="flex items-start justify-between gap-3 border-b px-6 py-4" style={{ borderColor: 'var(--cp-border)' }}>
        <div className="min-w-0">
          <h2 className="break-all text-base font-semibold">{id}</h2>
          <p className="mt-1 break-all text-xs" style={{ color: 'var(--cp-muted)' }}>{metadata?.target ?? t('aiCenter.rules.unrecognized', 'Model identity is not resolved')} · {adapterId}</p>
          {editing && <p className="mt-2 text-xs" style={{ color: 'var(--cp-accent)' }}>{t('aiCenter.rules.draftHelp', 'Changes stay in the page draft. Finish editing on the main page to apply them.')}</p>}
        </div>
        <button type="button" onClick={onClose} aria-label={t('common.close', 'Close')} className="rounded-lg p-2"><X size={20} /></button>
      </div>
      <div className="flex min-h-0 flex-1 flex-col gap-4 overflow-y-auto p-6">
        {loading && <p role="status">{t('common.loading', 'Loading...')}</p>}
        {loadError && <div role="alert" style={{ color: 'var(--cp-danger)' }}>{loadError} <button type="button" onClick={onRetry} className="underline">{t('aiCenter.rules.retry', 'Retry')}</button></div>}
        <section className={section} style={{ borderColor: 'var(--cp-border)' }}>
          <h3 className="text-sm font-semibold">{t('aiCenter.rules.aliases', 'Alias mappings')}</h3>
          <Autocomplete freeSolo size="small" disabled={controlsDisabled} options={options.models.map((model) => model.target)}
            inputValue={row.alias} onInputChange={(_, value) => onChange('alias', value)}
            renderInput={(params) => <TextField {...params} label={t('aiCenter.rules.builtinModel', 'Mapped built-in model')} placeholder={metadata?.target ?? 'manufacturer/model'} />} />
          {!metadata && <p className="text-xs" style={{ color: 'var(--cp-warning)' }}>{t('aiCenter.rules.mapFirst', 'Select a recognized built-in model before configuring tasks, API types and thinking efforts.')}</p>}
        </section>
        <section className={section} style={{ borderColor: 'var(--cp-border)' }}>
          <h3 className="text-sm font-semibold">{t('aiCenter.rules.routes', 'Task lists')}</h3>
          <Autocomplete multiple disableCloseOnSelect size="small" disabled={controlsDisabled}
            options={canUseTasks ? options.tasks : []} value={tasks} onChange={(_, values) => onChange('routes', values.join(', '))}
            renderInput={(params) => <TextField {...params} label={t('aiCenter.rules.selectTasks', 'Select LLM tasks')} />} />
          <p className="text-xs" style={{ color: 'var(--cp-muted)' }}>{canUseTasks
            ? t('aiCenter.rules.tasksHelp', 'These tasks use the upstream default thinking parameters.')
            : t('aiCenter.rules.noTasks', 'This model does not support server-default LLM task admission. Existing selections can be removed.')}</p>
        </section>
        <section className={section} style={{ borderColor: 'var(--cp-border)' }}>
          <h3 className="text-sm font-semibold">{t('aiCenter.rules.operations', 'Protocol mappings')}</h3>
          <p className="text-xs" style={{ color: 'var(--cp-muted)' }}>{t('aiCenter.rules.defaultHelp', 'Without an override, each supported API type uses the adapter default operation. Disabling a capability is separate from removing its override.')}</p>
          {apiTypes.map((apiType) => {
            const allowed = knownTypes.includes(apiType)
            const choices = adapter?.operations.filter((operation) => operation.apiTypes.includes(apiType)) ?? []
            const selected = typeof operations[apiType] === 'string' ? operations[apiType] as string : ''
            const defaultLabel = adapter?.defaults[apiType] ?? t('aiCenter.rules.serverSelects', 'Resolved by the server')
            let scoped: string
            try { scoped = rulesJson(scopedRequestRules(row.request_rules, apiType)) } catch { scoped = '[]' }
            return <div key={apiType} className="flex flex-col gap-3 rounded-lg border p-3" style={{ borderColor: 'var(--cp-border)' }}>
              <div className="flex items-center justify-between gap-3">
                <label className="flex items-center gap-2 text-sm font-medium">
                  <input type="checkbox" checked={allowed && !disabledTypes.has(apiType)} disabled={controlsDisabled || !allowed}
                    onChange={(event) => {
                      const next = new Set(disabledTypes)
                      if (event.target.checked) next.delete(apiType); else next.add(apiType)
                      onChange('disabled_api_types', rulesJson([...next]))
                    }} />{apiType}
                </label>
                {!allowed && <span className="text-xs" style={{ color: 'var(--cp-warning)' }}>{t('aiCenter.rules.unsupported', 'Unsupported')}</span>}
              </div>
              <label className="flex flex-col gap-1 text-xs">{t('aiCenter.rules.operation', 'Adapter operation')}
                <select aria-label={`${t('aiCenter.rules.operation', 'Adapter operation')} ${apiType}`} value={selected} disabled={controlsDisabled}
                  className="w-full rounded-lg border p-2" onChange={(event) => {
                    const next = { ...operations }
                    if (event.target.value) next[apiType] = event.target.value; else delete next[apiType]
                    onChange('operations', rulesJson(next))
                  }}>
                  <option value="">{t('aiCenter.rules.useDefault', 'Use adapter default')} · {defaultLabel}</option>
                  {selected && (!allowed || !choices.some((choice) => choice.id === selected)) && <option value={selected}>{selected} · {t('aiCenter.rules.unsupported', 'Unsupported')}</option>}
                  {allowed && choices.map((operation) => <option key={operation.id} value={operation.id}>{operation.id}</option>)}
                </select>
              </label>
              <details><summary className="cursor-pointer text-xs font-medium">{t('aiCenter.rules.requestRules', 'Advanced request rules')}</summary>
                <div className="mt-3"><JsonRuleInput label={`request_rules · ${apiType}`} kind="array"
                  value={row.scoped_request_rules[apiType] ?? scoped} disabled={controlsDisabled}
                  onChange={(value) => onChange('scoped_request_rules', { ...row.scoped_request_rules, [apiType]: value })} /></div>
                <p className="mt-2 text-xs" style={{ color: 'var(--cp-muted)' }}>{t('aiCenter.rules.requestHelp', 'Each rule supports defaults, set, remove and when. The selected API type is attached automatically; unrelated rules are preserved.')}</p>
              </details>
            </div>
          })}
          {apiTypes.length === 0 && <p className="text-xs" style={{ color: 'var(--cp-muted)' }}>{t('aiCenter.wizard.emptyRules', 'No entries configured.')}</p>}
          <details><summary className="cursor-pointer text-xs">{t('aiCenter.rules.allRequestRules', 'All model request rules (JSON)')}</summary>
            <div className="mt-3"><JsonRuleInput label="Model request_rules" value={row.request_rules} kind="array" disabled={controlsDisabled}
              onChange={(value) => { onChange('request_rules', value); onChange('scoped_request_rules', {}) }} /></div>
          </details>
        </section>
        <section className={section} style={{ borderColor: 'var(--cp-border)' }}>
          <h3 className="text-sm font-semibold">{t('aiCenter.rules.variants', 'Thinking level mappings')}</h3>
          <p className="text-xs" style={{ color: 'var(--cp-muted)' }}>{t('aiCenter.rules.effortsHelp', 'Choices come from the mapped model’s supported_efforts. Unconfigured efforts are unsupported; an explicitly enabled empty object is a valid mapping.')}</p>
          {metadata?.efforts.includes('native') && <p className="text-xs">{t('aiCenter.rules.nativeHelp', 'native is the base model’s default behavior and does not create a thinking variant.')}</p>}
          {efforts.map((effort) => {
            const variant = variants.find((variant) => variant.variant === `reasoning-${effort}` && variant.model_driver === metadata?.driver)
            return <div key={effort} className="flex flex-col gap-3 rounded-lg border p-3" style={{ borderColor: 'var(--cp-border)' }}>
              <label className="flex items-center gap-2 text-sm font-medium">
                <input type="checkbox" aria-label={`Enable effort ${effort}`} checked={Boolean(variant)} disabled={controlsDisabled}
                  onChange={(event) => updateVariant(effort, event.target.checked)} />{effort}
                {!variant && <span className="text-xs font-normal" style={{ color: 'var(--cp-muted)' }}>{t('aiCenter.rules.notConfigured', 'Not configured')}</span>}
              </label>
              {variant && <JsonRuleInput label={`provider_options · ${effort}`} kind="object" disabled={controlsDisabled}
                value={row.effort_options[effort] ?? rulesJson(variant.provider_options ?? {})}
                onChange={(value) => onChange('effort_options', { ...row.effort_options, [effort]: value })} />}
            </div>
          })}
          {unsupportedVariants.length > 0 && <div role="alert" className="text-xs" style={{ color: 'var(--cp-danger)' }}>
            {t('aiCenter.rules.invalidEfforts', 'Some configured efforts do not match this model:')} {unsupportedVariants.map((variant) => String(variant.variant)).join(', ')}
            {editing && <button type="button" disabled={saving} className="ml-2 underline" onClick={() => onChange('variants', rulesJson(variants.filter((variant) => !unsupportedVariants.includes(variant))))}>{t('common.remove', 'Remove')}</button>}
          </div>}
        </section>
        <section className={section} style={{ borderColor: 'var(--cp-border)' }}>
          <h3 className="text-sm font-semibold">{t('aiCenter.rules.pricing', 'Pricing')}</h3>
          <p className="text-xs" style={{ color: 'var(--cp-muted)' }}>{t('aiCenter.rules.priceHelp', 'Token rates are per individual token in the specified currency. Use null to clear an explicit price override.')}</p>
          <JsonRuleInput label={t('aiCenter.rules.pricing', 'Pricing')} value={row.pricing} kind="price" disabled={controlsDisabled} onChange={(value) => onChange('pricing', value)} />
        </section>
      </div>
      <div className="border-t p-4 text-right" style={{ borderColor: 'var(--cp-border)' }}>
        <button type="button" onClick={onClose} className="rounded-lg border px-4 py-2 text-sm">{t('aiCenter.rules.backToList', 'Back to model list')}</button>
      </div>
    </div>
  </Drawer>
}
