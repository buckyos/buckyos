/* "Create sample": replays the shared fixture of design §3.8 through the public interface, exactly as
 * the backend tests do. The JSON is a copy of src/frame/aiworkspace/fixtures/project-workspace/commits.json
 * (the e2e suite asserts the two files are identical). The template then adds one commit of its own: the "task summary"
 * wish block on the flow page (许愿格交互升级 §3.10 D, D10), there to show how a wish works on a flow page and a phone —
 * the card in the column, no lines, its input and results reached from its records. The fixture stays the backend's. */

import fixture from '../fixtures/project-workspace.commits.json'
import { unwrap, type AiwsClient } from './client'
import { TemplateFailure } from './demos'
import { base64ToBytes } from './ids'
import { uploadWorkspaceAsset } from './session'
import { aiwsT, aiwsText } from '../i18n'
import { PROTOCOL_VERSION, type Operation, type WorkspaceSummary } from './types'

interface Fixture {
  assets: { placeholder: string; file_name: string; base64: string }[]
  commits: { message: string; operations: Operation[] }[]
}

const SURFACE = 'surface-main'
const CONTENT = 'surface-main-content'
const TASKS = 'tasks'
const SAMPLE_WISH_ID = 'wish-task-summary'
const SAMPLE_WISH_CELL = 'cell-task-summary'

/** The sample's wish (text in the language at creation time): it reads the task table and writes beside it on the page,
 * placed after the two task tables (`g`) and before the project record (`h`). */
export function sampleWishCommit(commits: Fixture['commits']): { message: string; operations: Operation[] } {
  const { t } = aiwsText()
  const table = commits.flatMap((c) => c.operations).find((op) => op.op === 'entity.create' && op.entity_id === TASKS)
  const tasks = typeof table?.name === 'string' ? table.name : TASKS
  const title = t('aiworkspace.samplewish.wish.title')
  return {
    message: t('aiworkspace.samplewish.commit'),
    operations: [
      { op: 'entity.create', entity_id: SAMPLE_WISH_ID, type_id: 'buckyos.wish', parent_id: CONTENT, order_key: 'a', name: title, payload: {
        title, prompt: t('aiworkspace.samplewish.wish.prompt', { tasks }), executor: 'xllm', output_mode: 'overwrite',
        inputs: [{ entity_id: TASKS, version: { mode: 'follow' } }],
        output: { container_id: CONTENT, surface_id: SURFACE, name: t('aiworkspace.samplewish.wish.output') },
      } },
      { op: 'entity.create', entity_id: SAMPLE_WISH_CELL, type_id: 'buckyos.cell', parent_id: SURFACE, order_key: 'gm', placement: { x: 0, y: 0, w: 420, h: 300 },
        payload: { view: { type: 'wish' }, source_ref: { entity_id: SAMPLE_WISH_ID }, title: t('aiworkspace.samplewish.block.wish') } },
    ],
  }
}

export async function createSampleWorkspace(client: AiwsClient, title: string): Promise<WorkspaceSummary> {
  const workspace = unwrap(await client.wsCreate(title))
  try {
    await fillSample(client, workspace)
  } catch (error) {
    throw new TemplateFailure(workspace, aiwsT('aiworkspace.core.sample.fillFailed'), error)
  }
  return workspace
}

async function fillSample(client: AiwsClient, workspace: WorkspaceSummary) {
  const data = fixture as unknown as Fixture
  const ws = { workspace_id: workspace.workspace_id }
  let text = JSON.stringify(data.commits)
  for (const asset of data.assets) {
    // assets first: a commit may only reference content the backend has already verified (design §3.6)
    const bytes = base64ToBytes(asset.base64)
    const done = await uploadWorkspaceAsset(client, ws.workspace_id, new File([bytes as BlobPart], asset.file_name))
    text = text.split(asset.placeholder).join(done.object_id)
  }
  const commits = JSON.parse(text) as Fixture['commits']
  for (const [index, commit] of [...commits, sampleWishCommit(commits)].entries()) {
    const result = await client.commit({
      protocol_version: PROTOCOL_VERSION, workspace_id: workspace.workspace_id, epoch: workspace.epoch,
      idempotency_key: `fixture/${index + 1}`, session_id: 'fixture', origin: 'human', message: commit.message, operations: commit.operations,
    })
    if (result.status !== 'accepted') throw new Error(aiwsT('aiworkspace.core.sample.commitRejected', { index: index + 1, code: result.code }))
  }
}
