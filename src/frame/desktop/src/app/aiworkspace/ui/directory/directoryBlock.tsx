/* eslint-disable react-refresh/only-export-components -- a Block definition and its renderers, not a component module */
/* The directory Block (框架 PRD §7.4 "目录 Block"): shows a `buckyos.dir` on a free canvas or a flow page; its editor is
 * the directory's standard editor (DirectoryEditor, `embedded`). On a free canvas the Block shows a compact listing
 * until "edit content" (or the toolbar's "add"); in view mode it can be browsed and previewed; on a flow page the editor
 * is live (a phone edits it there). Offered in the "insert…" catalog under data, not as a standing toolbar button. */

import { FolderOpen, Plus } from 'lucide-react'
import { aiwsT } from '../../i18n'
import { useAiwsText } from '../../useAiwsText'
import { sourceLabel } from '../blocks/affordances'
import { requestIntent } from '../blocks/editorToolbar'
import { cellOp, createOp } from '../blocks/ops'
import { blockRegistry, type BlockDefinition, type RenderContext } from '../blocks/registry'
import { freeTableName } from '../controls/data/source'
import { DirectoryEditor } from './DirectoryEditor'

function DirectoryStatic(context: RenderContext) {
  if (!context.source) return null
  return <div className="aiws-block-static aiws-block-directory"><DirectoryEditor entityId={context.source.entity_id} size="embedded" variant="static" cellId={context.cell.entity_id} /></div>
}

function DirectoryView(context: RenderContext) {
  if (!context.source) return null
  return <div className="aiws-block-static aiws-block-directory"><DirectoryEditor entityId={context.source.entity_id} size="embedded" variant="browse" cellId={context.cell.entity_id} /></div>
}

function DirectoryBlockEditor(context: RenderContext) {
  if (!context.source) return null
  return (
    <div className="aiws-block-static aiws-block-directory">
      <DirectoryEditor entityId={context.source.entity_id} size="embedded" variant="edit" readOnlyReason={context.readOnlyReason} cellId={context.cell.entity_id} />
    </div>
  )
}

function DirectorySimplified(context: RenderContext) {
  const { t } = useAiwsText()
  return <div className="aiws-block-simplified">{t('aiworkspace.directory.simplified', { name: context.source?.title ?? context.source?.name ?? context.payload.title ?? '' })}</div>
}

const canAdd = (context: RenderContext) => context.mode === 'edit' && context.readOnlyReason === null && Boolean(context.source?.capabilities.includes('update')) && !context.source?.source

export const directoryDefinition: BlockDefinition = {
  type: 'directory', version: 1, get title() { return aiwsT('aiworkspace.directory.blockTitle') }, accepts: ['buckyos.dir'], allowNoSource: false,
  defaultSize: { w: 480, h: 360 }, cost: { editor: true, html: false },
  Static: DirectoryStatic, View: DirectoryView, Editor: DirectoryBlockEditor, Simplified: DirectorySimplified,
  catalog: { group: 'data', get description() { return aiwsT('aiworkspace.directory.blockDescription') }, needs: 'none' },
  actions: [{ id: 'edit', get label() { return aiwsT('aiworkspace.blocks.action.editContent') }, modes: ['edit'], needs: ['update'], onSource: true, run: (context) => context.activateEditor(), key: 'Enter' }],
  hover: (context) => sourceLabel(context, FolderOpen),
  // while editing, the editor's own head has the tools
  toolbar: (context) => (context.editorActive || !canAdd(context) ? [] : [
    { kind: 'button', id: 'directory-add', icon: Plus, label: aiwsT('aiworkspace.directory.add.title'), run: () => { requestIntent(context.cell.entity_id, 'add'); context.activateEditor() } },
  ]),
  create: (args) => {
    if (args.existingSourceId) return [cellOp(args, 'directory', args.existingSourceId)]
    const name = freeTableName(args.store, args.contentFolderId, args.title || aiwsT('aiworkspace.directory.defaultName'))
    return [createOp(args.dataId, 'buckyos.dir', args.contentFolderId, args.dataOrderKey, { members: {} }, name), cellOp(args, 'directory', args.dataId)]
  },
}

let registered = false
/** Register the directory Block once (idempotent), like the other built-in definitions. */
export function registerDirectoryBlock() {
  if (registered) return
  registered = true
  blockRegistry.register(directoryDefinition)
}
