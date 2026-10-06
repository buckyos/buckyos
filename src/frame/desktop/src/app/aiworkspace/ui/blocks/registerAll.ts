/* Register the shipped Block definitions once per app lifetime; extensions register the same way. */

import { registerBuiltinBlocks } from './builtin'
import { registerSampleBlocks, registerVideoBlock } from './samples'
import { registerDeclarativeBlock } from '../extensions/declarative'
import { registerHtmlBlock } from '../extensions/HtmlBlockHost'
import { registerWishBlock } from '../wish/wishBlock'

let registered = false
export function registerDefaultBlocks() {
  if (registered) return
  registered = true
  registerBuiltinBlocks()
  registerWishBlock()
  registerHtmlBlock()
  registerDeclarativeBlock()
  registerSampleBlocks()
  registerVideoBlock()
}
