/* The Office import dialog (Office 导入 TODO §5.2, §6): a dropped or pasted document first asks whether it goes in as
 * a file or is imported; then the file's facts, the mode — editable (taken apart by the shared converter) or as it
 * looks (LibreOffice makes a PDF, the PDF importer takes it apart; only when the content server has LibreOffice) —
 * the choices (hidden slides, the path, columns; deleted revisions; keeping the original) and, once started,
 * progress with a cancel button. Old binary and ODF documents only have the second mode. It lives in the Workspace
 * shell and is driven by `officeImports` (officeImportRequest.ts). */

import { useEffect, useRef, useState, useSyncExternalStore } from 'react'
import { EDITABLE_KINDS, isDocumentKind } from '@office/package'
import { useStore } from '../../state/hooks'
import { useAiwsText } from '../../useAiwsText'
import { useOverlayMounted } from '../shell/popover'
import { pdfImports } from '../canvas/pdfImportRequest'
import { ASSET_LIMIT, defaultChoices, faithfulPdf, importOffice, libreOfficeAvailable, OfficeReader, readError, type OfficeImportChoices, type OfficeImportProgress, type OfficeInspection } from './officeImport'
import { officeImports, type OfficeImportRequest } from './officeImportRequest'

export function OfficeImportHost() {
  const request = useSyncExternalStore(officeImports.subscribe, officeImports.snapshot)
  return request ? <ImportOfficeDialog key={request.source.name + request.source.bytes.length} request={request} onClose={() => officeImports.open(null)} /> : null
}

const mm = (pt: number) => Math.round((pt * 25.4) / 72)
const PROGRESS: Record<OfficeImportProgress['stage'], string> = {
  read: 'aiworkspace.office.progress.read', convert: 'aiworkspace.office.progress.convert', draw: 'aiworkspace.office.progress.draw',
  upload: 'aiworkspace.office.progress.upload', commit: 'aiworkspace.import.progressSend', write: 'aiworkspace.import.progressWrite',
}

