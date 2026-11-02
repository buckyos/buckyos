/* The classroom extension (第四期规划 §1: the default implementation of classroom interaction, replaceable): its
 * controls, registered like every other Block definition. */

import '../../session/session.css'
import { blockRegistry } from '../blocks/registry'
import { audienceDefinition } from './AudienceBlock'
import { choiceDefinition } from './ChoiceBlock'

let registered = false
export function registerClassroomBlocks() {
  if (registered) return
  registered = true
  blockRegistry.register(choiceDefinition)
  blockRegistry.register(audienceDefinition)
}
