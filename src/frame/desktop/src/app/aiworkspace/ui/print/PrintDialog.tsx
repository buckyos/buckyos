/* "导出可打印 PDF" (打印 TODO §3.1, T4, T7): the dialog of one canvas — the presentation path (a free canvas with more
 * than one), the page size and orientation, an optional PDF Frame of the page's shape, the print fonts that are not on
 * the node, the pages and what will not be printed. The place in the Zone is chosen first (the system file picker's
 * save mode), then the PDF is made and committed there; downloading to this device is the other way. Progress and
 * cancelling follow the print package and the `media.page` Run; a failure leaves no file. The PDF never goes back
 * into the Workspace (Q10). Full screen on a phone. Mounted once by the shell (PrintHost). */

import { useCallback, useEffect, useRef, useState } from 'react'
import { X } from 'lucide-react'
import { defaultPaper, frameFits, orientationOf, paperOf, pdfFileName, PAPER_NAMES, PAPERS, type Orientation, type Paper, type PaperName } from '@print/page'
import { SYSTEM_DEFAULT } from '@print/fonts'
import { downloadExtToolsOnline, fetchExtToolsStatus, type ExtToolsStatusResponse } from '../../../../api/settings'
import { useSaveToZone } from '../../../../components/saveToZone'
import { useI18n } from '../../../../i18n/provider'
import { useViewerIsAdmin } from '../../../settings/hooks/use-viewer-is-admin'
import { ONLINE_POLL_MS, onlineBusy, onlineErrorText, onlineRequest, onlineStateLabel, packageOfItem, requestPending, type OnlineRequest } from '../../../settings/pages/extToolsOnline'
import { OnlineProgress } from '../../../settings/pages/OnlineProgress'
import { describeError } from '../../api/session'
import { playable, resolveSteps, STEP_PROBLEM_TEXT, type ResolvedStep } from '../../presentation/model'
import { useOutlineVersion, useSessionStatus, useStore } from '../../state/hooks'
import { useWorkspaceSettings } from '../../state/settings'
import type { AiwsText } from '../../i18n'
import { useAiwsText } from '../../useAiwsText'
import { listPdfFrames } from '../controls/pdfframe/api'
import { Modal } from '../shell/dialogs'
import { saveBlob } from '../shell/download'
import { useShell } from '../shell/shellContext'
import { openExtTools } from './extTools'
import { useFontCheck, type FontCheck } from './fonts'
import { PrintFontsDialog } from './PrintFontsDialog'
import { issueCounts, makePrintPdf, PrintFailure, readPdf, type PrintMade, type PrintPhase, type PrintRequest } from './generate'
import { onPrintRequest, type PrintTarget } from './request'
import './print.css'

type OpenPrint = { kind: 'print'; target: PrintTarget; n: number }
let opened = 0

/** The print dialog and the print fonts dialog, as the menu entries ask for them; the fonts dialog opened from the print
 * dialog goes back to it (which checks the fonts again). */
export function PrintHost() {
  const [open, setOpen] = useState<OpenPrint | { kind: 'fonts'; back: OpenPrint | null } | null>(null)
  useEffect(() => onPrintRequest((request) => setOpen(request.kind === 'print' ? { ...request, n: ++opened } : { kind: 'fonts', back: null })), [])
  if (!open) return null
  if (open.kind === 'fonts') return <PrintFontsDialog onClose={() => setOpen(open.back ? { ...open.back, n: ++opened } : null)} />
  return <PrintDialog key={open.n} target={open.target} onClose={() => setOpen(null)} onFonts={() => setOpen({ kind: 'fonts', back: open })} />
}

type Failure = { kind: 'cancelled' | 'browser_missing' | 'other'; text: string }

function paperLabel(name: PaperName, t: AiwsText['t']): string {
  const p = PAPERS[name]
  return t('aiworkspace.print.paperSize', { name, w: Math.round(p.width_mm * 10) / 10, h: Math.round(p.height_mm * 10) / 10 })
}

