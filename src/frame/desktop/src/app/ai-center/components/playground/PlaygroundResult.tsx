import { useState } from 'react'
import { useI18n } from '../../../../i18n/provider'
import { record, resourceUrl, type JsonObject } from '../../datamodel/playground'
import { panelStyle } from './styles'

export function JsonDetails({ label, value, open = false }: { label: string; value: unknown; open?: boolean }) {
  const { t } = useI18n()
  const [copyState, setCopyState] = useState('')
  const source = JSON.stringify(value, null, 2)
  const preview = source.length > 64000 ? source.slice(0, 64000) : source
  return <details open={open || undefined} className="min-w-0 rounded-xl p-3" style={panelStyle}>
    <summary className="cursor-pointer text-sm font-medium">{label}</summary>
    <div className="flex gap-4 py-2 text-xs" style={{ color: 'var(--cp-accent)' }}>
      <button type="button" onClick={async () => {
        try { await navigator.clipboard.writeText(source); setCopyState('common.copied') } catch { setCopyState('aiCenter.playground.copyFailed') }
      }}>{t(copyState || 'common.copy')}</button>
      <button type="button" onClick={() => {
        const url = URL.createObjectURL(new Blob([source], { type: 'application/json' }))
        const link = document.createElement('a'); link.href = url; link.download = `${label.replace(/[^\p{L}\p{N} -]/gu, '')}.json`; link.click()
        setTimeout(() => URL.revokeObjectURL(url), 1000)
      }}>{t('aiCenter.playground.download')}</button>
    </div>
    <pre className="text-xs overflow-auto max-h-96 whitespace-pre-wrap break-all">{preview}</pre>
    {source.length > preview.length && <p className="text-xs mt-2" style={{ color: 'var(--cp-muted)' }}>{t('aiCenter.playground.truncated')}</p>}
  </details>
}

function MediaResult({ value, path }: { value: unknown; path: string }) {
  const { t } = useI18n()
  const [failed, setFailed] = useState(false)
  const ref = record(value)
  const url = resourceUrl(ref)
  const mime = String(ref.mime ?? ref.mime_hint ?? '')
  const kind = mime.startsWith('audio/') || /audio|stems/.test(path) ? 'audio' : mime.startsWith('video/') || /video/.test(path) ? 'video' : mime.startsWith('image/') || /image|mask/.test(path) ? 'image' : 'file'
  return <div className="space-y-2 rounded-lg border p-3 min-w-0" style={{ borderColor: 'var(--cp-border)' }}>
    <p className="text-xs break-all">{path}</p>
    {url && !failed && kind === 'image' && <img src={url} alt={path} className="max-h-80 max-w-full rounded-lg object-contain" onError={() => setFailed(true)} />}
    {url && !failed && kind === 'audio' && <audio src={url} controls preload="metadata" className="w-full" onError={() => setFailed(true)} />}
    {url && !failed && kind === 'video' && <video src={url} controls preload="metadata" className="max-h-80 w-full rounded-lg" onError={() => setFailed(true)} />}
    {failed && <p className="text-sm" style={{ color: 'var(--cp-warning)' }}>{t('aiCenter.playground.previewFailed')}</p>}
    {url ? <a href={url} target="_blank" rel="noreferrer" className="text-sm break-all" style={{ color: 'var(--cp-accent)' }}>{t('aiCenter.playground.openResource')}</a> : <p className="text-sm break-all">{String(ref.obj_id ?? '')} · {t('aiCenter.playground.noPreview')}</p>}
    <p className="text-xs break-all" style={{ color: 'var(--cp-muted)' }}>{mime || ref.kind as string}</p>
  </div>
}

function collectResources(value: unknown, path = '', result: { path: string; value: unknown }[] = [], depth = 0) {
  if (depth > 12 || result.length >= 32 || value === null || typeof value !== 'object') return result
  const obj = record(value)
  if (['url', 'base64', 'named_object'].includes(String(obj.kind))) { result.push({ path, value }); return result }
  Object.entries(value).forEach(([key, item]) => { if (key !== 'embedding') collectResources(item, path ? `${path}.${key}` : key, result, depth + 1) })
  return result
}

