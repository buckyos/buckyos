/* Direct inference for a wish macro run in this browser (浏览器优先 Q1, AI Native 第一期 §2.4): `aiws.ai.complete` and
 * `aiws.llm.map` call the model with the user's own AICC session — the service calls no model. The client is xllm's
 * `AiccLlmClient` over the page's kRPC client (or, in development, the OpenAI-format endpoint of the dev override),
 * so a refusal arrives classified (`refused { code }`), never as text to match. */

import { buckyos, getActiveSessionToken } from 'buckyos'
import { AiccLlmClient, LLMComputeException, providerErrorFromRpc, type AiMessage, type LlmClient } from 'buckyos/xllm'
import { OpenAiLlmClient } from 'buckyos/xllm/dev'
import { toAiccRpcCallOptions } from '../../../api/aicc_rpc_options'
import { devModelEndpoint } from '../api/transport'
import { aiwsT } from '../i18n'

export interface InferRequest { model: string; messages: { role: 'system' | 'user' | 'assistant'; text: string }[]; schema: unknown | null; max_tokens: number }
export type Infer = (req: InferRequest) => Promise<{ text: string; tokens?: number }>

/** The run's sub-code of an AICC refusal (§7.4). */
const REFUSALS: Record<string, string> = {
  quota_exhausted: 'AI_QUOTA_EXHAUSTED',
  quota_budget_exceeded: 'AI_QUOTA_BUDGET_EXCEEDED',
  resource_not_permitted: 'AI_RESOURCE_NOT_PERMITTED',
  attribution_unavailable: 'AI_ATTRIBUTION_UNAVAILABLE',
  credentials: 'AI_UNAVAILABLE',
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

function llmClient(): LlmClient {
  const dev = devModelEndpoint()
  if (dev) return new OpenAiLlmClient({ baseUrl: dev.baseUrl, apiKey: dev.apiKey ?? 'dev' })
  return new AiccLlmClient(aiccClient() as unknown as ConstructorParameters<typeof AiccLlmClient>[0])
}

function textOf(message: AiMessage): string {
  return message.content.map((c) => (c.type === 'text' ? c.text : '')).join('')
}

export function createInfer(signal?: AbortSignal): Infer {
  const client = llmClient()
  return async (req) => {
    try {
      const r = await client.infer({
        messages: req.messages.map((m) => ({ role: m.role, content: [{ type: 'text', text: m.text }] })),
        model_alias: req.model, fallbacks: [], temperature: 0, max_completion_tokens: req.max_tokens,
        force_json: req.schema !== null, ...(req.schema !== null ? { json_schema: req.schema as never } : {}),
        disable_capabilities: [], tool_specs: [], allow_tool_calls: false, signal: signal ?? new AbortController().signal,
      })
      return { text: textOf(r.message), tokens: r.usage?.output_tokens }
    } catch (error) {
      throw classified(error)
    }
  }
}
