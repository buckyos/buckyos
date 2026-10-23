// Types of the program host for TypeScript callers (the Desktop's local executor runs aiws.js in a Worker,
// 内核架构改进 T3-4). The host reads and writes through the `Deno` global its caller provides.
export function createHost(root: string): {
  aiws: unknown
  flush(error: unknown): { results: unknown[]; facts: unknown; checks: unknown[]; external: string[]; llm_map: unknown; error: string | null }
  summary(): string
}
