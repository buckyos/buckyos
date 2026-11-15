/* eslint-disable react-refresh/only-export-components -- the dialogs and their readers */
/* The windows a wish's records open (许愿格交互升级 §3.4): the request as it was, the task contract, the macro's code, its
 * log, and a version of an object as a run left it — read only, the macro of the current record editable where the user
 * may (the frame is `confirm.tsx`'s). */

import { useCallback, type ReactNode } from 'react'
import { unwrap } from '../../api/client'
import type { Json, Result, WishAnalysis, WishInput } from '../../api/types'
import { useLoad, useStore } from '../../state/hooks'
import type { WorkspaceStore } from '../../state/store'
import { useAiwsText } from '../../useAiwsText'
import { WishDialog } from './confirm'
import { CodeLines, ProgramView } from './ProgramView'

// ---- reading what a record points at

export async function objectText(store: WorkspaceStore, objectId: string): Promise<string> {
  return (await store.session.fetchAsset(objectId)).text()
}

/** An object as it was at one version, found among the entity's versions (`doc.list_versions`) and read through a fixed
 * reference (`doc.resolve`); null when that version is not kept. `current`: it is the version the entity has now. */
export async function versionOf(store: WorkspaceStore, entityId: string, match: (v: { run_id?: string | null; content_rev: number; object_id: string }) => boolean): Promise<{ object: Json; current: boolean } | null> {
  const listed = await store.session.listVersions(entityId)
  const version = listed.versions.find(match)
  if (!version) return null
  const read = unwrap(await store.session.call<Result<{ object: Json }>>('doc.resolve', {
    workspace_id: store.session.workspaceId, reference: { entity_id: entityId, version: { mode: 'fixed_revision', object_id: version.object_id } },
  }))
  return { object: read.object, current: version.content_rev === listed.content_rev }
}

function Loading<T>({ load, children }: { load: () => Promise<T>; children: (data: T) => ReactNode }) {
  const { t } = useAiwsText()
  const read = useLoad(load)
  if (read.error && read.data === undefined) return <div className="aiws-error" role="alert" data-testid="aiws-wish-dialog-error">{t('aiworkspace.wishcard.dialog.readFailed', { error: read.error })}</div>
  if (read.data === undefined) return <div className="aiws-muted">{t('aiworkspace.wish.loading')}</div>
  return <>{children(read.data)}</>
}

export function TextDialog({ title, kind, objectId, onClose }: { title: string; kind: string; objectId: string; onClose: () => void }) {
  const store = useStore()
  const load = useCallback(() => objectText(store, objectId), [store, objectId])
  return (
    <WishDialog title={title} kind={kind} onClose={onClose} wide={kind === 'log'}>
      <Loading load={load}>{(text) => kind === 'log' ? <CodeLines text={text} /> : <div className="aiws-wd-text">{text}</div>}</Loading>
    </WishDialog>
  )
}

const LEVEL: Record<string, string> = { error: 'aiworkspace.wishcard.check.error', warning: 'aiworkspace.wishcard.check.warning' }
const KIND: Record<string, string> = { program: 'aiworkspace.wishcard.check.program', review: 'aiworkspace.wishcard.check.review' }

/** The task contract as a stage left it: the task description, the results it promises, its acceptance checks. */
export function AnalysisView({ analysis, inputs }: { analysis: WishAnalysis | undefined; inputs?: WishInput[] }) {
  const { t, list, clauses } = useAiwsText()
  const store = useStore()
  if (!analysis) return <div className="aiws-muted">{t('aiworkspace.wishcard.dialog.noAnalysis')}</div>
  const contract = analysis.output_contract?.results ?? []
  return (
    <div className="aiws-wd-analysis" data-testid="aiws-wish-analysis-view">
      {analysis.warnings?.length > 0 && <div className="aiws-warning"><b>{clauses(analysis.warnings)}</b></div>}
      <div className="aiws-wd-section"><div className="aiws-muted">{t('aiworkspace.wish.contextPrompt')}</div><div className="aiws-wd-text">{analysis.context_prompt}</div></div>
      {contract.length > 0 && (
        <div className="aiws-wd-section">
          <div className="aiws-muted">{t('aiworkspace.wishcard.dialog.contract')}</div>
          <table className="aiws-rt-table aiws-wd-table" data-testid="aiws-wish-contract">
            <thead><tr><th>{t('aiworkspace.wish.contract.result')}</th><th>{t('aiworkspace.wish.contract.type')}</th><th>{t('aiworkspace.wish.contract.views')}</th></tr></thead>
            <tbody>{contract.map((r) => (
              <tr key={r.name}><td><code>{r.name}</code> {r.title}</td><td>{r.type}{r.key ? t('aiworkspace.wish.contract.key', { key: r.key.join('+') }) : ''}{r.target ? ` → ${r.target}` : ''}</td><td>{list((r.views ?? []).map((v) => v.renderer)) || t('aiworkspace.wish.contract.defaultView')}</td></tr>
            ))}</tbody>
          </table>
        </div>
      )}
      {analysis.checks?.length > 0 && (
        <div className="aiws-wd-section">
          <div className="aiws-muted">{t('aiworkspace.wishcard.dialog.checks')}</div>
          <ul className="aiws-wd-list" data-testid="aiws-wish-checkdefs">
            {analysis.checks.map((c) => (
              <li key={c.id} data-level={c.level ?? ''}>
                <span className={`aiws-chip${c.level === 'error' ? ' aiws-chip-warn' : ''}`}>{t(KIND[c.kind] ?? KIND.program)}{c.level ? ` · ${t(LEVEL[c.level])}` : ''}</span> {c.text}
              </li>
            ))}
          </ul>
        </div>
      )}
      {inputs && inputs.length > 0 && (
        <div className="aiws-wd-section">
          <div className="aiws-muted">{t('aiworkspace.wish.inputs')}</div>
          <div>{list(inputs.map((i) => i.label ?? store.outline.get(i.entity_id)?.title ?? store.outline.get(i.entity_id)?.name ?? i.entity_id))}</div>
        </div>
      )}
    </div>
  )
}

