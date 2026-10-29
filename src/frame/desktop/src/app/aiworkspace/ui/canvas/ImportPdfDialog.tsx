/* The PDF import dialog (PDF 导入方案 §5 步 2–3): a dropped or pasted PDF first asks whether it goes in as a file or
 * as editable pages (Q7); then the file's facts (page count, size, the first page and what it would become), the
 * choices (range, editable or high fidelity, paragraphs or lines, crowded pages, keeping the original, the path,
 * columns) and, once started, progress with a cancel button. It lives outside the canvas view (the import ends on
 * a new canvas) and is driven by `pdfImports` (pdfImportRequest.ts). */

import { useEffect, useRef, useState, useSyncExternalStore } from 'react'
import { useStore } from '../../state/hooks'
import { useAiwsText } from '../../useAiwsText'
import { useOverlayMounted } from '../shell/popover'
import { ASSET_LIMIT, defaultChoices, importPdf, inspectPdf, PATH_LIMIT, type PdfImportChoices, type PdfImportProgress, type PdfInspection } from './pdfImport'
import { pdfImports, type PdfImportRequest } from './pdfImportRequest'

export function PdfImportHost() {
  const request = useSyncExternalStore(pdfImports.subscribe, pdfImports.snapshot)
  return request ? <ImportPdfDialog key={request.source.name + request.source.bytes.length} request={request} onClose={() => pdfImports.open(null)} /> : null
}

const mm = (pt: number) => Math.round((pt * 25.4) / 72)
const PROGRESS: Record<PdfImportProgress['stage'], string> = { convert: 'aiworkspace.pdf.progress.convert', upload: 'aiworkspace.pdf.progress.upload', commit: 'aiworkspace.pdf.progress.commit' }

