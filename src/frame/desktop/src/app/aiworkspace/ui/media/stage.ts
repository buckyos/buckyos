/* A file becoming an asset (多媒体对象 §3): its facts, the file and its poster uploaded side by side, and the size its
 * Block takes — a picture's or poster's own proportions, like an inserted image; a card for the rest. */

import type { AssetPayload } from '../../api/types'
import type { WorkspaceStore } from '../../state/store'
import { describeMediaFile, type MediaFacts } from './poster'

/** The keys an AssetRef takes from a staged file (`media_type` and `size` are the backend's). */
export type StagedPayload = Omit<AssetPayload, 'media_type' | 'size'> & { file_name: string }

/** At most this wide or high when inserted (UI improvement §7.2). */
const INSERT_EDGE = 480

export async function stageMediaFile(store: WorkspaceStore, file: File, known?: MediaFacts): Promise<{ facts: MediaFacts; payload: StagedPayload }> {
  const facts = known ?? await describeMediaFile(file)
  const posterName = `${file.name.replace(/\.[^.]*$/, '')}.poster.jpg`
  const [uploaded, poster] = await Promise.all([
    store.session.uploadAsset(file, file.name),
    // a poster that cannot be stored leaves the icon: never a reason to fail the file
    facts.poster ? store.session.uploadAsset(facts.poster, posterName).catch(() => null) : Promise.resolve(null),
  ])
  const payload: StagedPayload = { object_id: uploaded.object_id, file_name: file.name }
  if (facts.image) payload.image = facts.image
  if (poster && facts.kind !== 'image') payload.poster = poster.object_id
  if (facts.durationMs) payload.duration_ms = facts.durationMs
  if (facts.pageCount) payload.page_count = facts.pageCount
  return { facts, payload }
}

export function mediaBlockSize(facts: MediaFacts): { w: number; h: number } {
  const picture = facts.image && (facts.kind === 'image' || facts.poster) ? facts.image : null
  if (picture) {
    const k = Math.min(1, INSERT_EDGE / Math.max(picture.width, picture.height, 1))
    return { w: Math.max(24, Math.round(picture.width * k)), h: Math.max(24, Math.round(picture.height * k)) }
  }
  if (facts.kind === 'video') return { w: 320, h: 180 }
  return { w: 300, h: 88 }
}
