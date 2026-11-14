/* The editor of a tab (UI 框架 PRD §5.3): each type's standard editor, filling the editing area — the table editor and
 * the directory editor in their tab size, rich text, the record form, an asset's preview and replace, a macro's three
 * pages (task, code, runs), a control definition's summary, a note, a folder's items, a presentation path's or a
 * viewport's summary with "在路径编辑中打开", the Workspace's permission editor, and the raw content of the rest
 * (read only). The data layer's limits hold: a sourced object, a write lock or a missing capability stays read only. */

import { createElement, useCallback, type ReactNode } from 'react'
import { ExternalLink } from 'lucide-react'
import type { ReadOk } from '../../api/session'
import type { BlockDefRead, EntityEnvelope, Reference, ShowPathPayload, ViewportPayload, WishPayloadRead, WishRunView } from '../../api/types'
import { useDirectReadOnly, useLoad, useReadOnlyReason, useStore, useVersion, useWorkspaceUi } from '../../state/hooks'
import { useAiwsText } from '../../useAiwsText'
import { AssetCell, NoteEditor, RecordCell, RichTextCell } from '../blocks/editors'
import { DirectoryEditor } from '../directory/DirectoryEditor'
import { WorkspacePermissionEditor } from '../permissions'
import { useShell } from '../shell/shellContext'
import { FreshnessBadge } from '../sources/FreshnessBadge'
import { entityLabel } from '../sources/dataOps'
import { StandardTableEditor } from '../table/StandardTableEditor'
import { ProgramView, WishPanel } from '../wish/WishPanel'
import { iconOf, nameOf, readOnlyReason, SESSION_TYPES, typeName } from './model'
import { PERMISSIONS_TAB, type MacroPage } from './tabs'

/** The editor of one tab; `onPage` turns a macro's page. */
export function TabEditor({ id, entity, page, onPage, onOpen }: { id: string; entity: EntityEnvelope | undefined; page: MacroPage | undefined; onPage: (page: MacroPage) => void; onOpen: (id: string) => void }) {
  if (id === PERMISSIONS_TAB) return <div className="aiws-detail aiws-dm-perm-editor" data-testid="aiws-detail-permissions"><WorkspacePermissionEditor /></div>
  if (!entity || entity.deleted) return <GoneEditor id={id} entity={entity} />
  return <DataDetail entity={entity} page={page} onPage={onPage} onOpen={onOpen} />
}

function GoneEditor({ id, entity }: { id: string; entity: EntityEnvelope | undefined }) {
  const { t } = useAiwsText()
  return (
    <div className="aiws-empty aiws-dm-gone" data-testid="aiws-dm-gone" data-entity-id={id}>
      <p>{entity?.deleted ? t('aiworkspace.datamode.gone.deleted', { name: entityLabel(entity) }) : t('aiworkspace.datamode.gone.unreadable', { id })}</p>
      <p className="aiws-muted">{t('aiworkspace.datamode.gone.hint')}</p>
    </div>
  )
}

/** Types whose editor draws its own title row (§7.2): no head of the tab's own. */
const OWN_HEAD = new Set(['buckyos.table-source', 'buckyos.dir'])

