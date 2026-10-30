/* AI Workspace text tables (keys `aiworkspace.<area>.<name>`), one file per area of the app.
 * Only the app reads them, through src/app/aiworkspace/i18n.ts: they stay out of the Desktop dictionaries
 * (and so out of the Desktop's main bundle) because nothing else shows AI Workspace text. */

import * as blocks from './blocks'
import * as canvas from './canvas'
import * as common from './common'
import * as core from './core'
import * as data from './data'
import * as demos from './demos'
import * as ext from './ext'
import * as imports from './import'
import * as mindmap from './mindmap'
import * as pdf from './pdf'
import * as presentation from './presentation'
import * as richtext from './richtext'
import * as shell from './shell'
import * as wish from './wish'

const parts = [common, shell, canvas, blocks, wish, data, presentation, richtext, core, demos, mindmap, pdf, ext, imports]

export const aiWorkspaceEn: Record<string, string> = Object.assign({}, ...parts.map((part) => part.en))
export const aiWorkspaceZh: Record<string, string> = Object.assign({}, ...parts.map((part) => part.zh))
