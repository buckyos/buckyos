/* The `composite` host (扩展控件 P1, K4): one ContainerDefinition for every composite extension control. Which control
 * an instance is, is its `def_ref`; the layout comes from that definition's `composition` (the shared solver), the
 * frame from its `chrome`, the properties from its `config_schema`. Inserting one creates the container and the
 * Blocks its slots ask for (`min`); its Blocks stay ordinary Blocks of the BlockTree. */

import { LayoutPanelTop } from 'lucide-react'
import { COMPOSITE_CONTROL, solveComposite } from '@controls/composite'
import { aiwsT } from '../../../i18n'
import type { ContainerDefinition } from '../registry'
import { CompositeChrome } from './Chrome'
import { COMPOSITE, compositeCreate } from './model'

export const compositeControl: ContainerDefinition = {
  kind: COMPOSITE, version: 1, get title() { return aiwsT('aiworkspace.ext.composite.title') }, icon: LayoutPanelTop,
  // the properties are each definition's (`config_schema`), checked by the clients (Q15)
  configSchema: { type: 'object' },
  children: { accepts: 'any', containers: true },
  layout: { direction: 'bottom_up', algorithm: 'fixed', solve: solveComposite },
  Chrome: CompositeChrome,
  create: compositeCreate,
  agent: COMPOSITE_CONTROL,
}
