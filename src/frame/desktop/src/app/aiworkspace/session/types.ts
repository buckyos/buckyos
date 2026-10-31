/* The presentation session's wire shapes (第四期规划; service README “Presentation sessions”). */

import type { Json, SessionFeed } from '../api/types'

export type SessionState = 'prepared' | 'live' | 'ended' | 'archived' | 'expired'
export type SessionRole = 'presenter' | 'owner' | 'audience' | 'display' | 'prompter'
export type LinkKind = 'at' | 'dt' | 'pt'

export interface SessionPolicy { guest_interact: boolean; voice: boolean; show_names: boolean; expected_audience: number; media_limit_mib: number }

/** The authoritative stage (Q01-C): what the service last confirmed. */
export interface SessionStage {
  step_id?: string | null
  index?: number
  count?: number
  title?: string | null
  surface_id?: string | null
  black?: boolean
  free?: boolean
  transition?: 'fly' | 'fade' | 'cut'
  nonce?: number
  media?: Record<string, MediaState>
  rect?: { x: number; y: number; w: number; h: number } | null
  followers?: number
  updated_ms?: number
}

/** A media Block's playback state (§7.3.2): position at a demonstration time, and the rate it runs at. */
export interface MediaState { action: 'play' | 'pause' | 'seek' | 'rate' | 'ended' | 'sync'; media_pos_ms: number; t_rel_ms: number; rate: number; playing: boolean }

export interface SessionStep { id: string; kind: 'frame' | 'viewport'; target_id: string; surface_id: string | null; title: string; transition: string; hide: string[] }

export interface SessionArchive { complete: boolean; duration_ms: number; missing: Json[]; gaps: Json[]; voice: Json[]; saved_at: string; events: { first: number; last: number } }

export interface SessionInfo {
  session_id: string
  title: string
  state: SessionState
  access: 'open' | 'closed'
  posts: 'open' | 'closed'
  workspace_id: string
  source_workspace_id: string
  path_id: string
  owner: string
  presenter: string
  guest_interact: boolean
  voice: boolean
  show_names: boolean
  created_at: string
  started_at: string | null
  ended_at: string | null
  saved_at: string | null
  expires_at: string | null
  controller: string | null
  policy?: SessionPolicy
  baseline?: { seq: number; epoch: string; snapshot_id: string; content_root: string; notes_entity_id?: string | null; source: { workspace_id: string; epoch: string; seq: number } }
  archive?: SessionArchive
  // session.get / join
  role?: SessionRole
  principal?: string
  level?: 'verified' | 'guest'
  owner_actions?: boolean
  stage?: SessionStage
  steps?: SessionStep[]
  head_event?: number
  t_rel_ms?: number
  voice_service?: { available: boolean; reason?: string; port?: number; hosts?: string[] }
  online?: number
  my_feeds?: Record<string, string>
  participants?: SessionParticipant[]
}

export interface SessionParticipant { principal: string; name: string; class: string; level: 'verified' | 'guest'; role: string; online: boolean; joined_at: string; last_seen: string }

export interface SessionEvent {
  seq: number
  event_id?: string
  session_id?: string
  t_rel_ms: number
  received_at?: string
  actor?: string
  scope?: string
  track?: string
  type: string
  command_id?: string | null
  schema?: number
  data: Json & Record<string, Json>
  commit_seq?: number | null
}

export interface Invite { invite_id: string; kind: LinkKind; token: string; label: string | null; config: Record<string, Json>; created_by: string; created_at: string; revoked: boolean }

export interface SessionPost { post_id: string; kind: 'comment' | 'rating' | 'moment'; author: string; author_name: string | null; body: string | null; score: number | null; t_rel_ms: number | null; created_at: string }

export interface ReplayBundle { workspace_id: string; epoch: string; baseline_seq: number; head_seq: number; baseline: Record<string, Json[]>; duration_ms: number; head_event: number; archive: Json | null }

export type FeedName = SessionFeed