function PrintDialog({ target, onClose, onFonts }: { target: PrintTarget; onClose: () => void; onFonts: () => void }) {
  const { t, list, locale } = useAiwsText()
  const store = useStore()
  const shell = useShell()
  const status = useSessionStatus()
  useOutlineVersion()
  const settings = useWorkspaceSettings()
  const fonts = useFontCheck(settings.value.fonts)
  const isAdmin = useViewerIsAdmin()
  const save = useSaveToZone(t('aiworkspace.print.nameTaken'))
  const info = store.session.info()
  const surface = store.outline.get(target.surfaceId)
  const title = surface?.title || surface?.name || info.title
  const fileName = pdfFileName(title)
  const free = target.mode === 'free'

  const [pathId, setPathId] = useState<string | null>(target.paths[0]?.id ?? null)
  const path = target.paths.find((p) => p.id === pathId) ?? target.paths[0] ?? null
  const steps = free && path ? resolveSteps(store.outline, path.payload) : []
  const exportable = (surfaceId: string | null) => Boolean(surfaceId && store.outline.get(surfaceId)?.capabilities.includes('export'))
  const pages = playable(steps).filter((s) => exportable(s.surfaceId))
  const skipped = steps.filter((s) => !pages.includes(s))
  const elsewhere = pages.filter((s) => s.surfaceId !== target.surfaceId).length

  const [chosenPaper, setChosenPaper] = useState<{ name: PaperName; orientation: Orientation } | null>(null)
  const paper: Paper = chosenPaper ? paperOf(chosenPaper.name, chosenPaper.orientation) : defaultPaper(target.mode, path?.payload.stage)
  const paperName = paper.name === 'custom' ? 'A4' : paper.name
  const orientation = orientationOf(paper.width_mm, paper.height_mm)

  const frames = listPdfFrames(store.outline.all())
  const [frameId, setFrameId] = useState<string | null>(null)
  const frame = frames.find((f) => f.id === frameId && frameFits(f.geometry, paper)) ?? null

  const [phase, setPhase] = useState<PrintPhase | null>(null)
  const phaseAt = useRef<PrintPhase | null>(null)
  const abort = useRef<AbortController | null>(null)
  const [done, setDone] = useState<{ text: string; made: PrintMade } | null>(null)
  const [failure, setFailure] = useState<Failure | null>(null)
  const busy = phase !== null
  const follow = useCallback((next: PrintPhase) => { phaseAt.current = next; setPhase(next) }, [])

  const offline = status.kind !== 'live'
  const blocked = offline ? t('aiworkspace.shell.needsOnline')
    : !exportable(target.surfaceId) ? t('aiworkspace.shell.menu.noExportPermission')
      : free && pages.length === 0 ? t('aiworkspace.print.nothingToPrint') : null

  const failureOf = (error: unknown): Failure => {
    if (error instanceof PrintFailure) {
      if (error.kind === 'cancelled') return { kind: 'cancelled', text: t('aiworkspace.print.cancelled') }
      if (error.kind === 'browser_missing') return { kind: 'browser_missing', text: t('aiworkspace.print.browserMissing') }
      if (error.kind === 'tools_missing') return { kind: 'other', text: t('aiworkspace.print.toolsMissing', { tools: error.detail }) }
      if (error.kind === 'deliver') return { kind: 'other', text: t('aiworkspace.print.deliverFailed', { error: error.detail }) }
      return { kind: 'other', text: t('aiworkspace.print.renderFailed', { error: error.detail }) }
    }
    const at = phaseAt.current
    const key = at === 'package' ? 'aiworkspace.print.packageFailed' : at === 'render' ? 'aiworkspace.print.renderFailed' : 'aiworkspace.print.saveFailed'
    return { kind: 'other', text: t(key, { error: describeError(error) }) }
  }

  const run = async (to: 'zone' | 'device') => {
    if (busy || blocked) return
    const controller = new AbortController()
    abort.current = controller
    setFailure(null)
    setDone(null)
    follow(to === 'zone' ? 'place' : 'package')
    const request: PrintRequest = { workspaceId: store.session.workspaceId, surfaceId: target.surfaceId, pathId: free ? path?.id ?? null : null, paper, frameId: frame?.id ?? null, locale }
    const made: { pdf?: PrintMade } = {}
    const make = async () => {
      made.pdf = await makePrintPdf(shell.client, request, controller.signal, follow)
      follow('save')
      return made.pdf
    }
    try {
      if (to === 'zone') {
        const saved = await save({
          name: fileName,
          kind: 'file',
          grant: async () => (await make()).pdfObjId,
          // the mock Zone of the Desktop's mock runtime takes the bytes themselves
          mockFiles: async () => [{ path: '', blob: await readPdf(shell.client.transport, made.pdf?.pdfObjId ?? '') }],
        })
        if (saved.status === 'cancelled' || !made.pdf) return
        const text = t('aiworkspace.print.savedToZone', { path: saved.path })
        store.notify('info', text)
        setDone({ text, made: made.pdf })
      } else {
        const pdf = await make()
        saveBlob(await readPdf(shell.client.transport, pdf.pdfObjId, controller.signal), fileName)
        setDone({ text: t('aiworkspace.print.downloaded', { name: fileName }), made: pdf })
      }
    } catch (error) {
      setFailure(failureOf(error))
    } finally {
      abort.current = null
      phaseAt.current = null
      setPhase(null)
    }
  }
  const cancel = () => abort.current?.abort()
  const close = () => { if (!busy) onClose() }

  const phaseText: Record<PrintPhase, string> = {
    place: t('aiworkspace.print.phase.place'),
    package: t('aiworkspace.print.phase.package'),
    render: t('aiworkspace.print.phase.render'),
    save: t('aiworkspace.print.phase.save'),
  }
  const skipReason = (s: ResolvedStep) => (s.step.enabled === false ? t('aiworkspace.print.step.disabled')
    : s.problem && s.problem !== 'rotated' ? STEP_PROBLEM_TEXT[s.problem] : t('aiworkspace.print.step.noExport'))
  const issueText: Record<string, (count: number) => string> = {
    step_skipped: (count) => t('aiworkspace.print.issue.stepSkipped', { count }),
    surface_not_exportable: (count) => t('aiworkspace.print.issue.surfaceNotExportable', { count }),
    surface_unreadable: (count) => t('aiworkspace.print.issue.surfaceUnreadable', { count }),
    content_unreadable: (count) => t('aiworkspace.print.issue.contentUnreadable', { count }),
    asset_missing: (count) => t('aiworkspace.print.issue.assetMissing', { count }),
    asset_omitted: (count) => t('aiworkspace.print.issue.assetOmitted', { count }),
  }
  const label = t('aiworkspace.print.title', { name: title })

  return (
    <Modal label={label} testId="aiws-print-dialog" className="aiws-print-dialog" onClose={close}>
      <div className="aiws-dialog-head"><b className="aiws-user-text">{label}</b><span className="aiws-grow" /><button type="button" className="aiws-tool aiws-tool-small aiws-dialog-close" aria-label={t('aiworkspace.common.close')} title={t('aiworkspace.common.close')} disabled={busy} data-testid="aiws-print-close" onClick={close}><X size={16} aria-hidden="true" /></button></div>
      <div className="aiws-muted aiws-print-hint">{free ? t('aiworkspace.print.hintFree') : t('aiworkspace.print.hintFlow')}</div>

      <div className="aiws-print-fields">
        {free && target.paths.length > 1 && (
          <label className="aiws-print-field">{t('aiworkspace.print.path')}
            <select data-testid="aiws-print-path" value={path?.id ?? ''} disabled={busy} onChange={(event) => { setPathId(event.target.value); setFrameId(null) }}>
              {target.paths.map((p) => <option key={p.id} value={p.id}>{p.title}</option>)}
            </select>
          </label>
        )}
        {free && target.paths.length === 1 && path && <div className="aiws-print-field" data-testid="aiws-print-path-name">{t('aiworkspace.print.path')}<span className="aiws-user-text">{path.title}</span></div>}
        <div className="aiws-print-field">
          <label htmlFor="aiws-print-paper">{t('aiworkspace.print.paper')}</label>
          <div className="aiws-print-paper">
            <select id="aiws-print-paper" data-testid="aiws-print-paper" value={paperName} disabled={busy}
              onChange={(event) => setChosenPaper({ name: event.target.value as PaperName, orientation })}>
              {PAPER_NAMES.map((name) => <option key={name} value={name}>{paperLabel(name, t)}</option>)}
            </select>
            <div className="aiws-segmented" role="group" aria-label={t('aiworkspace.print.orientation')} data-testid="aiws-print-orientation">
              {(['portrait', 'landscape'] as Orientation[]).map((o) => (
                <button key={o} type="button" aria-pressed={orientation === o} disabled={busy} data-testid={`aiws-print-${o}`} onClick={() => setChosenPaper({ name: paperName, orientation: o })}>
                  {o === 'portrait' ? t('aiworkspace.print.portrait') : t('aiworkspace.print.landscape')}
                </button>
              ))}
            </div>
          </div>
        </div>
        <label className="aiws-print-field">{t('aiworkspace.print.frame')}
          <select data-testid="aiws-print-frame" value={frame?.id ?? ''} disabled={busy} onChange={(event) => setFrameId(event.target.value || null)}>
            <option value="">{t('aiworkspace.print.frameNone')}</option>
            {frames.map((f) => {
              const fits = frameFits(f.geometry, paper)
              return <option key={f.id} value={f.id} disabled={!fits}>{fits ? f.title : t('aiworkspace.print.frameMismatch', { name: f.title })}</option>
            })}
          </select>
        </label>
        {frames.length > 0 && !frames.some((f) => frameFits(f.geometry, paper)) && <div className="aiws-muted aiws-print-note" data-testid="aiws-print-frame-none-fits">{t('aiworkspace.print.frameNoneFits')}</div>}
      </div>

      <FontNotice check={fonts} isAdmin={isAdmin} canSet={settings.writeReason === null} onFonts={onFonts} list={list} />

      <div className="aiws-print-pages" data-testid="aiws-print-pages">
        {done?.made.pages != null ? t('aiworkspace.print.pagesMade', { count: done.made.pages })
          : free ? (elsewhere > 0 ? t('aiworkspace.print.pagesElsewhere', { count: pages.length, other: elsewhere }) : t('aiworkspace.print.pages', { count: pages.length }))
            : t('aiworkspace.print.pagesAuto')}
      </div>
      {skipped.length > 0 && !done && (
        <ul className="aiws-print-issues" data-testid="aiws-print-skipped" aria-label={t('aiworkspace.print.skippedLabel')}>
          {skipped.map((s) => <li key={s.step.id} data-step={s.step.id}>{t('aiworkspace.print.step.skipped', { n: s.index + 1, title: s.title, reason: skipReason(s) })}</li>)}
        </ul>
      )}
      {done && done.made.issues.length > 0 && (
        <ul className="aiws-print-issues" data-testid="aiws-print-issues" aria-label={t('aiworkspace.print.issuesLabel')}>
          {issueCounts(done.made.issues).map(({ code, count }) => <li key={code} data-code={code}>{issueText[code]?.(count) ?? code}</li>)}
        </ul>
      )}

      {blocked && !busy && <div className="aiws-warning" role="status" data-testid="aiws-print-blocked">{blocked}</div>}
      {busy && (
        <div className="aiws-print-progress" role="status" data-testid="aiws-print-progress" data-phase={phase}>
          <span className="aiws-print-spinner" aria-hidden="true" />
          <span className="aiws-grow">{phaseText[phase]}</span>
          {phase !== 'place' && <button type="button" data-testid="aiws-print-cancel" disabled={phase === 'save'} onClick={cancel}>{t('aiworkspace.common.cancel')}</button>}
        </div>
      )}
      {failure && <div className={failure.kind === 'cancelled' ? 'aiws-muted' : 'aiws-error'} role={failure.kind === 'cancelled' ? 'status' : 'alert'} data-testid="aiws-print-failure" data-kind={failure.kind}>{failure.text}</div>}
      {failure?.kind === 'browser_missing' && !busy && <BrowserMissing isAdmin={isAdmin} />}
      {done && <div className="aiws-print-done" role="status" data-testid="aiws-print-done">{done.text}</div>}

      <div className="aiws-dialog-actions">
        <button type="button" data-testid="aiws-print-device" disabled={busy || blocked !== null} onClick={() => { void run('device') }}>{t('aiworkspace.print.toDevice')}</button>
        <span className="aiws-grow" />
        <button type="button" className="is-primary" data-testid="aiws-print-zone" disabled={busy || blocked !== null} onClick={() => { void run('zone') }}>{t('aiworkspace.print.toZone')}</button>
      </div>
    </Modal>
  )
}

