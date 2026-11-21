/* Field type → the icon of its column header and its row in the field list. */

import { Banknote, Braces, Calendar, CalendarClock, CircleChevronDown, Hash, Image, LayoutDashboard, Link2, SquareCheck, Tags, Type } from 'lucide-react'
import type { FieldDef } from '../../api/types'

export const FIELD_ICONS: Partial<Record<FieldDef['type'], typeof Type>> = {
  text: Type, number: Hash, decimal: Hash, currency: Banknote, date: Calendar, datetime: CalendarClock, boolean: SquareCheck, select: CircleChevronDown, multi_select: Tags, object_ref: Link2, json: Braces,
}

export function fieldIcon(field: FieldDef) {
  if (field.ref_purpose === 'image') return Image
  if (field.ref_purpose === 'canvas') return LayoutDashboard
  return FIELD_ICONS[field.type] ?? Type
}
