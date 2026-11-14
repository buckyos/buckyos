// Types of host.js: the executor's side of a macro's bridge (AI Native 第一期 §2.3–§2.6).
export type BridgeOp = 'sql' | 'ai.complete' | 'llm.map' | 'zone.fetch' | 'ai.image' | 'ai.video'
export interface InferMessage { role: 'system' | 'user' | 'assistant'; text: string }
export interface InferRequest { model: string; messages: InferMessage[]; schema: unknown | null; max_tokens: number }
export interface ZoneAnswer { status: number; headers: Record<string, string>; text?: string; base64?: string }
export interface BridgeHostOptions {
  tool(name: string, args: unknown): Promise<{ status?: string; text?: string; code?: string; [k: string]: unknown }>
  infer(req: InferRequest): Promise<{ text: string; tokens?: number }>
  zoneFetch?(url: string, init: { method: string; headers: Record<string, string>; body: string | null }): Promise<ZoneAnswer>
  /** One AICC call with the executor's session (a refusal throws with `code`, as `infer`). */
  aicc?(method: string, params: Record<string, unknown>): Promise<any> // eslint-disable-line @typescript-eslint/no-explicit-any
  /** The task manager's record of an AICC task. */
  task?(id: string): Promise<unknown>
  /** `asset.import_object { obj_id }` of the run's Workspace: the generated file becomes a staged asset. */
  importAsset?(objId: string): Promise<{ object_id: string; media_type?: string; size?: number; image?: { width: number; height: number } }>
  zoneHosts?: string[]
  zoneOrigin?: string | null
  budget?: Record<string, unknown>
  manifest?: unknown
}
export function createBridgeHost(options: BridgeHostOptions): {
  handle(op: string, req: unknown): Promise<Record<string, unknown>>
  spent(): { calls: number; tokens: number; items: number; image: number; video: number }
}
export function canonical(v: unknown): string
export function sha256Hex(text: string): Promise<string>
export function validate(schema: unknown, v: unknown): string | null
export function extractJson(text: string): unknown
export function inZone(url: URL, zoneHosts: string[]): boolean
export const MAP_SYSTEM: string
