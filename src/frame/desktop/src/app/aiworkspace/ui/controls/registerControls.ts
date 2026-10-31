/* Register the built-in Block-controls and the Blocks they bring (脑图 TODO M1): the mind map control and its text
 * node. Called with the Block registrations, once per app lifetime. */

import { blockRegistry } from '../blocks/registry'
import { compositeControl } from './composite/definition'
import { mindmapControl } from './mindmap/definition'
import { mindnodeDefinition } from './mindmap/nodeBlock'
import { controlRegistry } from './registry'

export function registerControls() {
  controlRegistry.register(mindmapControl)
  controlRegistry.register(compositeControl)
  blockRegistry.register(mindnodeDefinition)
}
