import { useEffect, useRef, useState } from 'react'
import useSWR from 'swr'
import { FlaskConical, Play, RefreshCw, Square } from 'lucide-react'
import { useI18n } from '../../i18n/provider'
import { isMockRuntime } from '../../runtime'
import { cancelPlaygroundTask, getPlaygroundTask, invokePlayground, listPlaygroundModels } from '../../api/aicc_playground'
import { useProviders } from './hooks/use-aicc-store'
import type { ApiType } from './mock/types'
import { defaultRequest, importRequest, PLAYGROUND_APIS, taskResponse, validateRequest, type JsonObject, type ValidationIssue } from './datamodel/playground'
import { RequestFields } from './components/playground/RequestFields'
import { controlClass, controlStyle, panelStyle } from './components/playground/styles'
import { JsonDetails, PlaygroundResult } from './components/playground/PlaygroundResult'

interface PlaygroundRun {
  id: number
  startedAt: number
  elapsed: number
  request: JsonObject
  response?: JsonObject
  task?: JsonObject
  error?: unknown
  submitting: boolean
  monitoring: boolean
  monitorError?: string
  cancelRequested?: boolean
}

function describeError(error: unknown): unknown {
  if (error instanceof Error) return { ...error, name: error.name, message: error.message }
  return error
}