export function DataDetail({ entity, page, onPage, onOpen }: { entity: EntityEnvelope; page: MacroPage | undefined; onPage: (page: MacroPage) => void; onOpen: (id: string) => void }) {
  const { t } = useAiwsText()
  const store = useStore()
  const windowReason = useReadOnlyReason()
  const readOnly = useDirectReadOnly()
  const reason = readOnlyReason(store, entity, windowReason)
  // data a canvas owns (内核架构改进 D05): pinning ends the ownership and moves it to the data tree
  const ownData = entity.type_id !== 'buckyos.container' && store.outline.ancestors(entity.entity_id).includes('canvas-content')
  const pin = async () => {
    const key = store.core.order_key_between(store.outline.childrenOf('data').at(-1)?.order_key ?? undefined, undefined)
    await store.submit({ editId: `pin:${entity.entity_id}`, label: t('aiworkspace.data.detail.pinLabel', { name: entityLabel(entity) }), operations: [
      { op: 'data.pin', entity_id: entity.entity_id },
      { op: 'tree.move', entity_id: entity.entity_id, new_parent_id: 'data', order_key: key },
    ] })
  }
  const head = !OWN_HEAD.has(entity.type_id) || entity.degraded
  return (
    <div className={`aiws-detail aiws-dm-detail${head ? '' : ' is-bare'}`} data-testid={`aiws-detail-${entity.entity_id}`} data-type={entity.type_id} data-read-only={reason ? 'true' : 'false'}>
      {head && (
        <div className="aiws-detail-head">
          {createElement(iconOf(entity), { size: 18, 'aria-hidden': true })}
          <b>{nameOf(store, entity, entity.entity_id)}</b>
          <span className="aiws-muted">{typeName(entity)}</span>
          {entity.derived && <FreshnessBadge entityId={entity.entity_id} />}
          <span className="aiws-grow" />
          {reason && entity.type_id !== 'buckyos.container' && <span className="aiws-chip" data-testid="aiws-dm-readonly" title={reason}>{reason}</span>}
        </div>
      )}
      {ownData && (
        <div className="aiws-inline-form aiws-dm-own" data-testid="aiws-own-data">
          <span className="aiws-muted">{t('aiworkspace.data.detail.ownData')}</span>
          {entity.capabilities.includes('structure') && !readOnly && <button type="button" data-testid="aiws-data-pin" onClick={() => { void pin() }}>{t('aiworkspace.data.detail.pin')}</button>}
        </div>
      )}
      <div className={`aiws-dm-content is-${entity.type_id.replace('buckyos.', '')}`}>
        <DataContent entity={entity} reason={reason} readOnly={readOnly} page={page} onPage={onPage} onOpen={onOpen} />
      </div>
    </div>
  )
}

function DataContent({ entity, reason, readOnly, page, onPage, onOpen }: { entity: EntityEnvelope; reason: string | null; readOnly: boolean; page: MacroPage | undefined; onPage: (page: MacroPage) => void; onOpen: (id: string) => void }) {
  const ui = useWorkspaceUi()
  const renderEmbed = useCallback((reference: Reference) => ui.renderCell(reference.entity_id, 1), [ui])
  if (entity.degraded) return <RawDetail entity={entity} />
  switch (entity.type_id) {
    case 'buckyos.table-source': return <StandardTableEditor sourceId={entity.entity_id} source={entity} size="tab" readOnly={readOnly || Boolean(entity.source)} readOnlyReason={reason} />
    case 'buckyos.dir': return <DirectoryEditor key={entity.entity_id} entityId={entity.entity_id} size="tab" readOnlyReason={readOnly ? reason : null} />
    case 'buckyos.richtext': return <RichTextCell entity={entity} entityId={entity.entity_id} renderEmbed={renderEmbed} />
    case 'buckyos.record': return <RecordCell entityId={entity.entity_id} entity={entity} readOnly={readOnly} />
    case 'buckyos.asset-ref': return <AssetCell entityId={entity.entity_id} fit="contain" readOnly={readOnly || Boolean(entity.source)} />
    case 'buckyos.annotation': return <NoteEditor entityId={entity.entity_id} entity={entity} readOnly={readOnly} />
    case 'buckyos.wish': return <MacroEditor entity={entity} readOnly={readOnly} page={page ?? 'task'} onPage={onPage} />
    case 'buckyos.block-def': return <BlockDefDetail entity={entity} />
    case 'buckyos.container': return entity.kind === 'folder' || entity.system ? <FolderDetail entity={entity} onOpen={onOpen} /> : <BlockSummary entity={entity} />
    case 'buckyos.cell': return <BlockSummary entity={entity} />
    case 'buckyos.show-path': case 'buckyos.viewport': return <PathSummary entity={entity} />
    default: return <RawDetail entity={entity} />
  }
}

// ---- a macro (a wish with its program): 任务 / 代码 / 运行记录

const PAGES: MacroPage[] = ['task', 'code', 'runs']

