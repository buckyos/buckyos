/* "Create sample": replays the shared fixture of design §3.8 through the public interface, exactly as
 * the backend tests do. The JSON is a copy of src/frame/aiworkspace/fixtures/project-workspace/commits.json
 * (the e2e suite asserts the two files are identical). */

import fixture from '../fixtures/project-workspace.commits.json'
import { unwrap, type AiwsClient } from './client'
import { base64ToBytes } from './ids'
import { PROTOCOL_VERSION, type Operation, type WorkspaceSummary } from './types'

interface Fixture {
  assets: { placeholder: string; file_name: string; base64: string }[]
  commits: { message: string; operations: Operation[] }[]
}

export async function createSampleWorkspace(client: AiwsClient, title: string): Promise<WorkspaceSummary> {
  const data = fixture as unknown as Fixture
  const workspace = unwrap(await client.wsCreate(title))
  const ws = { workspace_id: workspace.workspace_id }
  let text = JSON.stringify(data.commits)
  for (const asset of data.assets) {
    // assets first: a commit may only reference content the backend has already verified (design §3.6)
    const bytes = base64ToBytes(asset.base64)
    const begin = unwrap(await client.assetBeginUpload(ws, bytes.length, asset.file_name))
    await client.transport.upload(begin.upload_id, bytes)
    const done = unwrap(await client.assetFinishUpload(ws, begin.upload_id))
    text = text.split(asset.placeholder).join(done.object_id)
  }
  const commits = JSON.parse(text) as Fixture['commits']
  for (const [index, commit] of commits.entries()) {
    const result = await client.commit({
      protocol_version: PROTOCOL_VERSION, workspace_id: workspace.workspace_id, epoch: workspace.epoch,
      idempotency_key: `fixture/${index + 1}`, session_id: 'fixture', origin: 'human', message: commit.message, operations: commit.operations,
    })
    if (result.status !== 'accepted') throw new Error(`样例的第 ${index + 1} 个提交未被接受：${result.code}`)
  }
  return workspace
}
