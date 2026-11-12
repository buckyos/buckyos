/* Register the built-in Block-controls and the Blocks they bring (脑图 TODO M1): the mind map control and its text
 * node. Called with the Block registrations, once per app lifetime. */

import { blockRegistry } from '../blocks/registry'
import { compositeControl } from './composite/definition'
import { mindmapControl } from './mindmap/definition'
import { mindnodeDefinition } from './mindmap/nodeBlock'
import { controlRegistry } from './registry'
import { todoControl } from './todo/definition'
import { calendarControl } from './calendar/definition'
import { labelDefinition } from '../blocks/labelBlock'

export function registerControls() {
  controlRegistry.register(mindmapControl)
  controlRegistry.register(compositeControl)
  controlRegistry.register(todoControl)
  controlRegistry.register(calendarControl)
  blockRegistry.register(mindnodeDefinition)
  blockRegistry.register(labelDefinition)
}