function MacroEditor({ entity, readOnly, page, onPage }: { entity: EntityEnvelope; readOnly: boolean; page: MacroPage; onPage: (page: MacroPage) => void }) {
  const { t } = useAiwsText()
  const store = useStore()
  const windowReason = useReadOnlyReason()
  const version = useVersion(`e:${entity.entity_id}`)
  const load = useCallback(() => store.session.read<WishPayloadRead>(entity.entity_id), [store, entity.entity_id])
  const read = useLoad<ReadOk<WishPayloadRead>>(load, version)
  const payload = read.data?.content.payload
  const canUpdate = entity.capabilities.includes('update')
  const codeReason = windowReason ?? (!canUpdate ? t('aiworkspace.datamode.code.noPermission')
    : payload?.executor !== 'xllm' ? t('aiworkspace.datamode.code.mock') : null)
  return (
    <div className="aiws-dm-macro-editor" data-testid={`aiws-dm-macro-editor-${entity.entity_id}`} data-page={page}>
      <div className="aiws-dm-pages" role="tablist" aria-label={t('aiworkspace.datamode.macroPages')}>
        {PAGES.map((p) => <button key={p} type="button" role="tab" aria-selected={page === p} data-testid={`aiws-dm-macro-page-${p}`} onClick={() => onPage(p)}>{t(`aiworkspace.datamode.page.${p}`)}</button>)}
      </div>
      <div className="aiws-dm-page" role="tabpanel">
        {page === 'task' && <WishPanel wishId={entity.entity_id} readOnly={readOnly} />}
        {page === 'code' && (payload && !payload.program
          ? <div className="aiws-muted" data-testid="aiws-dm-code-none">{t(payload.executor === 'xllm' ? 'aiworkspace.datamode.code.none' : 'aiworkspace.datamode.code.mock')}</div>
          : <ProgramView wishId={entity.entity_id} canEdit={!readOnly && codeReason === null} reason={codeReason} page />)}
        {page === 'runs' && <RunList wishId={entity.entity_id} />}
      </div>
    </div>
  )
}

const RUN_STATE = new Set(['queued', 'snapshotting', 'running', 'validating', 'waiting_confirmation', 'applying', 'succeeded', 'failed', 'cancelled', 'interrupted', 'conflict', 'rejected'])

function RunList({ wishId }: { wishId: string }) {
  const { t, dateTime } = useAiwsText()
  const store = useStore()
  const version = useVersion(`e:${wishId}`)
  const load = useCallback(() => store.wish.history(wishId), [store, wishId])
  const runs = useLoad<WishRunView[]>(load, version)
  if (runs.error && !runs.data) return <div className="aiws-error" role="alert">{runs.error}</div>
  if (!runs.data) return <div className="aiws-muted">{t('aiworkspace.wish.loading')}</div>
  if (runs.data.length === 0) return <div className="aiws-muted" data-testid="aiws-dm-runs-empty">{t('aiworkspace.datamode.runs.none')}</div>
  return (
    <ol className="aiws-dm-runs" data-testid="aiws-dm-runs">
      {runs.data.map((run) => (
        <li key={run.run_id} className="aiws-dm-run" data-testid="aiws-dm-run" data-state={run.state} data-stage={run.stage}>
          <b>{t(`aiworkspace.datamode.runs.stage.${run.stage}`)}</b>
          <span className={`aiws-chip${run.state === 'failed' || run.state === 'rejected' || run.state === 'conflict' ? ' aiws-chip-warn' : run.state === 'succeeded' ? ' aiws-chip-ok' : ''}`}>
            {RUN_STATE.has(run.state) ? t(`aiworkspace.datamode.runs.state.${run.state}`) : run.state}
          </span>
          {run.simulated && <span className="aiws-chip aiws-chip-derived">{t('aiworkspace.data.simulated')}</span>}
          <span className="aiws-grow" />
          <span className="aiws-muted">{dateTime(run.created_at)}</span>
          {run.error && <div className="aiws-error aiws-dm-run-error">{run.error.detail ?? run.error.code}</div>}
        </li>
      ))}
    </ol>
  )
}

// ---- the others

function FolderDetail({ entity, onOpen }: { entity: EntityEnvelope; onOpen: (id: string) => void }) {
  const { t } = useAiwsText()
  const store = useStore()
  const children = store.outline.childrenOf(entity.entity_id).filter((c) => c.type_id !== 'buckyos.cell')
  const surface = entity.surface_id ? store.outline.get(entity.surface_id) : undefined
  return (
    <div className="aiws-folder-detail aiws-dm-folder">
      {entity.system === 'surface_content' && surface && <div className="aiws-muted">{t('aiworkspace.data.detail.surfaceContent', { name: entityLabel(surface) })}</div>}
      {entity.system === 'canvas_content' && <div className="aiws-muted">{t('aiworkspace.data.detail.canvasContent')}</div>}
      {entity.system === 'shows' && <div className="aiws-muted">{t('aiworkspace.data.detail.shows')}</div>}
      <ul className="aiws-dm-folder-list">
        {children.map((c) => (
          <li key={c.entity_id}>
            <button type="button" className="aiws-dm-folder-item" data-testid={`aiws-dm-folder-item-${c.entity_id}`} onClick={() => onOpen(c.entity_id)}>
              {createElement(iconOf(c), { size: 16, 'aria-hidden': true })}<span>{nameOf(store, c, c.entity_id)}</span><span className="aiws-muted">{typeName(c)}</span>
            </button>
          </li>
        ))}
      </ul>
      {children.length === 0 && <div className="aiws-muted">{t('aiworkspace.data.detail.emptyFolder')}</div>}
    </div>
  )
}