export function PlaygroundPage({ active = true }: { active?: boolean }) {
  const { t } = useI18n()
  const providers = useProviders()
  const [api, setApi] = useState<ApiType>('llm')
  const [params, setParams] = useState<JsonObject>(() => defaultRequest('llm'))
  const [provider, setProvider] = useState('')
  const [advanced, setAdvanced] = useState(false)
  const [source, setSource] = useState('')
  const [importError, setImportError] = useState('')
  const [importNotice, setImportNotice] = useState('')
  const [issues, setIssues] = useState<ValidationIssue[]>([])
  const [formVersion, setFormVersion] = useState(0)
  const [run, setRun] = useState<PlaygroundRun>()
  const [cancelling, setCancelling] = useState(false)
  const requestId = useRef(0)
  const mounted = useRef(true)
  useEffect(() => { mounted.current = true; return () => { mounted.current = false } }, [])
  const { data: models, error: modelsError, isLoading, mutate } = useSWR(active ? ['aicc-playground-models', providers] : null, () => listPlaygroundModels(providers), { revalidateOnFocus: false, shouldRetryOnError: false, keepPreviousData: true })
  const matchingModels = (models ?? []).filter((model) => model.api_types.includes(api) && (!provider || model.provider === provider))
  const exactModel = String(params.exact_model ?? '')
  const selectedModel = matchingModels.find((model) => model.exact_model === exactModel)
  const running = Boolean(run && !run.error && (run.submitting || run.response?.status === 'running'))
  const taskId = run?.response?.status === 'running' && typeof run.response.task_id === 'string' ? run.response.task_id : undefined
  const runId = run?.id
  const monitoring = run?.monitoring

  useEffect(() => {
    if (!running) return
    const timer = window.setInterval(() => setRun((current) => current ? { ...current, elapsed: Date.now() - current.startedAt } : current), 500)
    return () => window.clearInterval(timer)
  }, [running])

  useEffect(() => {
    if (!taskId || !monitoring) return
    let stopped = false
    let timer: ReturnType<typeof setTimeout>
    const deadline = Date.now() + 10 * 60 * 1000
    const poll = async () => {
      try {
        const task = await getPlaygroundTask(taskId)
        if (stopped) return
        const terminal = task.phase === 'Terminal'
        setRun((current) => current && current.id === runId ? { ...current, task, response: taskResponse(task, current.response!), elapsed: Date.now() - current.startedAt, monitoring: !terminal } : current)
        if (!terminal) {
          if (Date.now() >= deadline) setRun((current) => current && current.id === runId ? { ...current, monitoring: false, monitorError: 'monitorTimeout' } : current)
          else timer = setTimeout(() => void poll(), 1500)
        }
      } catch (error) {
        if (!stopped) setRun((current) => current && current.id === runId ? { ...current, monitoring: false, monitorError: error instanceof Error ? error.message : String(error) } : current)
      }
    }
    void poll()
    return () => { stopped = true; clearTimeout(timer) }
  }, [taskId, monitoring, runId])

  const updateParams = (next: JsonObject) => { setParams(next); setIssues([]); setImportNotice('') }
  const start = async () => {
    const errors = validateRequest(api, params)
    setIssues(errors)
    if (errors.length || !selectedModel || running) return
    const id = ++requestId.current
    const request = structuredClone(params)
    const startedAt = Date.now()
    setRun({ id, request, startedAt, elapsed: 0, submitting: true, monitoring: false })
    try {
      const response = await invokePlayground(api, request)
      if (!mounted.current) return
      if (!['succeeded', 'failed', 'running'].includes(String(response.status)) || typeof response.task_id !== 'string' || !response.task_id) {
        setRun({ id, request, response, startedAt, elapsed: Date.now() - startedAt, submitting: false, monitoring: false, error: t('aiCenter.playground.invalidResponse') })
        return
      }
      setRun({ id, request, response, startedAt, elapsed: Date.now() - startedAt, submitting: false, monitoring: response.status === 'running' })
    } catch (error) {
      if (mounted.current) setRun({ id, request, startedAt, elapsed: Date.now() - startedAt, submitting: false, monitoring: false, error: describeError(error) })
    }
  }

  return <div className="space-y-5 min-w-0" style={{ color: 'var(--cp-text)' }}>
    <div className="flex items-start justify-between gap-3">
      <div><h1 className="text-xl font-semibold flex items-center gap-2"><FlaskConical size={22} />{t('aiCenter.playground.title')}</h1><p className="text-sm mt-2" style={{ color: 'var(--cp-muted)' }}>{t('aiCenter.playground.subtitle')}</p></div>
      <button type="button" aria-label={t('aiCenter.playground.refreshModels')} disabled={isLoading} onClick={() => void mutate()} className="rounded-lg p-2" style={panelStyle}><RefreshCw size={18} className={isLoading ? 'animate-spin' : ''} /></button>
    </div>
    {isMockRuntime() && <p className="rounded-xl p-3 text-sm" style={{ ...panelStyle, color: 'var(--cp-warning)' }}>{t('aiCenter.playground.mock')}</p>}
    <div className="grid gap-6 items-start min-w-0" style={{ gridTemplateColumns: 'repeat(auto-fit, minmax(min(100%, 320px), 1fr))' }}>
      <form className="space-y-4 min-w-0" onSubmit={(event) => { event.preventDefault(); void start() }}>
        <fieldset disabled={running} className="space-y-4 rounded-xl p-4 min-w-0" style={panelStyle}>
          <legend className="font-semibold px-1">{t('aiCenter.playground.request')}</legend>
          <label className="block text-sm space-y-2"><span>API Type</span><select aria-label="API Type" className={controlClass} style={controlStyle} value={api} onChange={(event) => {
            const next = event.target.value as ApiType
            setApi(next); setParams(defaultRequest(next)); setIssues([]); setImportNotice(''); setImportError(''); setFormVersion((value) => value + 1)
          }}>{(Object.keys(PLAYGROUND_APIS) as ApiType[]).map((key) => <option key={key} value={key}>{key} · {t(`aiCenter.playground.api.${key}`)}</option>)}</select></label>
          <div className="grid gap-3" style={{ gridTemplateColumns: 'repeat(auto-fit, minmax(min(100%, 220px), 1fr))' }}>
            <label className="block text-sm space-y-2"><span>Provider</span><select aria-label="Provider" className={controlClass} style={controlStyle} value={provider} onChange={(event) => { setProvider(event.target.value); updateParams({ ...params, exact_model: '' }) }}>
              <option value="">{t('aiCenter.playground.allProviders')}</option>{[...new Set((models ?? []).map((model) => model.provider))].sort().map((name) => <option key={name}>{name}</option>)}
            </select></label>
            <label className="block text-sm space-y-2"><span>{t('aiCenter.playground.model')}</span><select aria-label={t('aiCenter.playground.model')} className={controlClass} style={controlStyle} value={exactModel} onChange={(event) => updateParams({ ...params, exact_model: event.target.value })}>
              <option value="">{t('aiCenter.playground.chooseModel')}</option>{exactModel && !selectedModel && <option value={exactModel}>{exactModel} ({t('aiCenter.playground.unavailable')})</option>}
              {matchingModels.map((model) => <option key={model.exact_model} value={model.exact_model}>{model.exact_model}{model.health && model.health !== 'available' && model.health !== 'healthy' ? ` · ${model.health}` : ''}</option>)}
            </select></label>
          </div>
          {isLoading && <p role="status" className="text-sm">{t('aiCenter.playground.loadingModels')}</p>}
          {modelsError && <div role="alert" className="text-sm" style={{ color: 'var(--cp-danger)' }}>{t('aiCenter.playground.modelsError')} <button type="button" className="underline" onClick={() => void mutate()}>{t('common.retry')}</button></div>}
          {!isLoading && !modelsError && !matchingModels.length && <p className="text-sm" style={{ color: 'var(--cp-muted)' }}>{t('aiCenter.playground.noModels')}</p>}
          <label className="block text-sm space-y-2"><span>{t('aiCenter.playground.executionMode')}</span><select aria-label={t('aiCenter.playground.executionMode')} className={controlClass} style={controlStyle} value={String(params.execution_mode ?? 'immediate')} onChange={(event) => updateParams({ ...params, execution_mode: event.target.value })}>
            <option value="immediate">{t('aiCenter.playground.immediate')}</option>{api !== 'decision' && <option value="stream">{t('aiCenter.playground.stream')}</option>}
          </select></label>
          <p className="text-xs" style={{ color: 'var(--cp-muted)' }}>{PLAYGROUND_APIS[api].method} · {t('aiCenter.playground.optionalHint')}</p>
          <RequestFields key={`${api}-${formVersion}`} fields={PLAYGROUND_APIS[api].fields} value={params} onChange={updateParams} />
          <label className="flex gap-2 items-center text-sm"><input type="checkbox" checked={advanced} onChange={(event) => setAdvanced(event.target.checked)} />{t('aiCenter.playground.advanced')}</label>
          {advanced && <div className="space-y-3">
            <p className="text-xs" style={{ color: 'var(--cp-muted)' }}>{t('aiCenter.playground.importHint')}</p>
            <textarea aria-label={t('aiCenter.playground.requestJson')} className={`${controlClass} font-mono`} style={controlStyle} rows={8} value={source} onChange={(event) => setSource(event.target.value)} />
            <div className="flex gap-3 flex-wrap text-sm">
              <button type="button" className="rounded-lg border px-3 py-2" style={controlStyle} onClick={() => {
                try {
                  const imported = importRequest(source, api)
                  setApi(imported.api); setParams(imported.params); setProvider(''); setIssues([]); setImportError(''); setFormVersion((value) => value + 1)
                  setImportNotice(t('aiCenter.playground.imported') + (imported.removed.length ? ` ${t('aiCenter.playground.removed')}: ${imported.removed.join(', ')}` : ''))
                } catch (error) { setImportError(t(`aiCenter.playground.${error instanceof SyntaxError ? 'invalidJson' : error instanceof Error ? error.message : 'invalidImport'}`)); setImportNotice('') }
              }}>{t('aiCenter.playground.extract')}</button>
              <button type="button" onClick={() => setSource(JSON.stringify({ method: PLAYGROUND_APIS[api].method, params }, null, 2))}>{t('aiCenter.playground.editCurrent')}</button>
            </div>
            {importError && <p role="alert" className="text-sm" style={{ color: 'var(--cp-danger)' }}>{importError}</p>}
          </div>}
          {importNotice && <p role="status" className="text-sm" style={{ color: 'var(--cp-success)' }}>{importNotice}</p>}
        </fieldset>
        {issues.length > 0 && <ul role="alert" className="rounded-xl p-4 text-sm space-y-1" style={{ ...panelStyle, color: 'var(--cp-danger)' }}>{issues.map((issue, index) => <li key={index}>{issue.path}: {t(`aiCenter.playground.validation.${issue.code}`)}</li>)}</ul>}
        <div className="flex flex-wrap gap-3">
          <button type="submit" disabled={running || !selectedModel || Boolean(modelsError)} className="inline-flex items-center gap-2 rounded-lg px-4 py-2 text-sm font-semibold disabled:opacity-50" style={{ background: 'var(--cp-accent)', color: 'var(--cp-on-accent, white)' }}><Play size={16} />{t(running ? 'aiCenter.playground.running' : 'aiCenter.playground.run')}</button>
          <button type="button" disabled={running} className="text-sm disabled:opacity-50" onClick={() => { setParams(defaultRequest(api)); setIssues([]); setImportError(''); setImportNotice(''); setSource(''); setFormVersion((value) => value + 1) }}>{t('common.reset')}</button>
          {taskId && <button type="button" disabled={cancelling || run?.cancelRequested} className="inline-flex items-center gap-2 text-sm disabled:opacity-50" onClick={async () => {
            setCancelling(true)
            try {
              const receipt = await cancelPlaygroundTask(taskId)
              setRun((current) => current && current.id === runId ? { ...current, cancelRequested: receipt.accepted === true, monitorError: receipt.accepted === true ? undefined : t('aiCenter.playground.cancelRejected'), monitoring: true } : current)
            } catch (error) { setRun((current) => current && current.id === runId ? { ...current, monitorError: error instanceof Error ? error.message : String(error) } : current) }
            finally { if (mounted.current) setCancelling(false) }
          }}><Square size={15} />{t(run?.cancelRequested ? 'aiCenter.playground.cancelRequested' : 'aiCenter.playground.cancel')}</button>}
        </div>
        <JsonDetails label={t('aiCenter.playground.requestPreview')} value={{ method: PLAYGROUND_APIS[api].method, params }} />
      </form>
      <div className="min-w-0 space-y-4">
        {run?.monitorError && <div role="alert" className="rounded-xl p-3 text-sm space-y-2" style={{ ...panelStyle, color: 'var(--cp-warning)' }}>
          <p>{t('aiCenter.playground.monitorError')}</p><p className="break-all">{run.monitorError === 'monitorTimeout' ? t('aiCenter.playground.monitorTimeout') : run.monitorError}</p>
          {taskId && <button type="button" className="underline" onClick={() => setRun((current) => current ? { ...current, monitoring: true, monitorError: undefined } : current)}>{t('aiCenter.playground.resumeMonitoring')}</button>}
        </div>}
        <PlaygroundResult key={run?.id ?? 0} request={run?.request} response={run?.response} task={run?.task} elapsed={run?.elapsed ?? 0} error={run?.error} />
      </div>
    </div>
  </div>
}