export function PlaygroundResult({ request, response, task, elapsed, error }: { request?: JsonObject; response?: JsonObject; task?: JsonObject; elapsed: number; error?: unknown }) {
  const { t } = useI18n()
  const textBlocks = Array.isArray(record(response?.message).content) ? (record(response?.message).content as unknown[]).map(record) : []
  const embeddings = Array.isArray(response?.data) ? response.data.map(record).filter((item) => Array.isArray(item.embedding)) : []
  const resources = collectResources(response)
  const resultError = error ?? response?.error
  return <section className="space-y-4 min-w-0" aria-label={t('aiCenter.playground.result')}>
    <h2 className="text-lg font-semibold">{t('aiCenter.playground.result')}</h2>
    {!request && <p className="text-sm rounded-xl p-5" style={panelStyle}>{t('aiCenter.playground.resultEmpty')}</p>}
    {request && <>
      <div className="rounded-xl p-4 space-y-2 text-sm break-all" style={panelStyle} aria-live="polite">
        <div className="flex flex-wrap justify-between gap-2"><strong>{t(`aiCenter.playground.status.${error ? 'error' : response?.status ?? 'submitting'}`, String(response?.status ?? ''))}</strong><span>{(elapsed / 1000).toFixed(1)} s</span></div>
        <p>{String(request.exact_model ?? '')}</p>
        {Boolean(response?.finish_reason) && <p>{t('aiCenter.playground.finishReason')}: {String(response?.finish_reason)}</p>}
        {Boolean(response?.task_id) && <a className="inline-block" style={{ color: 'var(--cp-accent)' }} href={`/taskcenter?taskid=${encodeURIComponent(String(response?.task_id))}`} target="_blank" rel="noreferrer">{t('aiCenter.playground.viewTask')}: {String(response?.task_id)}</a>}
        {Boolean(response?.provider_task_ref) && <p>{t('aiCenter.playground.providerTask')}: {String(response?.provider_task_ref)}</p>}
        {Boolean(task?.phase) && <p>{String(task?.phase)} {task?.message ? `· ${String(task.message)}` : ''}</p>}
      </div>
      {resultError != null && <div role="alert" className="rounded-xl p-3 text-sm break-all" style={{ ...panelStyle, color: 'var(--cp-danger)' }}>
        <strong>{t('aiCenter.playground.error')}</strong><pre className="whitespace-pre-wrap">{typeof resultError === 'string' ? resultError : JSON.stringify(resultError, null, 2)}</pre>
      </div>}
      {textBlocks.map((block, index) => (typeof block.text === 'string' || typeof block.summary === 'string') && <div key={index} className="rounded-xl p-4" style={panelStyle}><div className="text-xs mb-2" style={{ color: 'var(--cp-muted)' }}>{String(block.type)}</div><p className="text-sm whitespace-pre-wrap break-words">{String(block.text ?? block.summary)}</p></div>)}
      {typeof response?.text === 'string' && <p className="rounded-xl p-4 text-sm whitespace-pre-wrap break-words" style={panelStyle}>{response.text}</p>}
      {resources.map((resource) => <MediaResult key={resource.path} {...resource} />)}
      {embeddings.length > 0 && <div className="overflow-auto rounded-xl p-3" style={panelStyle}><table className="w-full text-sm"><thead><tr><th>ID</th><th>{t('aiCenter.playground.dimensions')}</th><th>{t('aiCenter.playground.vectorPreview')}</th></tr></thead><tbody>{embeddings.map((item, index) => <tr key={index}><td>{String(item.id ?? item.index ?? index)}</td><td>{(item.embedding as unknown[]).length}</td><td className="font-mono text-xs">{JSON.stringify((item.embedding as unknown[]).slice(0, 6))}…</td></tr>)}</tbody></table></div>}
      {['answers', 'results', 'captions', 'detections', 'masks', 'segments', 'actions', 'tool_calls', 'pages', 'structure', 'diagnostic'].map((key) => response?.[key] !== undefined && <JsonDetails key={key} label={t(`aiCenter.playground.field.${key}`, key)} value={response[key]} open />)}
      {response && <div className="grid gap-3" style={{ gridTemplateColumns: 'repeat(auto-fit, minmax(min(100%, 220px), 1fr))' }}>
        <JsonDetails label={t('aiCenter.playground.usage')} value={response.usage ?? t('aiCenter.playground.notReported')} open />
        <JsonDetails label={t('aiCenter.playground.cost')} value={response.cost ?? t('aiCenter.playground.notReported')} open />
      </div>}
      {Boolean(response?.route_trace) && <JsonDetails label={t('aiCenter.playground.routeTrace')} value={response?.route_trace} />}
      {Boolean(task?.progress) && <JsonDetails label={t('aiCenter.playground.progress')} value={task?.progress} open />}
      <JsonDetails label={t('aiCenter.playground.sentRequest')} value={request} />
      {response && <JsonDetails label={t('aiCenter.playground.rawResponse')} value={response} />}
      {task && <JsonDetails label={t('aiCenter.playground.rawTask')} value={task} />}
    </>}
  </section>
}