function BlockDefDetail({ entity }: { entity: EntityEnvelope }) {
  const { t, tNode, list, dateTime } = useAiwsText()
  const store = useStore()
  const shell = useShell()
  const version = useVersion(`e:${entity.entity_id}`)
  const load = useCallback(() => store.session.read<BlockDefRead>(entity.entity_id), [store, entity.entity_id])
  const read = useLoad<ReadOk<BlockDefRead>>(load, version)
  if (!read.data) return <div className="aiws-muted">{read.error ?? t('aiworkspace.data.detail.defLoading')}</div>
  const payload = read.data.content.payload
  const build = (read.data.derived as { build?: { built_at: string } } | null | undefined)?.build
  const users = store.outline.all().filter((e) => e.type_id === 'buckyos.cell' && e.def_id === entity.entity_id && !e.deleted)
  return (
    <div className="aiws-def-detail" data-testid="aiws-def-detail">
      <div>{tNode(payload.accepts ? 'aiworkspace.data.detail.defSummaryAccepts' : 'aiworkspace.data.detail.defSummary', {
        id: <code>{payload.def_id}</code>, version: payload.version ?? 1,
        kind: payload.kind === 'html' ? t('aiworkspace.data.detail.defHtml') : t('aiworkspace.data.detail.defDeclarative'), accepts: list(payload.accepts ?? []),
      })}</div>
      {payload.description && <div className="aiws-muted">{payload.description}</div>}
      {payload.ident && <div className="aiws-muted" data-testid="aiws-def-ident">{t('aiworkspace.data.detail.defIdent', { ident: `${payload.ident.author_did}#${payload.ident.name}` })}</div>}
      {payload.provenance?.pkg_id && <div className="aiws-muted">{t('aiworkspace.data.detail.defProvenance', { pkg: `${payload.provenance.pkg_id} ${payload.provenance.version ?? ''}` })}</div>}
      {build && <div className="aiws-muted">{t('aiworkspace.data.detail.defBuilt', { at: dateTime(build.built_at) })}</div>}
      <div className="aiws-muted">{t('aiworkspace.data.detail.defUsers', { count: users.length })}</div>
      <button type="button" className="aiws-link" data-testid="aiws-def-manage" onClick={() => shell.openDialog({ kind: 'extensions', defId: entity.entity_id })}>{t('aiworkspace.ext.manage')}</button>
      {payload.kind === 'html' && <div className="aiws-warning">{t('aiworkspace.data.detail.defHtmlRisk')}</div>}
      <details><summary>{t('aiworkspace.data.detail.defShow')}</summary><pre className="aiws-raw">{JSON.stringify(payload.kind === 'html' ? { ...payload, html: { ...payload.html, js: t('aiworkspace.data.detail.defChars', { count: payload.html?.js?.length ?? 0 }) } } : payload, null, 2)}</pre></details>
    </div>
  )
}

