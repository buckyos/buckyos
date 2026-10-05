/* Lazy loader of the shared Rust core (aiworkspace-wasm, wasm-bindgen `--target web`).
 * Regenerate with src/frame/aiworkspace/wasm/build.sh. */

import init, * as core from '../wasm/aiworkspace_wasm.js'
import wasmUrl from '../wasm/aiworkspace_wasm_bg.wasm?url'
import type { RichTextSchemaDef } from '../richtext/schema'
import type { AstNode } from './types'

export type AiwsCore = typeof core

let loading: Promise<AiwsCore> | null = null

export function loadCore(): Promise<AiwsCore> {
  loading ??= init({ module_or_path: wasmUrl }).then(() => core)
  return loading
}

/** `order_key` strictly between two siblings (either side may be absent). Same function as the backend. */
export function orderKeyBetween(wasm: AiwsCore, before?: string | null, after?: string | null): string {
  return wasm.order_key_between(before ?? undefined, after ?? undefined)
}

export function richTextSchemaDef(wasm: AiwsCore): RichTextSchemaDef {
  return JSON.parse(wasm.richtext_schema()) as RichTextSchemaDef
}

/** Canonical AST (default attrs dropped, marks sorted, adjacent text merged) — throws on a schema violation. */
export function canonicalAst(wasm: AiwsCore, ast: unknown): AstNode {
  return JSON.parse(wasm.richtext_canonicalize(JSON.stringify(ast))) as AstNode
}
