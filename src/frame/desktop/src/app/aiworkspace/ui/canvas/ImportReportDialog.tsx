/* The import report (draw.io 导入 TODO §5): what was imported, and every kind of loss with how often it happened and a
 * few examples (draw.io ids, shape names, URLs). Shown after an import that lost something or stopped. */

import { useAiwsText } from '../../useAiwsText'
import { useOverlayMounted } from '../shell/popover'
import type { ImportOutcome } from './drawioImport'
import { reportNoteText } from './drawioText'

export function ImportReportDialog({ outcome, onClose }: { outcome: ImportOutcome; onClose: () => void }) {
  useOverlayMounted()
  const { t, tNode, list } = useAiwsText()
  const { report } = outcome
  const counts = { pages: t('aiworkspace.drawio.count.pages', { count: report.pages }), objects: t('aiworkspace.drawio.count.objects', { count: report.objects }), lines: t('aiworkspace.drawio.count.lines', { count: report.lines }) }
  const notes = report.list()
  return (
    <div className="aiws-modal-backdrop" onPointerDown={(event) => { if (event.target === event.currentTarget) onClose() }}>
      <div className="aiws-dialog aiws-import-report" role="dialog" aria-modal="true" aria-label={t('aiworkspace.drawio.report.title')} data-testid="aiws-import-report" onKeyDown={(event) => { if (event.key === 'Escape') onClose() }}>
        <b>{t(outcome.error ? 'aiworkspace.drawio.report.headingStopped' : 'aiworkspace.drawio.report.headingDone', { name: outcome.name })}</b>
        {outcome.error && <div className="aiws-error" role="alert" data-testid="aiws-import-error">{outcome.error}</div>}
        <div className="aiws-muted" data-testid="aiws-import-summary">
          {report.images ? t('aiworkspace.drawio.report.summaryWithImages', { ...counts, images: t('aiworkspace.drawio.count.images', { count: report.images }) }) : t('aiworkspace.drawio.report.summary', counts)}
        </div>
        {notes.length > 0 ? (
          <>
            <div>{tNode('aiworkspace.drawio.report.notKept', { code: <code>config.drawio</code> })}</div>
            <ul className="aiws-import-notes" data-testid="aiws-import-notes">
              {notes.map((note) => (
                <li key={note.kind} data-kind={note.kind}>
                  {reportNoteText(note, t)}<span className="aiws-muted"> × {note.count}</span>
                  {note.samples.length > 0 && <div className="aiws-muted aiws-import-samples">{list(note.samples)}</div>}
                </li>
              ))}
            </ul>
          </>
        ) : !outcome.error && <div>{t('aiworkspace.drawio.report.allKept')}</div>}
        <div className="aiws-dialog-actions"><button type="button" className="is-primary" autoFocus onClick={onClose} data-testid="aiws-import-report-close">{t('aiworkspace.drawio.report.gotIt')}</button></div>
      </div>
    </div>
  )
}
