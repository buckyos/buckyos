/* The import report (draw.io 导入 TODO §5): what was imported, and every kind of loss with how often it happened and a
 * few examples (draw.io ids, shape names, URLs). Shown after an import that lost something or stopped. */

import { useOverlayMounted } from '../shell/popover'
import type { ImportOutcome } from './drawioImport'

export function ImportReportDialog({ outcome, onClose }: { outcome: ImportOutcome; onClose: () => void }) {
  useOverlayMounted()
  const { report } = outcome
  const notes = report.list()
  return (
    <div className="aiws-modal-backdrop" onPointerDown={(event) => { if (event.target === event.currentTarget) onClose() }}>
      <div className="aiws-dialog aiws-import-report" role="dialog" aria-modal="true" aria-label="导入报告" data-testid="aiws-import-report" onKeyDown={(event) => { if (event.key === 'Escape') onClose() }}>
        <b>导入 draw.io「{outcome.name}」{outcome.error ? '没有完成' : '完成'}</b>
        {outcome.error && <div className="aiws-error" role="alert" data-testid="aiws-import-error">{outcome.error}</div>}
        <div className="aiws-muted" data-testid="aiws-import-summary">
          {report.pages} 页 · {report.objects} 个对象 · {report.lines} 条连接线{report.images ? ` · ${report.images} 张图片` : ''}
        </div>
        {notes.length > 0 ? (
          <>
            <div>以下内容没有原样保留（原始样式保存在每个对象的 <code>config.drawio</code> 中）：</div>
            <ul className="aiws-import-notes" data-testid="aiws-import-notes">
              {notes.map((note) => (
                <li key={note.kind} data-kind={note.kind}>
                  {note.message}<span className="aiws-muted"> × {note.count}</span>
                  {note.samples.length > 0 && <div className="aiws-muted aiws-import-samples">{note.samples.join('、')}</div>}
                </li>
              ))}
            </ul>
          </>
        ) : !outcome.error && <div>全部内容都已按原样导入。</div>}
        <div className="aiws-dialog-actions"><button type="button" className="is-primary" autoFocus onClick={onClose} data-testid="aiws-import-report-close">知道了</button></div>
      </div>
    </div>
  )
}
