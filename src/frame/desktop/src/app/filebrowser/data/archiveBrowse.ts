/**
 * "Browse archives as folders" (archive TODO D13): on by default, kept like the
 * icon size. On, opening a `.zip` the server opens (`hello.archive`) enters it;
 * off, it goes to Preview by the usual content-open rules.
 */

import { useSyncExternalStore } from 'react'

const KEY = 'files.browseArchives'
const listeners = new Set<() => void>()

function read(): boolean {
  try {
    return localStorage.getItem(KEY) !== 'off'
  } catch {
    return true
  }
}

let current = read()

export function setBrowseArchives(on: boolean) {
  current = on
  try { localStorage.setItem(KEY, on ? 'on' : 'off') } catch { /* no storage */ }
  listeners.forEach((listener) => listener())
}

export function browseArchives(): boolean {
  return current
}

export function useBrowseArchives(): boolean {
  return useSyncExternalStore(
    (listener) => { listeners.add(listener); return () => { listeners.delete(listener) } },
    () => current,
  )
}
