/* Making the PDF of a print (打印 TODO T7, the Desktop's part): the aiworkspace service builds the print package and
 * hands it to the caller (`print.package`), then the Workflow's system definition `media.page` renders it with the
 * caller's own session — the Run's output PDF is then held for the caller too (`get_run_output` at the end), so its
 * object id goes straight to the Zone (`commit_object`) or is read back from NDM for a download.
 *
 * Against the standalone backend (the dev override of api/transport.ts) the Workflow is reached as plain kRPC at
 * `/kapi/workflow` with the override's token; the e2e suite answers there. */

import { buckyos, WorkflowRequestError, WorkflowRunStatus, type WorkflowClient, type WorkflowExecutionError, type WorkflowRunOutput } from 'buckyos'
import type { Paper } from '@print/page'
import { paperArgument } from '@print/page'
import { READY_EXPRESSION, type PrintIssue, type PrintIssueCode, type PrintJob } from '@print/package'
import { resolveNdmSource } from '../../../../components/preview/ndmSource'
import { formatPageDate } from '../controls/pdfframe/context'
import { unwrap, type AiwsClient } from '../../api/client'
import type { Transport } from '../../api/transport'

export const PAGE_DEFINITION = 'media.page'
const POLL_MS = 1000
/** `media.page` gives up after 10 minutes; past this the Run is not looked at any longer. */
const GIVE_UP_MS = 11 * 60_000

/** Where a print is: the place being chosen, the package being built, the pages laid out, the file written. */
export type PrintPhase = 'place' | 'package' | 'render' | 'save'

export interface PrintRequest {
  workspaceId: string
  surfaceId: string
  pathId: string | null
  paper: Paper
  frameId: string | null
  locale: string
}

export interface PrintMade {
  pdfObjId: string
  pages: number | null
  issues: PrintIssue[]
  job: PrintJob
}

/** Why no PDF came out, as the dialog tells it. `browser_missing`: the node has not downloaded Chromium yet. */
export type PrintFailureKind = 'cancelled' | 'browser_missing' | 'tools_missing' | 'render' | 'deliver'

export class PrintFailure extends Error {
  readonly kind: PrintFailureKind
  readonly detail: string
  constructor(kind: PrintFailureKind, detail = '') {
    super(detail || kind)
    this.name = 'PrintFailure'
    this.kind = kind
    this.detail = detail
  }
}

type PageRuns = Pick<WorkflowClient, 'createRun' | 'startRun' | 'getRunOutput' | 'cancelRun'>

/** The Workflow as the standalone backend's dev override reaches it: kRPC on this origin with the override's token. */
function devRuns(endpoint: string, token: string | null): PageRuns {
  let seq = 1
  const call = async <T>(method: string, params: object): Promise<T> => {
    const response = await fetch(`${endpoint}/kapi/workflow`, {
      method: 'POST', headers: { 'Content-Type': 'application/json' }, body: JSON.stringify({ method, params, sys: token ? [seq++, token] : [seq++] }),
    })
    if (!response.ok) throw new Error(`workflow: HTTP ${response.status}`)
    const body = await response.json() as { result?: Record<string, unknown>; error?: string }
    if (body.error) throw new Error(`workflow: ${body.error}`)
    const result = body.result ?? {}
    if (result.ok !== true) throw new WorkflowRequestError(String(result.error ?? 'workflow_error'), String(result.message ?? result.error ?? 'workflow_error'), result.detail)
    return result as T
  }
  return {
    createRun: (request) => call('create_run', request),
    startRun: (runId, waitMs) => call('start_run', waitMs == null ? { run_id: runId } : { run_id: runId, wait_ms: waitMs }),
    getRunOutput: (runId) => call('get_run_output', { run_id: runId }),
    cancelRun: (request) => call('cancel_run', request),
  }
}

async function pageRuns(transport: Transport): Promise<PageRuns> {
  if (transport.mode === 'dev-override') {
    const { endpoint, sessionToken } = await transport.importOptions()
    return devRuns(endpoint, sessionToken)
  }
  return buckyos.getWorkflowClient()
}

function cancelled(): PrintFailure {
  return new PrintFailure('cancelled')
}

function pause(ms: number, signal: AbortSignal): Promise<void> {
  return new Promise((resolve, reject) => {
    if (signal.aborted) { reject(cancelled()); return }
    const timer = window.setTimeout(() => { signal.removeEventListener('abort', stop); resolve() }, ms)
    const stop = () => { window.clearTimeout(timer); reject(cancelled()) }
    signal.addEventListener('abort', stop, { once: true })
  })
}

function runError(output: WorkflowRunOutput): WorkflowExecutionError | null {
  return output.run_error ?? Object.values(output.node_errors ?? {})[0] ?? null
}

