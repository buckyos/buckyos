/* The colour of each data a control shows from a list binding (控件 on_bind TODO Q3): the host hands them out in order
 * from one palette, `config.colors` names one per data (`{ <entity id>: "#rrggbb" }`). */

import type { Json } from '../../../api/types'

export const DATA_COLORS = ['#2563eb', '#16a34a', '#d97706', '#db2777', '#7c3aed', '#0891b2', '#dc2626', '#4b5563']

const HEX = /^#[0-9a-fA-F]{6}$/

/** The colour of each id, in order (the palette repeats past its end). */
export function dataColors(ids: string[], config: Record<string, Json> | null | undefined): Map<string, string> {
  const set = (config?.colors ?? {}) as Record<string, Json>
  return new Map(ids.map((id, i) => [id, typeof set[id] === 'string' && HEX.test(set[id] as string) ? set[id] as string : DATA_COLORS[i % DATA_COLORS.length]]))
}
