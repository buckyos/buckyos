/* Keys of a data control: while the control is selected on the canvas, the canvas hands key presses to it
 * (`ContainerDefinition.keys`); the mounted body registers the handler that knows its state (selection, view). */

type KeyHandler = (event: KeyboardEvent) => boolean

const handlers = new Map<string, KeyHandler>()

/** Register the body's key handler; returns the unregister function (an effect's cleanup). */
export function registerControlKeys(containerId: string, handler: KeyHandler): () => void {
  handlers.set(containerId, handler)
  return () => { if (handlers.get(containerId) === handler) handlers.delete(containerId) }
}

/** Give a key press to the control's body (true: handled). */
export function controlKey(containerId: string, event: KeyboardEvent): boolean {
  return handlers.get(containerId)?.(event) ?? false
}
