/* The wish runs under way in this window, in the status bar (许愿格交互升级 §3.8; state/runLog.ts). Collapsed, one run
 * is one line — the wish, the stage, what it is doing, the time it has taken — and more are counted. The status panel's
 * "运行中", above the work log, has a block for each, as the work log has its uploads: the stage, what it is doing, the
 * time, the tool calls and trial runs, the model, the latest lines, and a cancel on the right. A click on a block goes to
 * the wish: its Block on the current canvas, else the Block the run started from, else any, else its editor in the
 * data mode. */

import { useEffect, useState, useSyncExternalStore } from 'react'
import { X } from 'lucide-react'
import { describeError } from '../../api/session'
import type { WishRunView } from '../../api/types'
import { useOutlineVersion, useStore, useWorkspaceUi } from '../../state/hooks'
import { describeRunLine, elapsedText, lineOffset, runDoingText, runModel, runStageText, useRunLog, type RunLogRun } from '../../state/runLog'
import { serverClock } from '../../state/serverClock'
import type { WorkspaceStore } from '../../state/store'
import { useAiwsText } from '../../useAiwsText'
import './framework.css'
import './runlog.css'

/** The latest lines a block shows. */
const SHOWN_LINES = 5

/** The service's time, once a second (a run's start is on its clock). */
function useNow(): number {
  const store = useStore()
  const clock = serverClock(store.session)
  const [now, setNow] = useState(() => clock.now())
  useEffect(() => {
    const timer = window.setInterval(() => setNow(clock.now()), 1000)
    return () => window.clearInterval(timer)
  }, [clock])
  return now
}

function wishName(store: WorkspaceStore, wishId: string, untitled: string): string {
  const wish = store.outline.get(wishId)
  return wish?.title?.trim() || wish?.name || untitled
}

function locateWish(store: WorkspaceStore, run: WishRunView, openEntity: (entityId: string) => void) {
  const blocks = store.outline.all().filter((e) => e.type_id === 'buckyos.cell' && e.source_id === run.wish_id && !e.deleted)
  const surface = store.canvasFocus.surfaceId
  const started = (run.params.location as { cell_id?: unknown } | null | undefined)?.cell_id
  const block = (surface ? blocks.find((b) => store.outline.ancestors(b.entity_id).includes(surface)) : undefined)
    ?? blocks.find((b) => b.entity_id === started) ?? blocks[0]
  openEntity(block ? block.entity_id : run.wish_id)
}

/** The collapsed status bar's part: one run as a line, more counted. */
export function RunningStatus() {
  const runs = useRunLog()
  const { t } = useAiwsText()
  if (runs.length === 0) return null
  if (runs.length === 1) return <RunLine entry={runs[0]} />
  return <span className="aiws-status-progress" data-testid="aiws-status-running" data-count={runs.length}>{t('aiworkspace.worklog.status.running', { count: runs.length })}</span>
}

function RunLine({ entry }: { entry: RunLogRun }) {
  const store = useStore()
  const { t } = useAiwsText()
  useOutlineVersion()
  const now = useNow()
  const { run } = entry
  const text = [wishName(store, run.wish_id, t('aiworkspace.runlog.untitled')), runStageText(run.stage), runDoingText(run), elapsedText(now - Date.parse(run.created_at))]
    .filter(Boolean).join(' · ')
  return (
    <span className="aiws-status-progress aiws-status-run" data-testid="aiws-status-run" data-run-id={run.run_id} data-wish={run.wish_id} data-stage={run.stage} data-phase={entry.phase} title={text}>
      {text}
    </span>
  )
}

/** "运行中" of the status panel; absent while nothing runs. */
export function RunningSection() {
  const runs = useRunLog()
  const { t } = useAiwsText()
  if (runs.length === 0) return null
  return (
    <section className="aiws-runlog" data-testid="aiws-status-runs" aria-label={t('aiworkspace.runlog.title')}>
      <div className="aiws-panel-title">{t('aiworkspace.runlog.title')}</div>
      <RunBlocks runs={runs} />
    </section>
  )
}

function RunBlocks({ runs }: { runs: readonly RunLogRun[] }) {
  const store = useStore()
  const ui = useWorkspaceUi()
  const { t } = useAiwsText()
  useOutlineVersion()
  // the cancel in progress is the service's `busy`, which moves without the run changing
  useSyncExternalStore(store.wish.subscribe, store.wish.snapshot)
  const now = useNow()
  return (
    <ul className="aiws-runlog-list">
      {runs.map(({ run, phase, lines }) => {
        const name = wishName(store, run.wish_id, t('aiworkspace.runlog.untitled'))
        const p = run.progress ?? {}
        const doing = runDoingText(run)
        const model = runModel(run)
        const meta = [
          elapsedText(now - Date.parse(run.created_at)),
          p.tool_calls ? t('aiworkspace.runlog.toolCalls', { count: p.tool_calls }) : '',
          run.stage === 'code' && p.program_runs ? t('aiworkspace.runlog.trials', { count: p.program_runs }) : '',
          model,
        ].filter(Boolean)
        const latest = lines.slice(-SHOWN_LINES)
        const cancelling = store.wish.busy.get(run.wish_id) === 'cancelling'
        const cancel = () => {
          store.wish.cancel(run).catch((error: unknown) => {
            store.notify('error', t('aiworkspace.runlog.cancelFailed', { name, error: describeError(error) }))
          })
        }
        return (
          <li key={run.run_id} className="aiws-runlog-run" data-testid="aiws-status-run-block" data-run-id={run.run_id} data-wish={run.wish_id} data-stage={run.stage} data-phase={phase}>
            <button type="button" className="aiws-runlog-main" data-testid="aiws-status-run-locate" title={t('aiworkspace.runlog.locate')} onClick={() => locateWish(store, run, ui.openEntity)}>
              <span className="aiws-runlog-head">
                <span className="aiws-runlog-name aiws-user-text">{name}</span>
                <span className="aiws-chip">{runStageText(run.stage)}</span>
                {doing && <span className="aiws-runlog-doing" data-testid="aiws-status-run-doing">{doing}</span>}
              </span>
              <span className="aiws-runlog-meta" data-testid="aiws-status-run-meta">{meta.join(' · ')}</span>
              {latest.length > 0 && (
                <span className="aiws-runlog-lines" data-testid="aiws-status-run-lines">
                  {latest.map((line) => {
                    const text = describeRunLine(line, run)
                    return (
                      <span key={line.seq} className="aiws-runlog-line" data-testid="aiws-status-run-line" data-kind={line.kind} title={text}>
                        <span className="aiws-runlog-at">{lineOffset(line, run)}</span>
                        <span className="aiws-runlog-text">{text}</span>
                      </span>
                    )
                  })}
                </span>
              )}
            </button>
            <button type="button" className="aiws-tool" data-testid="aiws-status-run-cancel" disabled={cancelling} aria-label={t('aiworkspace.runlog.cancel', { name })} title={t('aiworkspace.common.cancel')} onClick={cancel}>
              <X size={14} />
            </button>
          </li>
        )
      })}
    </ul>
  )
}