export function AnalysisDialog({ objectId, onClose }: { objectId: string; onClose: () => void }) {
  const { t } = useAiwsText()
  const store = useStore()
  const load = useCallback(async () => JSON.parse(await objectText(store, objectId)) as { analysis?: WishAnalysis; inputs?: WishInput[] }, [store, objectId])
  return (
    <WishDialog title={t('aiworkspace.wishcard.dialog.analysis')} kind="analysis" onClose={onClose} wide>
      <Loading load={load}>{(data) => <AnalysisView analysis={data.analysis} inputs={data.inputs} />}</Loading>
    </WishDialog>
  )
}

/** The macro of a record: the current one editable where the user may (the data mode's code page), an earlier one read only. */
export function ProgramDialog({ wishId, sourceId, current, canEdit, onClose }: { wishId: string; sourceId: string | undefined; current: boolean; canEdit: boolean; onClose: () => void }) {
  const { t } = useAiwsText()
  const store = useStore()
  const load = useCallback(() => (sourceId ? objectText(store, sourceId) : Promise.resolve('')), [store, sourceId])
  return (
    <WishDialog title={t(current ? 'aiworkspace.wishcard.dialog.program' : 'aiworkspace.wishcard.dialog.programOld')} kind="program" onClose={onClose} wide>
      {current
        ? <ProgramView wishId={wishId} canEdit={canEdit} />
        : <Loading load={load}>{(text) => (text ? <CodeLines text={text} /> : <div className="aiws-muted">{t('aiworkspace.wish.program.none')}</div>)}</Loading>}
    </WishDialog>
  )
}

/** Plain text of a rich-text document (paragraphs on their own lines). */
function docText(node: { text?: string; content?: unknown[]; type?: string } | undefined): string {
  if (!node) return ''
  if (node.text) return node.text
  const parts = ((node.content ?? []) as { text?: string; content?: unknown[]; type?: string }[]).map(docText)
  const block = node.type && ['paragraph', 'heading', 'list_item', 'code_block', 'blockquote', 'table_row'].includes(node.type)
  return block ? `${parts.join('')}\n` : parts.join('')
}

/** A version's content, read only: rich text as its text, a record as its properties, anything else as data. */
function VersionContent({ object }: { object: Json }) {
  const o = (object ?? {}) as { ws_type?: string; content?: Record<string, unknown> }
  const content = o.content ?? {}
  const doc = (content.content ?? (content.payload as Record<string, unknown> | undefined)?.content) as { type?: string } | undefined
  if (doc && typeof doc === 'object' && doc.type === 'doc') return <div className="aiws-wd-text">{docText(doc as Parameters<typeof docText>[0]).trim()}</div>
  const props = (content.props ?? (content.payload as Record<string, unknown> | undefined)?.props) as Record<string, Json> | undefined
  if (props && typeof props === 'object') {
    return <dl className="aiws-wd-props">{Object.entries(props).map(([k, v]) => <div key={k}><dt>{k}</dt><dd>{typeof v === 'string' || typeof v === 'number' || typeof v === 'boolean' ? String(v) : JSON.stringify(v)}</dd></div>)}</dl>
  }
  return <pre className="aiws-wd-json">{JSON.stringify(content, null, 2).slice(0, 20000)}</pre>
}

/** An object as one run left it (or read it). Read only; `onMissing` when that version is not kept any more. */
export function VersionDialog({ title, entityId, match, onClose }: { title: string; entityId: string; match: (v: { run_id?: string | null; content_rev: number; object_id: string }) => boolean; onClose: () => void }) {
  const { t } = useAiwsText()
  const store = useStore()
  const load = useCallback(() => versionOf(store, entityId, match), [store, entityId, match])
  return (
    <WishDialog title={t('aiworkspace.wishcard.dialog.version', { name: title })} kind="version" onClose={onClose} wide>
      <Loading load={load}>{(found) => (found ? <VersionContent object={found.object} /> : <div className="aiws-muted" data-testid="aiws-wish-version-missing">{t('aiworkspace.wishcard.dialog.versionMissing')}</div>)}</Loading>
    </WishDialog>
  )
}
