/* eslint-disable react-refresh/only-export-components -- Block definitions bundle their renderers */
/* The `html` Block (phase two §10.4): a Block whose definition entity carries HTML / CSS / JS. In
 * edit and view mode it shows its last static snapshot (or a placeholder) and runs only after
 * explicit activation; while mounted, the extension talks to the host through `window.aiws`.
 * Crashes and silence fall back to the snapshot; other Blocks are unaffected. */

import { useEffect, useRef, useState } from 'react'
import type { Json } from '../../api/types'
import { aiwsT } from '../../i18n'
import { useStore } from '../../state/hooks'
import { useAiwsText } from '../../useAiwsText'
import { AssetBlobImage } from './AssetBlobImage'
import { blockRegistry, type BlockDefinition, type RenderContext } from '../blocks/registry'
import { cellOp } from '../blocks/ops'
import { makeBridge } from './bridge'
import { HtmlRuntime } from './htmlRuntime'

function Snapshot(context: RenderContext & { label?: string }) {
  const { t } = useAiwsText()
  const snapshot = context.payload.config?.snapshot
  return (
    <div className="aiws-html-static" data-testid={`aiws-html-static-${context.cell.entity_id}`}>
      {snapshot?.object_id
        ? <AssetBlobImage objectId={snapshot.object_id} mediaType={snapshot.media_type} alt={context.payload.title ?? t('aiworkspace.extension.snapshotAlt')} />
        : <div className="aiws-html-placeholder">{context.label ?? t('aiworkspace.extension.html.title')}<div className="aiws-muted">{t('aiworkspace.extension.html.noSnapshot')}</div></div>}
    </div>
  )
}

function HtmlStatic(context: RenderContext) {
  const { t } = useAiwsText()
  return <Snapshot {...context} label={context.documentDefinition?.title ?? context.payload.title ?? t('aiworkspace.extension.html.title')} />
}

/** Mounted after activation: the real runtime. */
function HtmlActive(context: RenderContext) {
  const store = useStore()
  const { t } = useAiwsText()
  const hostRef = useRef<HTMLDivElement>(null)
  const [failure, setFailure] = useState<string | null>(null)
  const [attempt, setAttempt] = useState(0)
  const source = context.documentDefinition
  const html = source?.kind === 'html' ? source.html : undefined
  const htmlKey = html ? JSON.stringify(html) : ''
  const cellId = context.cell.entity_id
  const mode = context.mode
  const deactivate = context.deactivateEditor
  useEffect(() => {
    const host = hostRef.current
    if (!host || !htmlKey) return
    const src = JSON.parse(htmlKey) as { html: string; css?: string; js?: string }
    const cell = store.outline.get(cellId)
    if (!cell) return
    const runtime = new HtmlRuntime(src, makeBridge(store, { cell, payload: context.payload, keyRevs: context.keyRevs, source: context.source, mode }))
    let cancelled = false
    runtime.onCrash = (message) => { if (!cancelled) { runtime.dispose(); setFailure(message) } }
    runtime.mount(host).catch((error: unknown) => { if (!cancelled) setFailure(error instanceof Error ? error.message : String(error)) })
    return () => { cancelled = true; runtime.dispose() }
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [store, htmlKey, cellId, mode, attempt])
  if (!html) return <div className="aiws-warning">{t('aiworkspace.extension.html.unavailable')}</div>
  if (failure) {
    return (
      <div className="aiws-block-fallback" data-testid={`aiws-html-failed-${context.cell.entity_id}`} data-reason="html_failed">
        <div className="aiws-error" role="alert">{t('aiworkspace.extension.html.failed', { error: failure })}</div>
        <Snapshot {...context} />
        <button type="button" className="aiws-link" onClick={() => { setFailure(null); setAttempt((n) => n + 1) }}>{t('aiworkspace.extension.html.runAgain')}</button>
        <button type="button" className="aiws-link" onClick={deactivate}>{t('aiworkspace.extension.html.backToStatic')}</button>
      </div>
    )
  }
  return (
    <div className="aiws-html-active" data-testid={`aiws-html-active-${context.cell.entity_id}`}>
      <div ref={hostRef} className="aiws-html-host" />
      <div className="aiws-html-bar"><span className="aiws-muted">{t('aiworkspace.extension.html.mounted', { name: source?.title ?? context.payload.def_ref?.entity_id ?? '' })}</span><button type="button" className="aiws-link" onClick={deactivate}>{t('aiworkspace.extension.html.stop')}</button></div>
    </div>
  )
}

export const htmlBlock: BlockDefinition = {
  type: 'html', version: 1, definitionKind: 'html', get title() { return aiwsT('aiworkspace.extension.html.title') }, accepts: ['buckyos.table-source', 'buckyos.richtext', 'buckyos.record', 'buckyos.asset-ref', 'buckyos.wish', 'buckyos.annotation'], allowNoSource: true,
  defaultSize: { w: 480, h: 320 }, cost: { editor: true, html: true },
  Static: HtmlStatic, Editor: HtmlActive,
  catalog: { group: 'extension', get description() { return aiwsT('aiworkspace.extension.html.description') }, needs: 'definition' },
  actions: [
    { id: 'run', get label() { return aiwsT('aiworkspace.extension.html.run') }, modes: ['edit'], run: (context) => context.activateEditor(), key: 'Enter' },
    { id: 'open-def', get label() { return aiwsT('aiworkspace.extension.html.openDefinition') }, modes: ['edit', 'view'], when: (context) => Boolean(context.payload.def_ref), run: (context) => { if (context.payload.def_ref) context.openEntity(context.payload.def_ref.entity_id) } },
  ],
  create: (args) => {
    // the definition reference is the Cell's `def_ref`, never part of its configuration
    const { def_id: defId, ...config } = args.config ?? {}
    return [cellOp({ ...args, config: Object.keys(config).length > 0 ? config : undefined }, 'html', args.existingSourceId, { def_ref: { entity_id: String(defId ?? '') } as unknown as Json })]
  },
}

export function registerHtmlBlock() {
  blockRegistry.register(htmlBlock)
}
