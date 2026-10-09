import type { CommentType, ContentType, Did, EntryUrl, FeedObject, FileObject, ObjId, PublicationCategory } from '../protocol/feed'

export type IdentityKind = 'person' | 'site' | 'agent' | 'self'

export interface IdentityView {
  did?: Did
  name: string
  kind: IdentityKind
  hue: number
}

export type Verification = 'verified' | 'unverified' | 'wrapper_only'

export type AudienceSpec =
  | { kind: 'public' }
  | { kind: 'followers' }
  | { kind: 'friends' }
  | { kind: 'group'; groupId: string }
  | { kind: 'dids'; dids: Did[] }

export interface AudienceView {
  spec: AudienceSpec
  restricted: boolean
}

export type EntryHeadState = 'active' | 'withdrawn' | 'conflict'

export interface EntryState {
  entry: EntryUrl
  seq: number
  state: EntryHeadState
  currentObjId?: ObjId
  isLatest: boolean
  version: number
  versionCount: number
}

export type ResourceState = 'local' | 'reachable' | 'preparing' | 'unavailable'

export interface ResolvedMedia {
  object: ObjId
  alt?: string
  file?: FileObject
}

export type InnerVisibility = 'visible' | 'not_visible' | 'withdrawn' | 'missing'

export interface EmbeddedView {
  relation: 'repost' | 'quote' | 'comment_on' | 'wraps'
  visibility: InnerVisibility
  item?: FeedItemView
}

export interface FeedItemView {
  objId: ObjId
  object: FeedObject
  contentType: ContentType | 'comment' | 'reaction'
  publisher: IdentityView
  originalAuthor?: string
  capturedFrom?: string
  isCapture: boolean
  isPrivateCapture: boolean
  verification: Verification
  entry?: EntryState
  audience: AudienceView
  media: ResolvedMedia[]
  cover?: ResolvedMedia
  wrappedFile?: FileObject
  embedded?: EmbeddedView
  category?: PublicationCategory
  createdAt: number
  isOwn: boolean
}

export type ReasonCode = 'followed' | 'friend' | 'topic' | 'subscription' | 'collector' | 'rule'

export interface ReasonRef {
  kind: 'source' | 'topic' | 'rule' | 'intent' | 'collector'
  id: string
  label: string
}

export interface ReadingReason {
  code: ReasonCode
  text: string
  refs: ReasonRef[]
}

export type TagSource = 'author' | 'model' | 'user'
export type TagStatus = 'declared' | 'inferred' | 'confirmed'
export type TagScope = 'whole_content' | 'part'
export type GenerationTag = 'ai_full' | 'ai_assisted' | 'low_quality'

export interface EffectiveTag {
  tag: string
  label: string
  source: TagSource
  status: TagStatus
  scope: TagScope
  confidence?: number
  basis?: string
  classifierRevision?: string
}

export interface ReadingEntry {
  objId: ObjId
  admittedAt: number
  reason: ReadingReason
  effectiveTags: EffectiveTag[]
  resources: ResourceState
  filteredBy: string[]
  topics: string[]
}

export type InteractionVisibility = 'public' | 'author_only' | 'private'
export type DeliveryState = 'none' | 'delivering' | 'delivered' | 'partially_failed'

export interface InteractionFlag {
  on: boolean
  visibility: InteractionVisibility
  delivery: DeliveryState
  entry?: EntryUrl
  seq?: number
}

export interface PersonalState {
  like: InteractionFlag
  bookmark: InteractionFlag
  repost: InteractionFlag
  readLater: boolean
  dislike: boolean
}

export type StatsViewKey = 'local' | 'author' | `collector:${string}`

export interface InteractionStats {
  view: StatsViewKey
  asOf: number
  sync: 'synced' | 'syncing' | 'partial'
  textComments: number
  likes: number
  reposts: number
  quotes: number
  claimed?: { likes?: number; source: string }
}

export interface CardView {
  item: FeedItemView
  reading?: ReadingEntry
  personal?: PersonalState
  stats?: InteractionStats
  resources: ResourceState
  canRepost: boolean
  repostBlockedReason?: 'restricted' | 'private_capture' | 'withdrawn' | 'not_visible' | 'reaction'
  sharedAs?: ObjId
}

export type CandidateSelection = 'unscreened' | 'not_selected' | 'preparing'

export interface SourcePath {
  transport: 'pull' | 'push'
  label: string
}

export interface CandidateEntry {
  objId: ObjId
  selection: CandidateSelection
  sourcePaths: SourcePath[]
  arrivedAt: number
  firstAdmittedAt: number | null
  openedAt: number | null
  resources: ResourceState
}

