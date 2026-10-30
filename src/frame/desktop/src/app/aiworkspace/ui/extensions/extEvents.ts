/* "Insert an instance of this control" from places that are not the canvas (the Workspace controls dialog): the open
 * canvas listens and opens its insert catalog at the definition. */

type Listener = (defEntityId: string) => void
const listeners = new Set<Listener>()

/** Ask the open canvas to insert an instance; false when no canvas listens. */
export function requestInsertDefinition(defEntityId: string): boolean {
  for (const listener of listeners) listener(defEntityId)
  return listeners.size > 0
}

export function onInsertDefinition(listener: Listener): () => void {
  listeners.add(listener)
  return () => { listeners.delete(listener) }
}
