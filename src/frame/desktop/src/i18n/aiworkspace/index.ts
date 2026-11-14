/* AI Workspace text tables (keys `aiworkspace.<area>.<name>`), one file per area of the app.
 * Only the app reads them, through src/app/aiworkspace/i18n.ts: they stay out of the Desktop dictionaries
 * (and so out of the Desktop's main bundle) because nothing else shows AI Workspace text. */

import * as activity from './activity'
import * as blocks from './blocks'
import * as blockui from './blockui'
import * as canvas from './canvas'
import * as common from './common'
import * as core from './core'
import * as customize from './customize'
import * as data from './data'
import * as datamode from './datamode'
import * as demos from './demos'
import * as directory from './directory'
import * as ext from './ext'
import * as framework from './framework'
import * as home from './home'
import * as imports from './import'
import * as members from './members'
import * as mindmap from './mindmap'
import * as office from './office'
import * as pdf from './pdf'
import * as permissions from './permissions'
import * as phone from './phone'
import * as presentation from './presentation'
import * as richtext from './richtext'
import * as schedule from './schedule'
import * as session from './session'
import * as settings from './settings'
import * as shell from './shell'
import * as surfaces from './surfaces'
import * as table from './table'
import * as wish from './wish'
import * as worklog from './worklog'

const parts = [common, shell, canvas, blocks, wish, data, directory, presentation, richtext, core, demos, mindmap, pdf, ext, imports, session, office, schedule, framework, home, surfaces, settings, activity, table, permissions, blockui, customize, members, worklog, datamode, phone]

export const aiWorkspaceEn: Record<string, string> = Object.assign({}, ...parts.map((part) => part.en))
export const aiWorkspaceZh: Record<string, string> = Object.assign({}, ...parts.map((part) => part.zh))
