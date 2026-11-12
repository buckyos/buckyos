/* The requests of the host's default binding dialogs (控件 on_bind TODO §2.2): `bindDialogs` asks the BindDialogHost the
 * workspace page mounts; without one every dialog answers "cancelled". */

import type { BindDialogs, DataQuery } from 'buckyos'

export type Ask =
  | { kind: 'one'; title: string; note?: string; query: DataQuery; allowNew: boolean; current: string | null; resolve: (v: string | { new: true } | null) => void }
  | { kind: 'many'; title: string; note?: string; query: DataQuery; preselect: 'all' | 'none' | string[]; allowNew: boolean; resolve: (v: (string | { new: true })[] | null) => void }
  | { kind: 'confirm'; title: string; text: string; resolve: (v: boolean) => void }

const hosts = new Set<(ask: Ask) => void>()

export function registerDialogHost(host: (ask: Ask) => void): () => void {
  hosts.add(host)
  return () => { hosts.delete(host) }
}

function open(ask: Ask) {
  const host = [...hosts].at(-1)
  if (!host) { if (ask.kind === 'confirm') ask.resolve(false); else ask.resolve(null); return }
  host(ask)
}

/** The dialogs a placement or a re-bind asks on this page. */
export const bindDialogs: BindDialogs = {
  pickOne: (o) => new Promise((resolve) => open({ kind: 'one', title: o.title, note: o.note, query: o.query, allowNew: o.allow_new === true, current: o.current ?? null, resolve })),
  pickMany: (o) => new Promise((resolve) => open({ kind: 'many', title: o.title, note: o.note, query: o.query, preselect: o.preselect ?? 'none', allowNew: o.allow_new === true, resolve })),
  confirm: (o) => new Promise((resolve) => open({ kind: 'confirm', title: o.title, text: o.text, resolve })),
}
