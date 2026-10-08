/* Starting a show (第三期规划 §7.2, §8): ask the service for the show — with workspace-level write access the workspace
 * is locked for writes, and a path whose Surfaces have operable Blocks gets a temporary clone the stage reads and
 * writes. Offline, only a local read-only show in this window is possible (§8.4). A reloaded stage resumes its show
 * while the show's lease is still running (§11.3). */

import type { AiwsClient } from '../api/client'
import { unwrap } from '../api/client'
import { OnlineWorkspaceSession } from '../api/session'
import type { KeyedContent, ShowPathPayload } from '../api/types'
import { WorkspaceStore } from '../state/store'
import { hasLiveBlocks, playable, resolveSteps, surfacesOfSteps } from './model'
import { clearActiveShow, loadActiveShow } from './relay'
import type { ShowSession } from './StageView'

export const WRITE_CAPS = ['update', 'append', 'delete', 'structure']

/** The store a show with a clone reads and writes: its own session on the clone, the core shared. */
async function cloneStore(client: AiwsClient, store: WorkspaceStore, cloneId: string): Promise<WorkspaceStore> {
  const session = await OnlineWorkspaceSession.open(client, cloneId)
  const clone = new WorkspaceStore(session, store.core)
  await clone.outline.reload()
  return clone
}

/** Ask for the show and build what the stage runs on. */
export async function openShow(client: AiwsClient, store: WorkspaceStore, pathId: string, stepId: string | null): Promise<ShowSession> {
  const workspaceId = store.session.workspaceId
  const read = await store.session.read<KeyedContent<ShowPathPayload>>(pathId)
  const steps = playable(resolveSteps(store.outline, read.content.payload))
  if (steps.length === 0) throw new Error('这条路径没有可以放映的步骤（步骤都已停用，或目标已删除）')
  if (store.session.status().kind !== 'live') {
    // offline: one window, read-only, no lock, no prompter, no clone (§8.4)
    return { workspaceId, pathId, startStepId: stepId, start: null, store }
  }
  const writer = store.session.info().capabilities.some((c) => WRITE_CAPS.includes(c))
  const live = writer && hasLiveBlocks(store.outline, surfacesOfSteps(steps))
  const start = unwrap(await client.showStart({ workspace_id: workspaceId }, pathId, live))
  if (!start.clone_workspace_id) return { workspaceId, pathId, startStepId: stepId, start, store }
  try {
    return { workspaceId, pathId, startStepId: stepId, start, store: await cloneStore(client, store, start.clone_workspace_id) }
  } catch (error) {
    await client.showEnd({ workspace_id: workspaceId }, start.show_id).catch(() => undefined)
    throw error
  }
}

/** A show this tab was running before a reload, if it is still alive. */
export async function resumeShow(client: AiwsClient, store: WorkspaceStore): Promise<ShowSession | null> {
  const workspaceId = store.session.workspaceId
  const saved = loadActiveShow(workspaceId)
  if (!saved) return null
  try {
    // the relay's own position wins over what this tab saved: a later state, commands already handled before the reload
    const now = unwrap(await client.showWatch({ workspace_id: workspaceId }, saved.start.show_id, {}))
    const step = 'step_id' in now.state ? now.state.step_id ?? saved.stepId : saved.stepId
    const showStore = saved.start.clone_workspace_id ? await cloneStore(client, store, saved.start.clone_workspace_id) : store
    return { workspaceId, pathId: saved.pathId, startStepId: step, start: saved.start, store: showStore, resume: { seq: Math.max(saved.seq, now.seq), cursor: Math.max(saved.cursor, now.cursor) } }
  } catch {
    clearActiveShow(workspaceId)
    return null
  }
}