function ImportPdfDialog({ request, onClose }: { request: PdfImportRequest; onClose: () => void }) {
  useOverlayMounted()
  const store = useStore()
  const { t } = useAiwsText()
  const [step, setStep] = useState(request.step)
  const [inspection, setInspection] = useState<PdfInspection | null>(null)
  const [error, setError] = useState<string | null>(null)
  const [choices, setChoices] = useState<PdfImportChoices | null>(null)
  const [progress, setProgress] = useState<PdfImportProgress | null>(null)
  const [previewUrl, setPreviewUrl] = useState<string | null>(null)
  const abort = useRef<AbortController | null>(null)
  const { source } = request
  const tooLarge = source.bytes.length > ASSET_LIMIT && !source.asset

  useEffect(() => {
    if (step !== 'options' || inspection) return
    const control = new AbortController()
    inspectPdf(source.bytes, control.signal).then((facts) => {
      if (control.signal.aborted) return
      setInspection(facts)
      setChoices(defaultChoices(facts, source.asset ? 0 : source.bytes.length))
      if (facts.preview) setPreviewUrl(URL.createObjectURL(facts.preview))
    }, (e: unknown) => { if (!control.signal.aborted) setError(t(e instanceof Error && e.message === 'password' ? 'aiworkspace.pdf.error.password' : 'aiworkspace.pdf.error.unreadable', { reason: e instanceof Error ? e.message : String(e) })) })
    return () => control.abort()
  }, [step, inspection, source, t])
  useEffect(() => () => { if (previewUrl) URL.revokeObjectURL(previewUrl) }, [previewUrl])

  const running = progress !== null
  const start = async () => {
    if (!choices || !inspection) return
    const control = new AbortController()
    abort.current = control
    setProgress({ stage: 'convert', done: 0, total: choices.to - choices.from + 1 })
    const outcome = await importPdf(store, source, choices, inspection.preview, setProgress, control.signal)
    abort.current = null
    onClose()
    ;(request.finish ?? pdfImports.finisher)?.(outcome)
  }
  const set = (patch: Partial<PdfImportChoices>) => setChoices((c) => (c ? { ...c, ...patch } : c))
  const count = inspection?.pageCount ?? 0

  if (step === 'ask') {
    return (
      <div className="aiws-modal-backdrop" onPointerDown={(event) => { if (event.target === event.currentTarget) onClose() }}>
        <div className="aiws-dialog aiws-pdf-dialog" role="dialog" aria-modal="true" aria-label={t('aiworkspace.pdf.askTitle')} data-testid="aiws-pdf-ask" onKeyDown={(event) => { if (event.key === 'Escape') onClose() }}>
          <div className="aiws-dialog-head"><b>{t('aiworkspace.pdf.askTitle')}</b></div>
          <div className="aiws-muted">{t('aiworkspace.pdf.askText', { name: source.name })}</div>
          <div className="aiws-dialog-actions">
            <button type="button" className="is-primary" autoFocus data-testid="aiws-pdf-ask-pages" onClick={() => setStep('options')}>{t('aiworkspace.pdf.askPages')}</button>
            <button type="button" data-testid="aiws-pdf-ask-file" onClick={() => { onClose(); request.insertAsFile?.() }}>{t('aiworkspace.pdf.askFile')}</button>
            <button type="button" onClick={onClose}>{t('aiworkspace.common.cancel')}</button>
          </div>
        </div>
      </div>
    )
  }

  const estimate = inspection?.estimate
  return (
    <div className="aiws-modal-backdrop" onPointerDown={(event) => { if (event.target === event.currentTarget && !running) onClose() }}>
      <div className="aiws-dialog aiws-pdf-dialog" role="dialog" aria-modal="true" aria-label={t('aiworkspace.pdf.title', { name: source.name })} data-testid="aiws-pdf-dialog" onKeyDown={(event) => { if (event.key === 'Escape' && !running) onClose() }}>
        <div className="aiws-dialog-head"><b>{t('aiworkspace.pdf.title', { name: source.name })}</b></div>
        {error ? (
          <>
            <div className="aiws-error" role="alert" data-testid="aiws-pdf-error">{error}</div>
            <div className="aiws-dialog-actions"><button type="button" autoFocus onClick={onClose}>{t('aiworkspace.common.close')}</button></div>
          </>
        ) : !inspection || !choices ? (
          <div className="aiws-muted" data-testid="aiws-pdf-reading">{t('aiworkspace.pdf.reading')}</div>
        ) : (
          <>
            <div className="aiws-pdf-facts">
              {previewUrl && <img className="aiws-pdf-preview" src={previewUrl} alt={t('aiworkspace.pdf.previewAlt')} data-testid="aiws-pdf-preview" />}
              <div className="aiws-pdf-facts-text">
                <div data-testid="aiws-pdf-pages">{t('aiworkspace.pdf.facts', { count, w: mm(inspection.size.w), h: mm(inspection.size.h) })}</div>
                {estimate && (
                  <div className="aiws-muted" data-testid="aiws-pdf-estimate">
                    {t('aiworkspace.pdf.estimate', { text: t('aiworkspace.pdf.count.text', { count: estimate.text }), images: t('aiworkspace.drawio.count.images', { count: estimate.images }), shapes: t('aiworkspace.pdf.count.shapes', { count: estimate.shapes }) })}
                    {estimate.raster ? t('aiworkspace.pdf.estimateRaster') : ''}
                  </div>
                )}
              </div>
            </div>
            <fieldset className="aiws-field" disabled={running}>
              <label className="aiws-inline-form">{t('aiworkspace.pdf.range')}
                <input type="number" min={1} max={count} value={choices.from} data-testid="aiws-pdf-from" onChange={(e) => set({ from: Math.max(1, Math.min(count, Number(e.target.value) || 1)) })} />
                <span>–</span>
                <input type="number" min={1} max={count} value={choices.to} data-testid="aiws-pdf-to" onChange={(e) => set({ to: Math.max(1, Math.min(count, Number(e.target.value) || count)) })} />
              </label>
              <label className="aiws-inline-form"><input type="radio" name="pdf-mode" checked={choices.mode === 'editable'} data-testid="aiws-pdf-mode-editable" onChange={() => set({ mode: 'editable' })} />{t('aiworkspace.pdf.modeEditable')}</label>
              <label className="aiws-inline-form"><input type="radio" name="pdf-mode" checked={choices.mode === 'raster'} data-testid="aiws-pdf-mode-raster" onChange={() => set({ mode: 'raster' })} />{t('aiworkspace.pdf.modeRaster')}</label>
              {choices.mode === 'editable' ? (
                <>
                  <label className="aiws-inline-form">{t('aiworkspace.pdf.unit')}
                    <select value={choices.unit} data-testid="aiws-pdf-unit" onChange={(e) => set({ unit: e.target.value === 'line' ? 'line' : 'paragraph' })}>
                      <option value="paragraph">{t('aiworkspace.pdf.unitParagraph')}</option>
                      <option value="line">{t('aiworkspace.pdf.unitLine')}</option>
                    </select>
                  </label>
                  <label className="aiws-inline-form">{t('aiworkspace.pdf.complex', { count: PATH_LIMIT })}
                    <select value={choices.complex} data-testid="aiws-pdf-complex" onChange={(e) => set({ complex: e.target.value === 'whole' ? 'whole' : 'vectors' })}>
                      <option value="vectors">{t('aiworkspace.pdf.complexVectors')}</option>
                      <option value="whole">{t('aiworkspace.pdf.complexWhole')}</option>
                    </select>
                  </label>
                </>
              ) : (
                <label className="aiws-inline-form">{t('aiworkspace.pdf.rasterEdge')}
                  <select value={choices.rasterEdge} data-testid="aiws-pdf-raster-edge" onChange={(e) => set({ rasterEdge: e.target.value === '3840' ? 3840 : 2560 })}>
                    <option value="2560">2560 px</option>
                    <option value="3840">3840 px</option>
                  </select>
                </label>
              )}
              <label className="aiws-inline-form" title={tooLarge ? t('aiworkspace.pdf.keepTooLarge') : undefined}>
                <input type="checkbox" checked={choices.keepOriginal && !tooLarge} disabled={tooLarge} data-testid="aiws-pdf-keep" onChange={(e) => set({ keepOriginal: e.target.checked })} />{t('aiworkspace.pdf.keepOriginal')}
                {tooLarge && <span className="aiws-muted"> {t('aiworkspace.pdf.keepTooLarge')}</span>}
              </label>
              <label className="aiws-inline-form"><input type="checkbox" checked={choices.createPath} data-testid="aiws-pdf-path" onChange={(e) => set({ createPath: e.target.checked })} />{t('aiworkspace.pdf.createPath')}</label>
              <label className="aiws-inline-form">{t('aiworkspace.pdf.columns')}
                <input type="number" min={1} max={12} value={choices.columns} data-testid="aiws-pdf-columns" onChange={(e) => set({ columns: Math.max(1, Math.min(12, Number(e.target.value) || 1)) })} />
              </label>
            </fieldset>
            {running && progress && (
              <div className="aiws-pdf-progress" data-testid="aiws-pdf-progress">
                <progress max={Math.max(1, progress.total)} value={progress.done} />
                <span className="aiws-muted">{t(PROGRESS[progress.stage], { done: Math.min(progress.done + 1, progress.total), total: progress.total })}</span>
              </div>
            )}
            <div className="aiws-dialog-actions">
              {running ? (
                <button type="button" data-testid="aiws-pdf-cancel" disabled={progress?.stage === 'commit'} onClick={() => abort.current?.abort()}>{t('aiworkspace.common.cancel')}</button>
              ) : (
                <>
                  <button type="button" className="is-primary" autoFocus data-testid="aiws-pdf-start" disabled={choices.from > choices.to} onClick={() => { void start() }}>{t('aiworkspace.pdf.start', { count: choices.to - choices.from + 1 })}</button>
                  <button type="button" onClick={onClose}>{t('aiworkspace.common.cancel')}</button>
                </>
              )}
            </div>
          </>
        )}
      </div>
    </div>
  )
}
