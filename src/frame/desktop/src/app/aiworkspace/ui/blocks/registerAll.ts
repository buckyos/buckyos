/* Register the shipped Block definitions (and the built-in Block-controls) once per app lifetime; extensions register the same way. */

import { registerBuiltinBlocks } from './builtin'
import { registerSampleBlocks, registerVideoBlock } from './samples'
import { registerDeclarativeBlock } from '../extensions/declarative'
import { registerHtmlBlock } from '../extensions/HtmlBlockHost'
import { registerWishBlock } from '../wish/wishBlock'
import { connectorDefinition } from '../canvas/connectors/definition'
import { inkDefinition } from '../../ink/inkBlock'
import { registerControls } from '../controls/registerControls'
import { blockRegistry } from './registry'
import { testHooks } from '../../api/testHooks'

let registered = false
export function registerDefaultBlocks() {
  if (registered) return
  registered = true
  registerBuiltinBlocks()
  blockRegistry.register(connectorDefinition)
  blockRegistry.register(inkDefinition)
  registerControls()
  registerWishBlock()
  registerHtmlBlock()
  registerDeclarativeBlock()
  registerSampleBlocks()
  registerVideoBlock()
  const hooks = testHooks()
  if (hooks) hooks.blocks = blockRegistry
}
