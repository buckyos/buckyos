import { z } from 'zod'
import { isTransferableRef, readSelectDialogResult, type ContentSelector, type SelectDialogParams, type SelectDialogResult, type TransferableContentRef } from 'buckyos/content'
import { entryNameSchema } from '../../app/filebrowser/data/schemas'
import { normalizeCyfsPath } from '../../components/preview/session'

const strings = z.union([z.string().max(255), z.array(z.string().max(255)).max(64)])
const selectorSchema = z.object({
  objType: strings.optional(),
  mime: strings.optional(),
  schema: strings.optional(),
  ext: z.array(z.string().max(32)).max(64).optional(),
  maxSize: z.number().int().min(0).optional(),
}).strict()
const startSchema = z.custom<TransferableContentRef>((value) => isTransferableRef(value) && value.kind === 'cyfs-path' && value.path.length <= 4096)

/** `sysdlg/select` parameters as a caller sends them. Callers may be untrusted Apps: every bound is enforced here. */
export const selectDialogParamsSchema = z.object({
  mode: z.enum(['open', 'folder', 'save']).optional(),
  title: z.string().max(80).optional(),
  confirmLabel: z.string().max(40).optional(),
  start: startSchema.optional(),
  multiple: z.boolean().optional(),
  limit: z.number().int().min(1).max(500).optional(),
  accept: z.array(selectorSchema).max(16).optional(),
  writable: z.boolean().optional(),
  exclude: z.array(startSchema).max(1000).optional(),
  suggestedName: z.string().max(1024).optional(),
  allowLocal: z.boolean().optional(),
}).strict()

/** Validated parameters with defaults applied and the fields of other modes dropped. */
export interface SelectDialogInput {
  mode: 'open' | 'folder' | 'save'
  title?: string
  confirmLabel?: string
  start?: Extract<TransferableContentRef, { kind: 'cyfs-path' }>
  multiple: boolean
  limit: number
  accept: ContentSelector[]
  writable: boolean
  /** DFS paths that cannot be the target, nor anything below them. */
  exclude: string[]
  suggestedName: string
  allowLocal: boolean
}

export function parseSelectDialogParams(params: unknown): SelectDialogInput {
  const p: SelectDialogParams = selectDialogParamsSchema.parse(params ?? {})
  const mode = p.mode ?? 'open'
  const multiple = mode === 'open' && p.multiple === true
  const suggested = mode === 'save' ? entryNameSchema.safeParse(p.suggestedName ?? '') : undefined
  const writable = mode === 'save' || (mode === 'folder' && p.writable === true)
  return {
    mode,
    title: p.title?.trim() || undefined,
    confirmLabel: p.confirmLabel?.trim() || undefined,
    start: p.start as SelectDialogInput['start'],
    multiple,
    limit: multiple ? p.limit ?? 100 : 1,
    accept: mode === 'open' ? p.accept ?? [] : [],
    writable,
    exclude: mode === 'open' ? [] : (p.exclude ?? []).map((ref) => normalizeCyfsPath((ref as { path: string }).path).slice('cyfs://'.length)),
    suggestedName: suggested?.success ? suggested.data : '',
    allowLocal: p.allowLocal === true && mode !== 'save' && !writable,
  }
}

/** A result is only handed to a caller when it has the contract's shape. */
export function checkSelectDialogResult(result: SelectDialogResult, input: Pick<SelectDialogInput, 'allowLocal'>): SelectDialogResult {
  const checked = readSelectDialogResult(result, { allowLocal: input.allowLocal })
  if (!checked) throw new Error('sysdlg/select produced an invalid result')
  return checked
}