function ImportOfficeDialog({ request, onClose }: { request: OfficeImportRequest; onClose: () => void }) {
  useOverlayMounted()
  const store = useStore()
  const { t } = useAiwsText()
  const { source } = request
  const editable = EDITABLE_KINDS.includes(source.kind)
  const isDoc = isDocumentKind(source.kind)
  const [step, setStep] = useState(request.step)
  const [reader] = useState(() => new OfficeReader())
  const [inspection, setInspection] = useState<OfficeInspection | null>(null)
  const [libreOffice, setLibreOffice] = useState<boolean | null>(null)
  const [error, setError] = useState<string | null>(null)
  const [choices, setChoices] = useState<OfficeImportChoices>(() => ({ ...defaultChoices(null, source.asset ? 0 : source.bytes.length), mode: editable ? 'editable' : 'faithful' }))
  const [progress, setProgress] = useState<OfficeImportProgress | null>(null)
  const abort = useRef<AbortController | null>(null)
  const tooLarge = source.bytes.length > ASSET_LIMIT && !source.asset

  useEffect(() => () => reader.close(), [reader])
  useEffect(() => {
    let live = true
    void libreOfficeAvailable(store).then((ok) => { if (live) setLibreOffice(ok) })
    return () => { live = false }
  }, [store])
  useEffect(() => {
    if (step !== 'options' || inspection || !editable) return
    let live = true
    reader.open(source.bytes).then((facts) => {
      if (!live) return
      setInspection(facts)
      setChoices((c) => ({ ...defaultChoices(facts, source.asset ? 0 : source.bytes.length), mode: c.mode }))
    }, (e: unknown) => { if (live) setError(readError(e)) })
    return () => { live = false }
  }, [step, inspection, editable, reader, source])

  const running = progress !== null
  const set = (patch: Partial<OfficeImportChoices>) => setChoices((c) => ({ ...c, ...patch }))
  const finish = request.finish ?? officeImports.finisher
  const start = async (mode: OfficeImportChoices['mode']) => {
    const control = new AbortController()
    abort.current = control
    setProgress({ stage: mode === 'faithful' ? 'upload' : 'convert', done: 0, total: 1 })
    if (mode === 'faithful') {
      try {
        const pdf = await faithfulPdf(store, source, setProgress, control.signal)
        onClose()
        pdfImports.open({ source: { name: pdf.name, bytes: pdf.bytes, original: pdf.original }, step: 'options', ...(request.finish ? { finish: request.finish } : {}) })
      } catch (e) {
        abort.current = null
        setProgress(null)
        setError(control.signal.aborted ? t('aiworkspace.office.cancelled') : t('aiworkspace.office.error.libreOffice', { reason: e instanceof Error ? e.message : String(e) }))
      }
      return
    }
    if (!inspection) return
    const outcome = await importOffice(store, reader, source, inspection, choices, libreOffice === true, setProgress, control.signal)
    abort.current = null
    onClose()
    finish?.(outcome)
  }

  if (step === 'ask') {
    return (
      <div className="aiws-modal-backdrop" onPointerDown={(event) => { if (event.target === event.currentTarget) onClose() }}>
        <div className="aiws-dialog aiws-pdf-dialog" role="dialog" aria-modal="true" aria-label={t('aiworkspace.office.askTitle')} data-testid="aiws-office-ask" onKeyDown={(event) => { if (event.key === 'Escape') onClose() }}>
          <div className="aiws-dialog-head"><b>{t('aiworkspace.office.askTitle')}</b></div>
          <div className="aiws-muted">{t(editable ? (isDoc ? 'aiworkspace.office.askTextDoc' : 'aiworkspace.office.askTextSlides') : 'aiworkspace.office.askTextLegacy', { name: source.name })}</div>
          {!editable && libreOffice === false && <div className="aiws-muted" data-testid="aiws-office-no-libreoffice">{t('aiworkspace.office.noLibreOffice')}</div>}
          <div className="aiws-dialog-actions">
            {editable
              ? <button type="button" className="is-primary" autoFocus data-testid="aiws-office-ask-import" onClick={() => setStep('options')}>{t(isDoc ? 'aiworkspace.office.askDocument' : 'aiworkspace.office.askPages')}</button>
              : <button type="button" className="is-primary" autoFocus data-testid="aiws-office-ask-import" disabled={libreOffice !== true} onClick={() => setStep('options')}>{t('aiworkspace.office.askFaithful')}</button>}
            <button type="button" data-testid="aiws-office-ask-file" onClick={() => { onClose(); request.insertAsFile?.() }}>{t('aiworkspace.office.askFile')}</button>
            <button type="button" onClick={onClose}>{t('aiworkspace.common.cancel')}</button>
          </div>
        </div>
      </div>
    )
  }

  const hidden = inspection?.slides.filter((s) => s.hidden).length ?? 0
  const ready = !editable || inspection !== null
  return (
    <div className="aiws-modal-backdrop" onPointerDown={(event) => { if (event.target === event.currentTarget && !running) onClose() }}>
      <div className="aiws-dialog aiws-pdf-dialog" role="dialog" aria-modal="true" aria-label={t('aiworkspace.office.title', { name: source.name })} data-testid="aiws-office-dialog" data-libreoffice={libreOffice === null ? 'unknown' : String(libreOffice)} onKeyDown={(event) => { if (event.key === 'Escape' && !running) onClose() }}>
        <div className="aiws-dialog-head"><b>{t('aiworkspace.office.title', { name: source.name })}</b></div>
        {error ? (
          <>
            <div className="aiws-error" role="alert" data-testid="aiws-office-error">{error}</div>
            <div className="aiws-dialog-actions"><button type="button" autoFocus onClick={onClose}>{t('aiworkspace.common.close')}</button></div>
          </>
        ) : !ready ? (
          <div className="aiws-muted" data-testid="aiws-office-reading">{t('aiworkspace.office.reading')}</div>
        ) : (
          <>
            <div className="aiws-muted" data-testid="aiws-office-facts">
              {inspection?.kind === 'pptx'
                ? t(hidden ? 'aiworkspace.office.factsSlidesHidden' : 'aiworkspace.office.factsSlides', { count: inspection.slides.length, hidden, w: mm(inspection.size?.w ?? 0), h: mm(inspection.size?.h ?? 0) })
                : t(isDoc ? 'aiworkspace.office.factsDoc' : 'aiworkspace.office.factsSlidesLegacy')}
            </div>
            <fieldset className="aiws-field" disabled={running}>
              <label className="aiws-inline-form" title={!editable ? t('aiworkspace.office.editableUnavailable') : undefined}>
                <input type="radio" name="office-mode" checked={choices.mode === 'editable'} disabled={!editable} data-testid="aiws-office-mode-editable" onChange={() => set({ mode: 'editable' })} />
                {t(isDoc ? 'aiworkspace.office.modeEditableDoc' : 'aiworkspace.office.modeEditableSlides')}
              </label>
              <label className="aiws-inline-form" title={libreOffice === false ? t('aiworkspace.office.noLibreOffice') : undefined}>
                <input type="radio" name="office-mode" checked={choices.mode === 'faithful'} disabled={libreOffice !== true} data-testid="aiws-office-mode-faithful" onChange={() => set({ mode: 'faithful' })} />
                {t('aiworkspace.office.modeFaithful')}
                {libreOffice === false && <span className="aiws-muted"> {t('aiworkspace.office.noLibreOfficeShort')}</span>}
              </label>
              {choices.mode === 'editable' && (
                <div className="aiws-muted" data-testid="aiws-office-objects-note">{t(libreOffice ? 'aiworkspace.office.objectsAsPictures' : 'aiworkspace.office.objectsAsBoxes')}</div>
              )}
              {choices.mode === 'editable' && inspection?.kind === 'pptx' && (
                <>
                  {hidden > 0 && <label className="aiws-inline-form"><input type="checkbox" checked={choices.includeHidden} data-testid="aiws-office-hidden" onChange={(e) => set({ includeHidden: e.target.checked })} />{t('aiworkspace.office.includeHidden', { count: hidden })}</label>}
                  <label className="aiws-inline-form"><input type="checkbox" checked={choices.createPath} data-testid="aiws-office-path" onChange={(e) => set({ createPath: e.target.checked })} />{t('aiworkspace.office.createPath')}</label>
                  <label className="aiws-inline-form">{t('aiworkspace.pdf.columns')}
                    <input type="number" min={1} max={12} value={choices.columns} data-testid="aiws-office-columns" onChange={(e) => set({ columns: Math.max(1, Math.min(12, Number(e.target.value) || 1)) })} />
                  </label>
                </>
              )}
              {choices.mode === 'editable' && inspection?.kind === 'docx' && (
                <label className="aiws-inline-form"><input type="checkbox" checked={choices.keepDeleted} data-testid="aiws-office-deleted" onChange={(e) => set({ keepDeleted: e.target.checked })} />{t('aiworkspace.office.keepDeleted')}</label>
              )}
              {choices.mode === 'editable' && (
                <label className="aiws-inline-form" title={tooLarge ? t('aiworkspace.pdf.keepTooLarge') : undefined}>
                  <input type="checkbox" checked={choices.keepOriginal && !tooLarge} disabled={tooLarge} data-testid="aiws-office-keep" onChange={(e) => set({ keepOriginal: e.target.checked })} />{t('aiworkspace.office.keepOriginal')}
                  {tooLarge && <span className="aiws-muted"> {t('aiworkspace.pdf.keepTooLarge')}</span>}
                </label>
              )}
            </fieldset>
            {running && progress && (
              <div className="aiws-pdf-progress" data-testid="aiws-office-progress">
                <progress max={Math.max(1, progress.total)} value={progress.done} />
                <span className="aiws-muted">{t(PROGRESS[progress.stage], { done: Math.min(progress.done + 1, Math.max(progress.total, progress.done + 1)), total: progress.total })}</span>
              </div>
            )}
            <div className="aiws-dialog-actions">
              {running ? (
                <button type="button" data-testid="aiws-office-cancel" disabled={progress?.stage === 'commit' || progress?.stage === 'write'} onClick={() => abort.current?.abort()}>{t('aiworkspace.common.cancel')}</button>
              ) : (
                <>
                  <button type="button" className="is-primary" autoFocus data-testid="aiws-office-start" disabled={choices.mode === 'faithful' ? libreOffice !== true : !inspection} onClick={() => { void start(choices.mode) }}>{t('aiworkspace.office.start')}</button>
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