/** §3.5 提示: the mapped fonts the node does not have; printing goes on with substitutes. */
function FontNotice({ check, isAdmin, canSet, onFonts, list }: { check: FontCheck; isAdmin: boolean | null; canSet: boolean; onFonts: () => void; list: (items: readonly string[]) => string }) {
  const { t } = useAiwsText()
  const set = canSet ? <button type="button" className="aiws-link" data-testid="aiws-print-fonts-settings" onClick={onFonts}>{t('aiworkspace.print.fonts.change')}</button> : null
  if (check.state === 'checking') return set ? <div className="aiws-print-note">{set}</div> : null
  if (check.state === 'unknown' && check.missing.length === 0) {
    return <div className="aiws-muted aiws-print-note" data-testid="aiws-print-fonts-unknown">{t('aiworkspace.print.fonts.unknown', { fonts: list(check.unchecked) })} {set}</div>
  }
  if (check.missing.length === 0) return set ? <div className="aiws-print-note">{set}</div> : null
  const names = list(check.missing.map((m) => m.family))
  const packaged = check.missing.some((m) => m.item !== null)
  return (
    <div className="aiws-warning aiws-print-fonts" role="status" data-testid="aiws-print-fonts-missing">
      <div>{t('aiworkspace.print.fonts.missing', { fonts: names })}</div>
      {check.missing.some((m) => m.item === null) && <div className="aiws-muted">{t('aiworkspace.print.fonts.unknownFamily', { fallback: list([SYSTEM_DEFAULT.sans, SYSTEM_DEFAULT.serif, SYSTEM_DEFAULT.mono]) })}</div>}
      <div className="aiws-print-fonts-actions">
        {packaged && isAdmin === true && <button type="button" data-testid="aiws-print-fonts-install" onClick={openExtTools}>{t('aiworkspace.print.fonts.install')}</button>}
        {packaged && isAdmin === false && <span className="aiws-muted" data-testid="aiws-print-fonts-ask">{t('aiworkspace.print.fonts.askAdmin')}</span>}
        {set}
      </div>
    </div>
  )
}

