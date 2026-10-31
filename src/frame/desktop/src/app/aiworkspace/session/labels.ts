/* Text keys of the session's enumerations (keys are written out, never assembled: tests/datamodel/aiworkspace-i18n). */

import type { ChannelStatus } from './channel'
import type { LinkKind, SessionState } from './types'

export const STATE_KEY: Readonly<Record<SessionState, string>> = {
  prepared: 'aiworkspace.session.state.prepared', live: 'aiworkspace.session.state.live', ended: 'aiworkspace.session.state.ended',
  archived: 'aiworkspace.session.state.archived', expired: 'aiworkspace.session.state.expired',
}

export const CHANNEL_KEY: Readonly<Record<ChannelStatus, string>> = {
  connecting: 'aiworkspace.session.channel.connecting', open: 'aiworkspace.session.channel.open', polling: 'aiworkspace.session.channel.polling',
  closed: 'aiworkspace.session.channel.closed', failed: 'aiworkspace.session.channel.failed',
}

export const LINK_KEY: Readonly<Record<LinkKind, string>> = {
  at: 'aiworkspace.session.links.kind.at', dt: 'aiworkspace.session.links.kind.dt', pt: 'aiworkspace.session.links.kind.pt',
}

export const TASK_MODE_KEY: Readonly<Record<string, string>> = {
  open: 'aiworkspace.session.choice.mode.open', assigned: 'aiworkspace.session.choice.mode.assigned', claim: 'aiworkspace.session.choice.mode.claim',
}

export const TASK_STATE_KEY: Readonly<Record<string, string>> = {
  open: 'aiworkspace.session.choice.state.open', closed: 'aiworkspace.session.choice.state.closed',
}
