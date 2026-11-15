// Types of the program host and of the executor's side of its bridge for TypeScript callers (the Desktop's local
// executor runs aiws.js in a Worker and answers its bridge with host.js). The program host reads and writes through
// the `Deno` global its caller provides; bridge calls go to `globalThis.__aiwsBridge(op, request)`.
/** The AI calls of a run (`results.ai`): `tokens` are output tokens, `models` the models that answered. */
export interface AiStats {
  complete: { calls: number; cached: number; tokens: number; input_tokens: number }
  map: { calls: number; items: number; cached: number; tokens: number; input_tokens: number }
  image: { calls: number; cached: number }
  video: { calls: number; cached: number }
  models: string[]
}
export function createHost(root: string): {
  aiws: unknown
  flush(error: unknown): {
    results: unknown[]; facts: unknown; checks: unknown[]; external: string[]; ai: AiStats; declare: unknown
    error: string | null; error_code: string | null
  }
  summary(): string
}
