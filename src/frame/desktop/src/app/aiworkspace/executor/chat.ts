/* The model the local executor talks to (内核架构改进 T3-4, D14): the Zone's AICC with the user's own
 * session (`helper.llm_chat` for a logical model, `chat.completions.create` for an exact one), or —
 * development against the standalone backend — an OpenAI-format endpoint named in the dev override. */

import { buckyos, getActiveSessionToken } from 'buckyos'
import { toAiccRpcCallOptions } from '../../../api/aicc_rpc_options'
import { devModelEndpoint } from '../api/transport'
import type { Json } from '../api/types'
import { aiwsT } from '../i18n'

export interface ChatTool { name: string; description: string; parameters: Json }
export interface ChatToolCall { id: string; name: string; args: Record<string, Json> }
export type ChatMessage =
  /** `raw`: the assistant message exactly as the provider returned it (AICC keeps provider state in it), sent back as is. */
  | { role: 'system' | 'user' | 'assistant'; text: string; toolCalls?: ChatToolCall[]; raw?: unknown }
  | { role: 'tool'; callId: string; text: string; isError?: boolean }
export interface ChatTurn { text: string; toolCalls: ChatToolCall[]; raw?: unknown }

export interface ChatClient {
  complete(model: string, messages: ChatMessage[], tools: ChatTool[], signal?: AbortSignal): Promise<ChatTurn>
}

function record(value: unknown): Record<string, unknown> {
  return value && typeof value === 'object' && !Array.isArray(value) ? (value as Record<string, unknown>) : {}
}

function parseArgs(raw: unknown): Record<string, Json> {
  if (typeof raw === 'string') { try { return record(JSON.parse(raw)) as Record<string, Json> } catch { return {} } }
  return record(raw) as Record<string, Json>
}

/** AICC through kRPC: the user's session pays and is attributed (the service's own run record keeps the context). */
class AiccChat implements ChatClient {
  async complete(model: string, messages: ChatMessage[], tools: ChatTool[]): Promise<ChatTurn> {
    const account = await buckyos.getAccountInfo().catch(() => null)
    const token = account?.session_token || await getActiveSessionToken()
    if (!token) throw new Error(aiwsT('aiworkspace.wish.error.noSession'))
    const encoded = messages.map((m) => m.role === 'tool'
      ? { role: 'tool', content: [{ type: 'tool_result', call_id: m.callId, content: [{ type: 'text', text: m.text }], ...(m.isError ? { is_error: true } : {}) }] }
      : m.raw ?? { role: m.role, content: [...(m.text ? [{ type: 'text', text: m.text }] : []), ...(m.toolCalls ?? []).map((c) => ({ type: 'tool_use', call_id: c.id, name: c.name, args: c.args }))] })
    const specs = tools.map((t) => ({ type: 'function', name: t.name, description: t.description, args_json_schema: t.parameters }))
    const exact = model.includes('@')
    const params = exact
      ? { exact_model: model, messages: encoded, tools: specs }
      : { logical_model: model, messages: encoded, tools: specs, requirements: { tool_call: tools.length > 0 } }
    const result = record(await buckyos.getServiceRpcClient('aicc').call(exact ? 'chat.completions.create' : 'helper.llm_chat', params, toAiccRpcCallOptions(token)))
    const message = record(result.message)
    const content = Array.isArray(message.content) ? message.content.map(record) : []
    const text = content.filter((c) => c.type === 'text').map((c) => String(c.text ?? '')).join('\n')
    const calls = (Array.isArray(result.tool_calls) && result.tool_calls.length ? result.tool_calls : content.filter((c) => c.type === 'tool_use')).map(record)
    return { text, toolCalls: calls.map((c, i) => ({ id: String(c.call_id ?? `call_${i}`), name: String(c.name ?? ''), args: parseArgs(c.args) })), raw: result.message }
  }
}

/** Development: an OpenAI-format `/chat/completions` (the e2e suite's scripted model). */
class OpenAiChat implements ChatClient {
  private readonly baseUrl: string
  private readonly apiKey?: string

  constructor(baseUrl: string, apiKey?: string) {
    this.baseUrl = baseUrl
    this.apiKey = apiKey
  }

  async complete(model: string, messages: ChatMessage[], tools: ChatTool[], signal?: AbortSignal): Promise<ChatTurn> {
    const encoded = messages.map((m) => m.role === 'tool'
      ? { role: 'tool', tool_call_id: m.callId, content: m.text }
      : { role: m.role, content: m.text || null, ...(m.toolCalls?.length ? { tool_calls: m.toolCalls.map((c) => ({ id: c.id, type: 'function', function: { name: c.name, arguments: JSON.stringify(c.args) } })) } : {}) })
    const body = { model, messages: encoded, ...(tools.length ? { tools: tools.map((t) => ({ type: 'function', function: { name: t.name, description: t.description, parameters: t.parameters } })) } : {}) }
    const response = await fetch(`${this.baseUrl.replace(/\/+$/, '')}/chat/completions`, { method: 'POST', headers: { 'Content-Type': 'application/json', ...(this.apiKey ? { Authorization: `Bearer ${this.apiKey}` } : {}) }, body: JSON.stringify(body), signal })
    const json = record(await response.json().catch(() => ({})))
    if (!response.ok) throw new Error(String(record(json.error).message ?? `HTTP ${response.status}`))
    const message = record(record((json.choices as unknown[] | undefined)?.[0]).message)
    const calls = Array.isArray(message.tool_calls) ? message.tool_calls.map(record) : []
    return { text: typeof message.content === 'string' ? message.content : '', toolCalls: calls.map((c, i) => ({ id: String(c.id ?? `call_${i}`), name: String(record(c.function).name ?? ''), args: parseArgs(record(c.function).arguments) })) }
  }
}

export function resolveChatClient(): ChatClient {
  const dev = devModelEndpoint()
  return dev ? new OpenAiChat(dev.baseUrl, dev.apiKey) : new AiccChat()
}