/** A presentation path or a viewport (§5.3): a read-only summary, edited in the path editing mode of its canvas. */
function PathSummary({ entity }: { entity: EntityEnvelope }) {
  const { t } = useAiwsText()
  const store = useStore()
  const shell = useShell()
  const version = useVersion(`e:${entity.entity_id}`)
  const load = useCallback(() => store.session.read<{ payload: Partial<ShowPathPayload & ViewportPayload> }>(entity.entity_id), [store, entity.entity_id])
  const read = useLoad<ReadOk<{ payload: Partial<ShowPathPayload & ViewportPayload> }>>(load, version)
  if (read.error && !read.data) return <div className="aiws-error" role="alert">{read.error}</div>
  if (!read.data) return <div className="aiws-muted">{t('aiworkspace.blocks.loading')}</div>
  const payload = read.data.content.payload ?? {}
  const surfaceOf = (id: string) => store.outline.ancestors(id).map((a) => store.outline.get(a)).find((e) => e?.kind === 'surface')
  const firstTarget = entity.type_id === 'buckyos.show-path' ? (payload.steps ?? [])[0]?.target : null
  const surface = entity.type_id === 'buckyos.viewport'
    ? store.outline.get(payload.surface_ref?.entity_id ?? '')
    : firstTarget ? surfaceOf(firstTarget.entity_id) : undefined
  const pathId = entity.type_id === 'buckyos.show-path' ? entity.entity_id : store.outline.childrenOf('shows').find((p) => p.type_id === 'buckyos.show-path')?.entity_id ?? null
  const openInEditor = () => {
    const target = surface ?? shell.activeSurface
    if (target) store.userState.set('surface:active', target.entity_id)
    if (pathId) store.userState.set('ui:show-path', pathId)
    store.userState.set('canvas:mode', 'presentation_edit')
    shell.setTopMode('canvas')
  }
  const lines: ReactNode[] = entity.type_id === 'buckyos.show-path'
    ? [
        <div key="purpose">{t(payload.purpose === 'guide' ? 'aiworkspace.datamode.path.guide' : 'aiworkspace.datamode.path.presentation')}</div>,
        <div key="steps">{t('aiworkspace.datamode.path.steps', { count: (payload.steps ?? []).length })}</div>,
        ...(payload.stage ? [<div key="stage" className="aiws-muted">{t('aiworkspace.datamode.path.stage', { w: payload.stage.w, h: payload.stage.h })}</div>] : []),
      ]
    : [
        <div key="canvas">{t('aiworkspace.datamode.viewport.canvas', { name: surface ? entityLabel(surface) : payload.surface_ref?.entity_id ?? '' })}</div>,
        <div key="center" className="aiws-muted">{t('aiworkspace.datamode.viewport.center', { x: Math.round(payload.center?.x ?? 0), y: Math.round(payload.center?.y ?? 0), zoom: Math.round((payload.zoom ?? 1) * 100) })}</div>,
      ]
  return (
    <div className="aiws-dm-path" data-testid="aiws-dm-path-summary">
      {lines}
      {(payload.notes || payload.caption) && <div className="aiws-muted">{payload.caption ?? payload.notes}</div>}
      <button type="button" data-testid="aiws-dm-open-path-editor" disabled={shell.phone} title={shell.phone ? t('aiworkspace.datamode.path.notOnPhone') : undefined} onClick={openInEditor}>
        <ExternalLink size={16} aria-hidden="true" /> {t('aiworkspace.datamode.path.openEditor')}
      </button>
      <RawDetail entity={entity} folded />
    </div>
  )
}

/** A canvas or a Block (shown with "显示全部对象"): where it is, and "在画布上定位". */
function BlockSummary({ entity }: { entity: EntityEnvelope }) {
  const { t } = useAiwsText()
  const ui = useWorkspaceUi()
  return (
    <div className="aiws-dm-path" data-testid="aiws-dm-block-summary">
      <button type="button" data-testid="aiws-dm-locate-here" onClick={() => ui.openEntity(entity.entity_id)}>{t('aiworkspace.datamode.locateHere')}</button>
      <RawDetail entity={entity} folded />
    </div>
  )
}

export function RawDetail({ entity, folded = false }: { entity: EntityEnvelope; folded?: boolean }) {
  const { t } = useAiwsText()
  const store = useStore()
  const version = useVersion(`e:${entity.entity_id}`)
  const load = useCallback(() => store.session.read<unknown>(entity.entity_id), [store, entity.entity_id])
  const read = useLoad<ReadOk<unknown>>(load, version)
  const session = SESSION_TYPES.includes(entity.type_id)
  return (
    <details open={!folded} data-testid="aiws-raw-detail">
      <summary>{entity.degraded ? t('aiworkspace.data.detail.raw', { reason: entity.degraded }) : t('aiworkspace.data.detail.rawContent')}</summary>
      {!entity.degraded && !folded && <p className="aiws-muted">{session ? t('aiworkspace.datamode.sessionReadOnly', { type: typeName(entity) }) : t('aiworkspace.data.detail.noEditor', { type: typeName(entity) })}</p>}
      {read.error && <div className="aiws-error" role="alert">{read.error}</div>}
      {read.data ? <pre className="aiws-raw">{JSON.stringify(read.data.content, null, 2)}</pre> : !read.error && <div className="aiws-muted">{t('aiworkspace.blocks.loading')}</div>}
    </details>
  )
}
