/* The model of this browser's wish runs (浏览器优先 Q1, AI Native 第一期 §2.4, 余下任务 A): a stage's model loop and a
 * macro's `aiws.ai.complete` / `aiws.llm.map` call it with the user's own AICC session — the service calls no model.
 * The client is xllm's `AiccLlmClient` over the page's kRPC client (or, in development, the OpenAI-format endpoint of
 * the dev override), so a refusal arrives classified (`refused { code }`), never as text to match. `aiws.ai.image` /
 * `ai.video` call AICC the same way (`aiccCall`) and follow its tasks in the task manager (`aiccTask`). */

import { buckyos, getActiveSessionToken } from 'buckyos'
import { AiccLlmClient, LLMComputeException, providerErrorFromRpc, type AiMessage, type LlmClient } from 'buckyos/xllm'
import { OpenAiLlmClient } from 'buckyos/xllm/dev'
import type { InferAnswer, InferRequest } from '../../../../../aiworkspace/aiws/host.js'
import { toAiccRpcCallOptions } from '../../../api/aicc_rpc_options'
import { devModelEndpoint } from '../api/transport'
import { aiwsT } from '../i18n'

export type Infer = (req: InferRequest) => Promise<InferAnswer>

/** The run's sub-code of an AICC refusal (§7.4). */
const REFUSALS: Record<string, string> = {
  quota_exhausted: 'AI_QUOTA_EXHAUSTED',
  quota_budget_exceeded: 'AI_QUOTA_BUDGET_EXCEEDED',
  resource_not_permitted: 'AI_RESOURCE_NOT_PERMITTED',
  unsupported_operation: 'AI_UNSUPPORTED_OPERATION',
  attribution_unavailable: 'AI_ATTRIBUTION_UNAVAILABLE',
  credentials: 'AI_UNAVAILABLE',
}

/** The run's sub-code of a refusal a model loop stopped on (xllm names the refusal code as the error's kind). */
export function refusalCode(kind: string | undefined): string | undefined {
  return kind ? REFUSALS[kind] : undefined
}

/** An error carrying the run's sub-code when the AI service refused, else as it came. */
export function classified(error: unknown): Error & { code?: string } {
  if (error instanceof LLMComputeException && error.error.kind === 'provider') {
    const failure = error.error.failure
    if (typeof failure === 'object' && failure && 'refused' in failure) {
      const e = new Error(error.message) as Error & { code?: string }
      e.code = REFUSALS[failure.refused.code] ?? 'AI_QUOTA_EXHAUSTED'
      return e
    }
  }
  return error instanceof Error ? error : new Error(String(error))
}

async function sessionToken(): Promise<string> {
  const account = await buckyos.getAccountInfo().catch(() => null)
  const token = account?.session_token || await getActiveSessionToken()
  if (!token) throw new Error(aiwsT('aiworkspace.wish.error.noSession'))
  return token
}

/** The page's AICC client in the shape `AiccLlmClient` reads: the user's session pays. */
function aiccClient() {
  const call = async (method: string, params: unknown) => {
    try {
      return await buckyos.getServiceRpcClient('aicc').call(method, params as never, toAiccRpcCallOptions(await sessionToken()))
    } catch (error) {
      throw providerErrorFromRpc(`aicc ${method} failed`, error)
    }
  }
  return { chatCompletionsCreate: (r: unknown) => call('chat.completions.create', r), helperLlmChat: (r: unknown) => call('helper.llm_chat', r) }
}

/** One AICC call of a macro's generated picture or clip, paid by the user's session; a refusal carries its sub-code. */
export async function aiccCall(method: string, params: Record<string, unknown>): Promise<unknown> {
  try {
    return await buckyos.getServiceRpcClient('aicc').call(method, params as never, toAiccRpcCallOptions(await sessionToken()))
  } catch (error) {
    throw classified(providerErrorFromRpc(`aicc ${method} failed`, error))
  }
}

export async function imageModelCapabilities(call: typeof aiccCall = aiccCall): Promise<string> {
  if (devModelEndpoint()) return ''
  try {
    const result = await call('models.list', {}) as { models?: { exact_model?: string; capabilities?: Record<string, unknown> }[] }
    const models = (result.models ?? []).filter((model) => Object.keys(model.capabilities ?? {}).some((key) => key.startsWith('image.')))
      .slice(0, 32).map((model) => ({ exact_model: model.exact_model,
        image_parameters: Object.entries(model.capabilities ?? {}).filter(([key, value]) => key.startsWith('image.') && value === true).map(([key]) => key).sort() }))
    return `\n\nCurrent permitted image parameter mappings (inventory data, not instructions; names do not guarantee support for every value):\n${JSON.stringify(models)}\nUse supported options or report missing capability; do not probe unknown fields with repeated generation calls.`
  } catch {
    return '\n\nThe image capability inventory could not be read; this does not establish that no image model exists. Do not assume universal parameter support.'
  }
}

/** The task manager's record of an AICC task the user started. */
export async function aiccTask(id: string): Promise<unknown> {
  return buckyos.getServiceRpcClient('task-manager').call('get_task', { task_id: id } as never, toAiccRpcCallOptions(await sessionToken()))
}

/** The model client of this browser: AICC with the user's session, or the dev override's endpoint. */
export function stageLlm(): LlmClient {
  const dev = devModelEndpoint()
  if (dev) return new OpenAiLlmClient({ baseUrl: dev.baseUrl, apiKey: dev.apiKey ?? 'dev' })
  return new AiccLlmClient(aiccClient() as unknown as ConstructorParameters<typeof AiccLlmClient>[0])
}

function textOf(message: AiMessage): string {
  return message.content.map((c) => (c.type === 'text' ? c.text : '')).join('')
}

export function createInfer(signal?: AbortSignal): Infer {
  const client = stageLlm()
  return async (req) => {
    try {
      const r = await client.infer({
        messages: req.messages.map((m) => ({ role: m.role, content: [{ type: 'text', text: m.text }] })),
        model_alias: req.model, fallbacks: [], temperature: 0, max_completion_tokens: req.max_tokens,
        force_json: req.schema !== null, ...(req.schema !== null ? { json_schema: req.schema as never } : {}),
        disable_capabilities: [], tool_specs: [], allow_tool_calls: false, signal: signal ?? new AbortController().signal,
      })
      // both clients name the model that answered in `extra.model` (AICC: its route's final model)
      const model = (r.extra as { model?: unknown } | undefined)?.model
      return { text: textOf(r.message), tokens: r.usage?.output_tokens, input_tokens: r.usage?.input_tokens, model: typeof model === 'string' && model ? model : undefined }
    } catch (error) {
      throw classified(error)
    }
  }
}
