import type { AttachmentInput, PublishInput } from '../datamodel/inputs'
import type {
  AudienceSpec,
  CandidatePage,
  CardView,
  CommentView,
  ContactGroup,
  FilterRule,
  HomeInfo,
  IdentityView,
  InteractionStats,
  MuteRule,
  PersonalState,
  ProfileView,
  PublishedEntryView,
  PublishedKindFilter,
  PublishedPage,
  PublishTask,
  ReaderIdentity,
  ReadingPage,
  ReadingQuery,
  ResourceState,
  SavedItem,
  SourceResolution,
  SourceView,
  StatsViewKey,
  StreamChange,
  SubscriptionIntent,
  SyncStatus,
  TopicView,
  WrappedBody,
} from '../datamodel/types'
import type { Did, EntryUrl, ObjId } from '../protocol/feed'

export type StoreDomain = 'reading' | 'candidates' | 'published' | 'comments' | 'saved' | 'sources' | 'prefs' | 'profile'

export type StoreStatus = 'loading' | 'ready' | 'error'

export type CommentTypeFilter = 'text' | 'like' | 'repost' | 'quote'

export type TagOverride = 'remove' | 'confirm' | 'to_assisted'

export interface CommentList {
  comments: CommentView[]
  stats: InteractionStats
  targetVersion: number
  versionCount: number
}

export interface EntryDebugView {
  entry: EntryUrl
  kind: PublishedEntryView['kind'] | 'follow'
  audience: AudienceSpec
  heads: { seq: number; state: string; current?: ObjId; at: number }[]
}

export interface StoreSettings {
  defaultAudience: AudienceSpec
  likeNoticeShown: boolean
}

export interface CollectorView {
  id: string
  name: string
}

export interface PreviewReaders {
  follower?: Did
  friend?: Did
}

export interface ReadingSummary {
  visible: number
  hiddenByRules: number
  hiddenByMute: number
}

export interface HomeStationStore {
  readonly id: string
  readonly owner: Did
  subscribe(listener: () => void): () => void
  getVersion(): number
  onDomains(domains: StoreDomain[], listener: () => void): () => void
  connect(): () => void
  watchCard(objId: ObjId, reader: ReaderIdentity): () => void

  peekStatus(): StoreStatus
  peekCard(objId: ObjId, reader?: ReaderIdentity): CardView | null
  peekIdentity(did: Did): IdentityView
  peekSettings(): StoreSettings
  peekGroups(): ContactGroup[]
  peekMuteRules(): MuteRule[]
  peekFilterRules(): FilterRule[]
  peekTopics(): TopicView[]
  peekSyncStatus(): SyncStatus
  peekHiddenSummary(): { hiddenByRules: number; hiddenByMute: number }
  peekReadingSummary(query: ReadingQuery): ReadingSummary
  peekEntries(): EntryDebugView[]
  peekCollector(): CollectorView | null
  peekPreviewReaders(): PreviewReaders
  peekMuteCandidates(): Did[]
  /** The signed-in user's home and zone-feed rights; null on portal pages and before boot. */
  peekHome(): HomeInfo | null

  listReading(query: ReadingQuery, cursor?: string | null): Promise<ReadingPage>
  listFollowedCandidates(cursor?: string | null, options?: { includeRead?: boolean }): Promise<CandidatePage>
  openCandidate(objId: ObjId): Promise<{ ok: boolean }>
  listPublished(owner: Did, options: { reader: ReaderIdentity; kind?: PublishedKindFilter; cursor?: string | null }): Promise<PublishedPage>
  listChanges(owner: Did, options: { reader: ReaderIdentity; after: number }): Promise<StreamChange[]>
  getItem(objId: ObjId, reader?: ReaderIdentity): Promise<CardView | null>
  getWrappedBody(objId: ObjId): Promise<WrappedBody>
  retryResources(objId: ObjId): Promise<ResourceState>
  listComments(objId: ObjId, options: { view: StatsViewKey; type: CommentTypeFilter }): Promise<CommentList>
  listSaved(kind: 'bookmark' | 'read_later'): Promise<SavedItem[]>
  getProfile(did: Did, reader: ReaderIdentity): Promise<ProfileView | null>

  setLike(objId: ObjId, on: boolean): Promise<PersonalState>
  setBookmark(objId: ObjId, options: { on: boolean; public: boolean }): Promise<PersonalState>
  setReadLater(objId: ObjId, on: boolean): Promise<PersonalState>
  setDislike(objId: ObjId, on: boolean): Promise<PersonalState>
  repost(objId: ObjId, on: boolean): Promise<PersonalState>
  quote(objId: ObjId, text: string, audience: AudienceSpec): Promise<PublishTask>
  comment(objId: ObjId, text: string): Promise<{ task: PublishTask; audience: AudienceSpec }>
  withdraw(entry: EntryUrl): Promise<void>
  editPost(entry: EntryUrl, text: string): Promise<ObjId | null>
  setAudience(entry: EntryUrl, audience: AudienceSpec): Promise<void>
  setZoneListing(entry: EntryUrl, listed: boolean): Promise<void>
  publish(input: PublishInput, key: string): Promise<PublishTask>
  retryPublish(key: string): Promise<PublishTask>
  retryDelivery(entry: EntryUrl): Promise<void>
  shareCapture(objId: ObjId): Promise<PublishTask>
  uploadAttachment(file: File, kind: AttachmentInput['kind']): Promise<ObjId>
  fetchLinkPreview(url: string): Promise<{ title: string; summary: string }>

  resolveSourceInput(kind: SourceResolution['inputKind'], text: string): Promise<SourceResolution>
  follow(resolution: SourceResolution): Promise<SourceView[]>
  unfollow(sourceId: string): Promise<'removed' | 'friend_basis'>
  pauseSource(sourceId: string, paused: boolean): Promise<void>
  listSources(): Promise<{ sources: SourceView[]; intents: SubscriptionIntent[] }>

  setMuteRule(rule: MuteRule, on: boolean): Promise<void>
  setFilterRule(rule: FilterRule): Promise<void>
  setTagOverride(objId: ObjId, tag: string, override: TagOverride | null): Promise<void>
  markLessLike(objId: ObjId): Promise<void>
  setFeatured(order: ObjId[]): Promise<void>
  setDefaultAudience(audience: AudienceSpec): Promise<void>
}