const BROWSER_ITEM = 'chromium'
const BROWSER_PACKAGE = 'ext-core'

/** `media.page` failed with `tools_missing` for `chromium` (§4.1 可用性): an administrator downloads it here — the next
 * print uses it, nothing restarts — anyone else is told whom to ask. */
function BrowserMissing({ isAdmin }: { isAdmin: boolean | null }) {
  const { t } = useAiwsText()
  if (isAdmin === null) return null
  if (!isAdmin) return <div className="aiws-muted" data-testid="aiws-print-browser-ask">{t('aiworkspace.print.browser.askAdmin')}</div>
  return <BrowserDownload />
}

function BrowserDownload() {
  const { t } = useAiwsText()
  const desktop = useI18n().t
  const [status, setStatus] = useState<ExtToolsStatusResponse | null>(null)
  const [failure, setFailure] = useState<string | null>(null)
  const [requests, setRequests] = useState<Record<string, OnlineRequest>>({})
  const [asking, setAsking] = useState(false)
  const load = useCallback(async () => {
    const { data, error } = await fetchExtToolsStatus()
    if (data) setStatus(data)
    setFailure(error ? String(error) : null)
  }, [])
  const pkg = packageOfItem(status, BROWSER_ITEM)
  const nodes = Object.entries(pkg?.nodes ?? {}).flatMap(([node, report]) => (report.online?.[BROWSER_ITEM] ? [{ node, status: report.online[BROWSER_ITEM] }] : []))
  const following = nodes.some(({ node, status: item }) => onlineBusy(item, requests[node]))
  const ready = nodes.length > 0 && nodes.every(({ status: item }) => item.state === 'ready')
  const startable = nodes.filter(({ node, status: item }) => !onlineBusy(item, requests[node]) && (item.state === 'missing' || item.state === 'failed'))
  useEffect(() => {
    const first = window.setTimeout(() => { void load() }, 0)
    return () => window.clearTimeout(first)
  }, [load])
  useEffect(() => {
    if (!following) return
    const timer = window.setInterval(() => { void load() }, ONLINE_POLL_MS)
    return () => window.clearInterval(timer)
  }, [load, following])
  const download = async () => {
    setAsking(true)
    const asked = await Promise.all(startable.map(({ node }) => downloadExtToolsOnline(pkg?.id ?? BROWSER_PACKAGE, BROWSER_ITEM, node)))
    setAsking(false)
    const refused = asked.find((result) => result.error)
    setFailure(refused ? String(refused.error) : null)
    setRequests((current) => ({ ...current, ...Object.fromEntries(startable.map(({ node, status: item }) => [node, onlineRequest(item)])) }))
    void load()
  }
  return (
    <div className="aiws-print-browser" data-testid="aiws-print-browser" data-state={ready ? 'ready' : following ? 'downloading' : 'missing'}>
      {nodes.map(({ node, status: item }) => {
        const pending = requestPending(requests[node], item)
        const problem = onlineErrorText(item, desktop)
        return (
          <div key={node} className="aiws-print-browser-node" data-node={node}>
            <div>{nodes.length > 1 ? `${node}: ` : ''}{onlineStateLabel(item, pending, desktop)}</div>
            {item.state === 'downloading' && <OnlineProgress status={item} />}
            {problem && <div className="aiws-error">{problem}</div>}
          </div>
        )
      })}
      {ready && <div data-testid="aiws-print-browser-ready">{t('aiworkspace.print.browser.ready')}</div>}
      <div className="aiws-print-fonts-actions">
        {startable.length > 0 && (
          <button type="button" className="is-primary" data-testid="aiws-print-browser-download" disabled={asking} onClick={() => { void download() }}>
            {startable.some(({ status: item }) => item.state === 'failed') ? t('aiworkspace.print.browser.retry') : t('aiworkspace.print.browser.download')}
          </button>
        )}
        <button type="button" className="aiws-link" data-testid="aiws-print-browser-settings" onClick={openExtTools}>{t('aiworkspace.print.browser.settings')}</button>
      </div>
      {failure && <div className="aiws-error">{failure}</div>}
    </div>
  )
}
