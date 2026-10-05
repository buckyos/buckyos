export class Emitter {
  private readonly listeners = new Set<() => void>()
  readonly subscribe = (listener: () => void) => {
    this.listeners.add(listener)
    return () => { this.listeners.delete(listener) }
  }
  emit() {
    for (const listener of [...this.listeners]) listener()
  }
}

/** Counters keyed by a string; a component re-reads whatever the key stands for when its counter moves. */
export class VersionMap {
  private readonly versions = new Map<string, number>()
  private readonly emitter = new Emitter()
  readonly subscribe = this.emitter.subscribe
  get(key: string): number {
    return this.versions.get(key) ?? 0
  }
  bump(keys: Iterable<string>) {
    let changed = false
    for (const key of keys) { this.versions.set(key, this.get(key) + 1); changed = true }
    if (changed) this.emitter.emit()
  }
}
