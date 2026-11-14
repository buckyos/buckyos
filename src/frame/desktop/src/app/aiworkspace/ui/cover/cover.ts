/* The Workspace's cover (UI 框架 PRD §9.2, Q37): one picture its managers choose, kept in the Workspace's settings
 * (`config.cover` on the root: `{ object_id, media_type, size }`, checked by the service). Nothing is taken from the
 * canvases: without one the cards draw their placeholder. The picture is an ordinary uploaded asset of the Workspace;
 * whoever reads the settings reads it (the asset route admits it through the root), and it travels with the settings
 * (templates, Clones, packages). Writing it is one ordinary settings Commit, `manage` on `/`.
 *
 * Chosen in the system file picker (the Zone first; this device is the picker's own secondary entry), checked here
 * the way the service will (type, size) so the person learns at once. "工作区 ▸ 设置封面…" asks the shell's host to
 * open the dialog (`requestCoverDialog`); the home's card menu opens it itself and writes over kRPC. */

import { unwrap, type AiwsClient } from '../../api/client'
import { randomId } from '../../api/ids'
import { uploadWorkspaceAsset } from '../../api/session'
import { PROTOCOL_VERSION, type CommitOutcome, type WorkspaceListItem } from '../../api/types'
import { aiwsT } from '../../i18n'
import { setConfigOp } from '../../state/settings'
import { formatBytes } from '../directory/format'

/** What the asset route serves as pictures (an SVG it serves as bytes: never a cover). */
export const COVER_MEDIA_TYPES = ['image/png', 'image/jpeg', 'image/gif', 'image/webp', 'image/avif', 'image/bmp']
/** The home page downloads each cover whole. */
export const COVER_MAX_BYTES = 8 * 1024 * 1024
const COVER_EXTENSIONS: Record<string, string> = { png: 'image/png', jpg: 'image/jpeg', jpeg: 'image/jpeg', gif: 'image/gif', webp: 'image/webp', avif: 'image/avif', bmp: 'image/bmp' }
/** The picker's filter (`<input accept>`): the types, and the extensions for files the Zone names no type of. */
export const COVER_ACCEPT = [...COVER_MEDIA_TYPES, ...Object.keys(COVER_EXTENSIONS).map((ext) => `.${ext}`)].join(',')

/** The picture's media type as its name or the browser says it. */
function mediaTypeOf(file: File): string {
  if (file.type) return file.type.toLowerCase()
  const ext = /\.([a-z0-9]+)$/i.exec(file.name)?.[1]?.toLowerCase() ?? ''
  return COVER_EXTENSIONS[ext] ?? ''
}

/** Why `file` cannot be a cover, or null (the service sniffs the bytes again). */
export function coverProblem(file: File): string | null {
  if (!COVER_MEDIA_TYPES.includes(mediaTypeOf(file))) return aiwsT('aiworkspace.cover.notPicture', { name: file.name })
  if (file.size > COVER_MAX_BYTES) return aiwsT('aiworkspace.cover.tooLarge', { name: file.name, size: formatBytes(file.size) })
  return null
}

/** The service did not take the cover: the message says why, as the person reads it. */
export class CoverRefused extends Error {}

/** Why a settings Commit did not go through, or null when it did. */
export function outcomeProblem(outcome: CommitOutcome): string | null {
  if (outcome.status === 'accepted' || outcome.status === 'saved_locally') return null
  const detail = 'errors' in outcome && Array.isArray(outcome.errors) && outcome.errors[0]?.detail ? outcome.errors[0].detail
    : 'detail' in outcome && typeof outcome.detail === 'string' && outcome.detail ? outcome.detail
      : 'code' in outcome ? String(outcome.code) : outcome.status
  return aiwsT('aiworkspace.cover.refused', { reason: detail })
}

// ---- "工作区 ▸ 设置封面…": the menu entry asks the shell's host to open the dialog

const listeners = new Set<() => void>()

export function onCoverDialog(listener: () => void): () => void {
  listeners.add(listener)
  return () => { listeners.delete(listener) }
}

export function requestCoverDialog() {
  for (const listener of [...listeners]) listener()
}

// ---- the home page: no Workspace is open there, the card writes over kRPC

/** Upload `file` into the Workspace and make it the cover (`null`: remove it) in one settings Commit, expected at the
 * version just read. */
export async function writeCoverFromHome(client: AiwsClient, workspace: WorkspaceListItem, file: File | null): Promise<void> {
  const ws = { workspace_id: workspace.workspace_id }
  const cover = file ? { object_id: (await uploadWorkspaceAsset(client, workspace.workspace_id, file)).object_id } : null
  const read = await client.read<{ payload: unknown; key_revs?: Record<string, number> }>(ws, 'root').then((r) => unwrap(r)).catch(() => null)
  const outcome = await client.commit({
    protocol_version: PROTOCOL_VERSION, workspace_id: workspace.workspace_id, epoch: workspace.epoch,
    idempotency_key: randomId('home-cover-'), session_id: randomId('home-'), origin: 'human',
    message: aiwsT(file ? 'aiworkspace.cover.setLabel' : 'aiworkspace.cover.removeLabel'),
    operations: [setConfigOp('root', { cover }, read?.content.key_revs ?? null)],
  })
  const problem = outcomeProblem({ ...outcome, idempotency_key: '' })
  if (problem) throw new CoverRefused(problem)
}
