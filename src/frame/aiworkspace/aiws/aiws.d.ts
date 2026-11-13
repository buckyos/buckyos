// Types of the program host and of the executor's side of its bridge for TypeScript callers (the Desktop's local
// executor runs aiws.js in a Worker and answers its bridge with host.js). The program host reads and writes through
// the `Deno` global its caller provides; bridge calls go to `globalThis.__aiwsBridge(op, request)`.
export function createHost(root: string): {
  aiws: unknown
  flush(error: unknown): {
    results: unknown[]; facts: unknown; checks: unknown[]; external: string[]; ai: unknown; declare: unknown
    error: string | null; error_code: string | null
  }
  summary(): string
}
