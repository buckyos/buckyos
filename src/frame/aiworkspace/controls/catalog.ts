/* The control catalogue (脑图方案 §8.1, R11): every built-in Block-control with its one-line summary and the
 * self-description an Agent reads on demand. `aiws/controls.json` is generated from it
 * (`deno run -A tools/build_catalog.ts`; catalog_test.ts checks it is current): the wish prompts carry it as
 * `catalog/controls.json`, the `aiws controls list / describe` verbs print it. */

import { MINDMAP } from './mindmap.ts'

export interface ControlDescriptor {
  /** The control id: the container's `kind`. */
  kind: string
  title: string
  /** One sentence: what it is. */
  summary: string
  /** When to use it. */
  when: string
  /** Exchange formats `aiws import / export` and wish results accept. */
  formats: string[]
  /** Markdown for a model: what it is, config, child rules, doc.commit examples, formats, don'ts. */
  agent: string
}

export const CONTROLS: readonly ControlDescriptor[] = [MINDMAP]

export function controlDescriptor(kind: string): ControlDescriptor | undefined {
  return CONTROLS.find((c) => c.kind === kind)
}

/** What `aiws/controls.json` holds. */
export function controlsCatalog(): { description: string; controls: ControlDescriptor[] } {
  return {
    description: 'Block-controls a canvas offers (containers whose kind is the control id; their Blocks are laid out by the control). Generated from src/frame/aiworkspace/controls/catalog.ts — do not edit.',
    controls: CONTROLS.map((c) => ({ ...c })),
  }
}

export function catalogJson(): string {
  return JSON.stringify(controlsCatalog(), null, 2) + '\n'
}