/** A finished Run as the PDF it made, or why it made none. */
function finished(output: WorkflowRunOutput): { pdfObjId: string; pages: number | null } {
  if (output.status === WorkflowRunStatus.Completed) {
    const page = output.output.page_to_pdf as { pdf_obj_id?: unknown; pages?: unknown } | undefined
    const pdfObjId = typeof page?.pdf_obj_id === 'string' ? page.pdf_obj_id : ''
    if (!pdfObjId) throw new PrintFailure('render', 'media.page returned no PDF')
    // the requester holds the output once the finished Run was read: nothing to read it with otherwise
    if (output.delivery.held === 0 && output.delivery.hold_error) throw new PrintFailure('deliver', output.delivery.hold_error)
    return { pdfObjId, pages: typeof page?.pages === 'number' ? page.pages : null }
  }
  const error = runError(output)
  if (error?.code === 'tools_missing') {
    const details = (error.details ?? {}) as { online_item?: unknown; tools_missing?: unknown }
    if (details.online_item === 'chromium') throw new PrintFailure('browser_missing', error.message)
    throw new PrintFailure('tools_missing', Array.isArray(details.tools_missing) ? details.tools_missing.join(', ') : error.message)
  }
  if (output.status === WorkflowRunStatus.Aborted && (!error || error.code === 'cancelled')) throw cancelled()
  throw new PrintFailure('render', error ? `${error.code}: ${error.message}` : output.status)
}

/** The print package, then `media.page` on it; `onPhase` follows the steps. The Run is cancelled with `signal`. */
export async function makePrintPdf(client: AiwsClient, request: PrintRequest, signal: AbortSignal, onPhase: (phase: PrintPhase) => void): Promise<PrintMade> {
  onPhase('package')
  const pkg = unwrap(await client.printPackage({
    workspace_id: request.workspaceId,
    surface_id: request.surfaceId,
    ...(request.pathId ? { path_id: request.pathId } : {}),
    paper: request.paper,
    ...(request.frameId ? { frame_id: request.frameId } : {}),
    date: formatPageDate(),
    locale: request.locale,
  }, signal))
  if (signal.aborted) throw cancelled()
  onPhase('render')
  const runs = await pageRuns(client.transport)
  const created = await runs.createRun({
    workflow_id: PAGE_DEFINITION,
    input: { obj_id: pkg.obj_id, output: 'pdf', paper: paperArgument(request.paper), landscape: false, margin_mm: 0, background: true, wait_ready: READY_EXPRESSION },
  })
  const runId = created.run_id
  const cancel = () => { void runs.cancelRun({ run_id: runId, reason: 'cancelled by the person printing' }).catch(() => {}) }
  signal.addEventListener('abort', cancel, { once: true })
  try {
    if (signal.aborted) { cancel(); throw cancelled() }
    await runs.startRun(runId)
    const until = Date.now() + GIVE_UP_MS
    for (;;) {
      if (signal.aborted) throw cancelled()
      const output = await runs.getRunOutput(runId)
      if (output.terminal) {
        if (signal.aborted) throw cancelled()
        return { ...finished(output), issues: pkg.issues, job: pkg.job }
      }
      if (Date.now() > until) throw new PrintFailure('render', output.status)
      await pause(POLL_MS, signal)
    }
  } finally {
    signal.removeEventListener('abort', cancel)
  }
}

/** The PDF's bytes, read from NDM with the caller's own session (a download to this device). */
export async function readPdf(transport: Transport, objId: string, signal?: AbortSignal): Promise<Blob> {
  const { endpoint, sessionToken } = await transport.importOptions()
  const resolved = await resolveNdmSource({ kind: 'object-id', objectId: objId }, endpoint, sessionToken, signal)
  if (resolved.readRef.kind !== 'blob') throw new PrintFailure('deliver', 'the PDF could not be read')
  return new Blob([resolved.readRef.blob], { type: 'application/pdf' })
}

/** The service's issues by kind, in a stable order: what the dialog lists after a print. */
export function issueCounts(issues: readonly PrintIssue[]): { code: PrintIssueCode; count: number }[] {
  const order: PrintIssueCode[] = ['step_skipped', 'surface_not_exportable', 'surface_unreadable', 'content_unreadable', 'asset_missing', 'asset_omitted']
  const counts = new Map<PrintIssueCode, number>()
  for (const issue of issues) counts.set(issue.code, (counts.get(issue.code) ?? 0) + 1)
  return [...counts.entries()].sort((a, b) => order.indexOf(a[0]) - order.indexOf(b[0])).map(([code, count]) => ({ code, count }))
}