export type CommentSourceKind = 'author_list' | 'collector' | 'push' | 'participant'

export interface CommentSourcePath {
  kind: CommentSourceKind
  label: string
}

export interface CommentView {
  objId: ObjId
  item: FeedItemView
  commentType: CommentType
  targetObjId: ObjId
  onOldVersion: boolean
  targetVersion: number
  sourcePaths: CommentSourcePath[]
  listedByAuthor: boolean
  listedByCollector: boolean
}

export type PublishStage = 'uploading' | 'failed' | 'published'

export interface DeliveryProgress {
  state: 'delivering' | 'delivered' | 'partially_failed'
  delivered: number
  total: number
}

export interface PublishTask {
  key: string
  stage: PublishStage
  entry?: EntryUrl
  objId?: ObjId
  delivery?: DeliveryProgress
  error?: string
  createdAt: number
}

export type FollowBasis = 'active' | 'friend'
export type NotifyState = 'acknowledged' | 'unsupported' | 'retrying' | 'pending'
export type SourceKind = 'person' | 'rss' | 'website' | 'channel'

export interface SourceView {
  id: string
  name: string
  kind: SourceKind
  did?: Did
  url?: string
  description?: string
  basis: FollowBasis[]
  notify: NotifyState
  lastSuccessAt?: number
  lastError?: { at: number; reason: string }
  paused: boolean
  intentId?: string
  credibility?: 'verified_did' | 'known_site' | 'unknown'
  updateMode?: string
}

export interface SubscriptionIntent {
  id: string
  text: string
  sourceIds: string[]
  status: 'collecting' | 'mapping' | 'paused'
  updatedAt: number
}

export type MuteRule =
  | { kind: 'person'; did: Did; name: string }
  | { kind: 'group'; groupId: string; name: string }

export interface FilterRule {
  id: string
  enabled: boolean
  conditions: GenerationTag[]
  acceptInferred: boolean
  minConfidence: number
  unknown: 'show' | 'hide'
}

export interface ContactGroup {
  id: string
  name: string
  members: Did[]
}

export interface TopicView {
  id: string
  name: string
  tags: string[]
  subscribed: boolean
  recentCount: number
}

export type FeedFilter = 'all' | 'following' | 'images' | 'videos' | 'longform' | 'news'

export interface ReadingQuery {
  filter: FeedFilter
  topicId: string | null
  search: string
  showFiltered: boolean
}

export interface ReadingPage {
  objIds: ObjId[]
  nextCursor: string | null
  hiddenByRules: number
  hiddenByMute: number
  total: number
}

export interface CandidatePage {
  entries: CandidateEntry[]
  nextCursor: string | null
  readCount: number
  retentionDays: number
  lastFetchAt: number
}

export type ReaderIdentity = { kind: 'owner' } | { kind: 'anonymous' } | { kind: 'did'; did: Did }

export type PublishedKindFilter = 'all' | 'posts' | 'comments' | 'reposts' | 'reactions' | PublicationCategory

export interface PublishedEntryView {
  entry: EntryUrl
  objId?: ObjId
  head: EntryState
  audience: AudienceView
  task?: PublishTask
  kind: 'post' | 'comment' | 'repost' | 'quote' | 'like' | 'bookmark'
  publishedAt: number
}

export interface PublishedPage {
  entries: PublishedEntryView[]
  nextCursor: string | null
  changeCursor: number
}

export interface StreamChange {
  cursor: number
  entry: EntryUrl
  seq: number
  state: EntryHeadState
  current?: ObjId
  kind: 'head' | 'audience'
  at: number
}

export interface ProfileView {
  did: Did
  name: string
  bio: string
  hue: number
  followers: number
  following: number
  posts: number
  featured: ObjId[]
}

export interface SyncStatus {
  candidates: number
  preparing: number
  lastFetchAt: number
  sources: number
  failingSources: number
}

export interface SavedItem {
  objId: ObjId
  savedAt: number
  visibility: InteractionVisibility
  targetState: 'active' | 'withdrawn' | 'updated'
}

export type WrappedBody =
  | { state: 'ready'; markdown: string; file: FileObject }
  | { state: 'preparing' }
  | { state: 'unavailable'; reason: string }

export interface SourceResolution {
  inputKind: 'follow' | 'url' | 'natural'
  input: string
  candidates: SourceView[]
  notifyHint: NotifyState
  intentText?: string
}

export const EMPTY_PERSONAL: PersonalState = {
  like: { on: false, visibility: 'public', delivery: 'none' },
  bookmark: { on: false, visibility: 'private', delivery: 'none' },
  repost: { on: false, visibility: 'public', delivery: 'none' },
  readLater: false,
  dislike: false,
}
