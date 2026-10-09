import type {
  AudienceSpec,
  CandidateEntry,
  CandidatePage,
  CardView,
  CommentSourcePath,
  CommentView,
  ContactGroup,
  EffectiveTag,
  EmbeddedView,
  EntryState,
  FeedItemView,
  FilterRule,
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
  ReadingEntry,
  ReadingPage,
  ReadingQuery,
  ResourceState,
  SavedItem,
  SourceResolution,
  SourceView,
  StatsViewKey,
  StreamChange,
  SyncStatus,
  TopicView,
  WrappedBody,
} from '../datamodel/types'
import { EMPTY_PERSONAL } from '../datamodel/types'
import type { AttachmentInput, PublishInput } from '../datamodel/inputs'
import { commentTarget, type CommentType, type Did, type EntryUrl, type FeedContent, type FeedObject, type ObjId } from '../protocol/feed'
import type { CommentList, CommentTypeFilter, EntryDebugView, HomeStationStore, StoreDomain, TagOverride } from '../store/types'
import { feedEntry, fid, oid, OWNER_DID, reactionEntry, seedDatabase, type MockDb, type StoredEntry } from './data'

export interface StoreScenario {
  empty: boolean
  error: boolean
  publishFail: boolean
}

const PAGE_SIZE = 8
const PUBLISHED_PAGE = 10
const CANDIDATE_RETENTION_DAYS = 14

function hostOf(url: string) {
  try {
    return new URL(url).hostname.replace(/^www\./, '')
  } catch {
    return url
  }
}

function sleep(ms: number) {
  return new Promise<void>(resolve => window.setTimeout(resolve, ms))
}

export function parseScenario(search: string): StoreScenario {
  const values = new URLSearchParams(search).getAll('scenario').flatMap(value => value.split(','))
  return { empty: values.includes('empty'), error: values.includes('error'), publishFail: values.includes('publish-fail') }
}

let storeCounter = 0

export function createHomeStationStore(scenario: StoreScenario = { empty: false, error: false, publishFail: false }): HomeStationStore {
  const startedAt = Date.now()
  const db: MockDb = seedDatabase(startedAt, { empty: scenario.empty })
  const id = `hs-${++storeCounter}`
  const listeners = new Set<() => void>()
  const domainListeners = new Set<{ domains: StoreDomain[]; listener: () => void }>()
  const failedOnce = new Set<string>()
  const taskInputs = new Map<string, { input: PublishInput; shareOf?: ObjId }>()
  const captureShares = new Map<ObjId, ObjId>()
  let version = 0
  let runtimeCounter = 0
  let publishFailed = false

  const delay = () => sleep(300 + Math.random() * 500)
  const now = () => Date.now()

  function emit(...domains: StoreDomain[]) {
    version += 1
    for (const listener of listeners) listener()
    for (const entry of domainListeners) {
      if (entry.domains.some(domain => domains.includes(domain))) entry.listener()
    }
  }

  async function guarded<T>(name: string, run: () => T): Promise<T> {
    await delay()
    if (scenario.error && !failedOnce.has(name)) {
      failedOnce.add(name)
      throw new Error('mock_unavailable')
    }
    return run()
  }

  /* ── identity, audience ── */

  function identityView(did: Did): IdentityView {
    const identity = db.identities.get(did)
    if (!identity) return { did, name: did.replace(/^did:[a-z]+:/, ''), kind: 'person', hue: 210 }
    return { did, name: identity.name, kind: identity.kind, hue: identity.hue }
  }

  function readerDid(reader: ReaderIdentity): Did | null {
    if (reader.kind === 'owner') return db.owner
    if (reader.kind === 'did') return reader.did
    return null
  }

  function audienceAllows(publisher: Did, audience: AudienceSpec, did: Did | null) {
    if (audience.kind === 'public') return true
    if (!did) return false
    if (did === publisher) return true
    if (audience.kind === 'followers') return db.followers.get(publisher)?.has(did) ?? false
    if (audience.kind === 'friends') return db.friends.get(publisher)?.has(did) ?? false
    if (audience.kind === 'group') return publisher === db.owner && (db.groups.find(group => group.id === audience.groupId)?.members.includes(did) ?? false)
    return audience.dids.includes(did)
  }

  function canRead(objId: ObjId, reader: ReaderIdentity) {
    const stored = db.objects.get(objId)
    if (!stored) return false
    if (stored.privateCapture) return reader.kind === 'owner'
    const entry = stored.object.entry ? db.entries.get(stored.object.entry) : undefined
    if (!entry) return true
    return audienceAllows(entry.publisher, entry.audience, readerDid(reader))
  }

  function audienceOf(objId: ObjId): AudienceSpec {
    const entry = entryOfObject(objId)
    return entry?.audience ?? { kind: 'public' }
  }

  function entryOfObject(objId: ObjId) {
    const entry = db.objects.get(objId)?.object.entry
    return entry ? db.entries.get(entry) : undefined
  }

  function latestHead(entry: StoredEntry) {
    return entry.heads[entry.heads.length - 1]
  }

  function versionsOf(entry: StoredEntry): ObjId[] {
    const versions: ObjId[] = []
    for (const head of entry.heads) if (head.current && !versions.includes(head.current)) versions.push(head.current)
    return versions
  }

  function entryState(objId: ObjId): EntryState | undefined {
    const entry = entryOfObject(objId)
    if (!entry) return undefined
    const head = latestHead(entry)
    const versions = versionsOf(entry)
    return {
      entry: entry.entry,
      seq: head.seq,
      state: head.state,
      currentObjId: head.current,
      isLatest: head.state === 'active' && head.current === objId,
      version: Math.max(1, versions.indexOf(objId) + 1),
      versionCount: Math.max(1, versions.length),
    }
  }

  function isWithdrawn(objId: ObjId) {
    return entryState(objId)?.state === 'withdrawn'
  }

  /* ── item projection ── */

  function buildItem(objId: ObjId, reader: ReaderIdentity, depth = 0): FeedItemView | null {
    const stored = db.objects.get(objId)
    if (!stored) return null
    const object = stored.object
    const content = object.content
    const resolve = (part: { object: ObjId; alt?: string }) => ({ object: part.object, alt: part.alt, file: db.files.get(part.object) })
    const contentType: FeedItemView['contentType'] = object.kind === 'comment'
      ? (object.comment_type === 'like' || object.comment_type === 'bookmark' ? 'reaction' : 'comment')
      : content?.type ?? 'text'
    let embedded: EmbeddedView | undefined
    const target = object.kind === 'comment' ? commentTarget(object) : object.wraps && db.objects.has(object.wraps) ? object.wraps : undefined
    if (target) {
      const relation: EmbeddedView['relation'] = object.comment_type === 'repost' ? 'repost' : object.comment_type === 'quote' ? 'quote' : object.kind === 'comment' ? 'comment_on' : 'wraps'
      if (!db.objects.has(target)) embedded = { relation, visibility: 'missing' }
      else if (!canRead(target, reader)) embedded = { relation, visibility: 'not_visible' }
      else if (isWithdrawn(target)) embedded = { relation, visibility: 'withdrawn' }
      else embedded = { relation, visibility: 'visible', item: depth < 1 ? buildItem(target, reader, depth + 1) ?? undefined : undefined }
    }
    const audience = audienceOf(objId)
    return {
      objId,
      object,
      contentType,
      publisher: identityView(object.publisher),
      originalAuthor: object.source?.original_author,
      capturedFrom: object.source ? hostOf(object.source.original_url) : undefined,
      isCapture: !!object.source,
      isPrivateCapture: !!stored.privateCapture,
      verification: stored.verification,
      entry: entryState(objId),
      audience: { spec: audience, restricted: audience.kind !== 'public' },
      media: (content?.media ?? []).map(resolve),
      cover: content?.cover ? resolve({ object: content.cover }) : undefined,
      wrappedFile: object.wraps ? db.files.get(object.wraps) : undefined,
      embedded,
      category: object.publication_category,
      createdAt: object.iat * 1000,
      isOwn: object.publisher === db.owner,
    }
  }

  function authorTags(object: FeedObject): EffectiveTag[] {
    return (object.tags ?? []).map(tag => ({ tag, label: tag, source: 'author', status: 'declared', scope: 'whole_content' }))
  }

  function applyOverrides(objId: ObjId, tags: EffectiveTag[]): EffectiveTag[] {
    const overrides = db.tagOverrides.get(objId)
    if (!overrides) return tags
    const result: EffectiveTag[] = []
    for (const tag of tags) {
      const override = overrides.get(tag.tag)
      if (override === 'remove') continue
      if (override === 'confirm') result.push({ ...tag, source: 'user', status: 'confirmed' })
      else if (override === 'to_assisted') result.push({ tag: 'ai_assisted', label: 'ai_assisted', source: 'user', status: 'confirmed', scope: tag.scope, basis: tag.basis })
      else result.push(tag)
    }
    return result
  }

  function tagHit(tags: EffectiveTag[], condition: string, rule: FilterRule): boolean | undefined {
    const tag = tags.find(candidate => candidate.tag === condition)
    if (!tag) return undefined
    if (tag.source === 'model' && tag.status === 'inferred') return rule.acceptInferred && (tag.confidence ?? 0) >= rule.minConfidence
    return true
  }

  function rulesHit(tags: EffectiveTag[], classified: boolean): string[] {
    return db.filterRules
      .filter(rule => rule.enabled)
      .filter(rule => rule.conditions.every(condition => {
        const hit = tagHit(tags, condition, rule)
        if (hit === undefined) return !classified && rule.unknown === 'hide'
        return hit
      }))
      .map(rule => rule.id)
  }

  function topicsFor(tags: EffectiveTag[]) {
    return db.topics.filter(topic => topic.tags.some(tag => tags.some(effective => effective.tag === tag))).map(topic => topic.id)
  }

  function readingEntry(objId: ObjId): ReadingEntry | undefined {
    const stored = db.reading.find(entry => entry.objId === objId)
    const object = db.objects.get(objId)?.object
    if (!stored || !object) return undefined
    const effectiveTags = applyOverrides(objId, [...authorTags(object), ...stored.modelTags])
    return {
      objId,
      admittedAt: stored.admittedAt,
      reason: stored.reason,
      effectiveTags,
      resources: db.resources.get(objId) ?? 'local',
      filteredBy: rulesHit(effectiveTags, stored.classified),
      topics: topicsFor(effectiveTags),
    }
  }

  function candidateTags(objId: ObjId): EffectiveTag[] {
    const object = db.objects.get(objId)?.object
    return object ? applyOverrides(objId, authorTags(object)) : []
  }

  function isMuted(did: Did) {
    return db.muteRules.some(rule => rule.kind === 'person' ? rule.did === did : db.groups.find(group => group.id === rule.groupId)?.members.includes(did))
  }

  function personal(objId: ObjId): PersonalState {
    return db.personal.get(objId) ?? EMPTY_PERSONAL
  }

  function setPersonal(objId: ObjId, patch: Partial<PersonalState>) {
    db.personal.set(objId, { ...personal(objId), ...patch })
  }

  function stats(objId: ObjId, view: StatsViewKey): InteractionStats {
    const seenKeys = new Set<string>()
    const counts = { text: 0, like: 0, repost: 0, quote: 0, bookmark: 0 }
    for (const record of db.records.values()) {
      if (record.withdrawn) continue
      const object = db.objects.get(record.objId)?.object
      if (!object || commentTarget(object) !== objId || !object.comment_type) continue
      if (view === 'author' && !record.seen.author) continue
      if (view.startsWith('collector:') && !record.seen.collector) continue
      if (object.comment_type === 'like' || object.comment_type === 'repost' || object.comment_type === 'bookmark') {
        const key = `${object.publisher}|${object.comment_type}`
        if (seenKeys.has(key)) continue
        seenKeys.add(key)
      }
      counts[object.comment_type] += 1
    }
    const claimed = db.claimed.get(objId)
    return {
      view,
      asOf: db.lastFetchAt,
      sync: view.startsWith('collector:') ? 'partial' : 'synced',
      textComments: counts.text,
      likes: counts.like,
      reposts: counts.repost,
      quotes: counts.quote,
      ...(claimed ? { claimed: { likes: claimed.likes, source: identityView(db.objects.get(objId)!.object.publisher).name } } : {}),
    }
  }

  function card(objId: ObjId, reader: ReaderIdentity = { kind: 'owner' }): CardView | null {
    if (!canRead(objId, reader)) return null
    const item = buildItem(objId, reader)
    if (!item) return null
    const owner = reader.kind === 'owner'
    let repostBlockedReason: CardView['repostBlockedReason']
    if (item.contentType === 'reaction') repostBlockedReason = 'reaction'
    else if (item.isPrivateCapture) repostBlockedReason = 'private_capture'
    else if (item.entry?.state === 'withdrawn') repostBlockedReason = 'withdrawn'
    else if (item.audience.restricted) repostBlockedReason = 'restricted'
    return {
      item,
      reading: owner ? readingEntry(objId) : undefined,
      personal: owner ? personal(objId) : undefined,
      stats: stats(objId, 'local'),
      resources: db.resources.get(objId) ?? 'local',
      canRepost: !repostBlockedReason,
      repostBlockedReason,
      sharedAs: captureShares.get(objId),
    }
  }

  /* ── reading list ── */

  function matchesQuery(item: FeedItemView, reading: ReadingEntry, query: ReadingQuery) {
    const content = item.object.content
    switch (query.filter) {
      case 'following':
        if (reading.reason.code !== 'followed' && reading.reason.code !== 'friend') return false
        break
      case 'images':
        if (content?.type !== 'image') return false
        break
      case 'videos':
        if (content?.type !== 'video') return false
        break
      case 'longform':
        if (content?.type !== 'article') return false
        break
      case 'news':
        if (!item.isCapture) return false
        break
    }
    if (query.topicId && !reading.topics.includes(query.topicId)) return false
    const search = query.search.trim().toLowerCase()
    if (search) {
      const haystack = [content?.title, content?.text, content?.summary, item.publisher.name, item.originalAuthor, item.embedded?.item?.object.content?.text, item.embedded?.item?.object.content?.title]
      if (!haystack.some(value => value?.toLowerCase().includes(search))) return false
    }
    return true
  }

  function computeReading(query: ReadingQuery) {
    const visible: ObjId[] = []
    let hiddenByRules = 0
    let hiddenByMute = 0
    for (const stored of db.reading) {
      const item = buildItem(stored.objId, { kind: 'owner' })
      const reading = readingEntry(stored.objId)
      if (!item || !reading || !matchesQuery(item, reading, query)) continue
      if (isMuted(item.object.publisher)) {
        hiddenByMute += 1
        continue
      }
      if (reading.filteredBy.length > 0 && !query.showFiltered) {
        hiddenByRules += 1
        continue
      }
      visible.push(stored.objId)
    }
    return { visible, hiddenByRules, hiddenByMute }
  }

  function pageAfter<T>(items: T[], cursor: string | null | undefined, keyOf: (item: T) => string, size: number) {
    let start = 0
    if (cursor) {
      const index = items.findIndex(item => keyOf(item) === cursor)
      start = index >= 0 ? index + 1 : items.length
    }
    const slice = items.slice(start, start + size)
    return { slice, nextCursor: start + size < items.length && slice.length > 0 ? keyOf(slice[slice.length - 1]) : null }
  }

  /* ── candidates ── */

  function visibleCandidates(includeRead: boolean) {
    const result: CandidateEntry[] = []
    let readCount = 0
    for (const candidate of db.candidates) {
      const object = db.objects.get(candidate.objId)?.object
      if (!object || isWithdrawn(candidate.objId) || !canRead(candidate.objId, { kind: 'owner' }) || isMuted(object.publisher)) continue
      if (rulesHit(candidateTags(candidate.objId), candidate.selection !== 'unscreened').length > 0) continue
      if (candidate.firstAdmittedAt !== null) continue
      if (candidate.openedAt !== null) {
        readCount += 1
        if (!includeRead) continue
      }
      result.push({ ...candidate, resources: db.resources.get(candidate.objId) ?? 'reachable' })
    }
    return { entries: result, readCount }
  }

  /* ── head bookkeeping ── */

  function appendChange(entry: StoredEntry, kind: StreamChange['kind'] = 'head') {
    const head = latestHead(entry)
    db.changes.push({ cursor: db.changes.length + 1, entry: entry.entry, seq: head.seq, state: head.state, current: head.current, kind, at: now() })
  }

  function signHead(entry: StoredEntry, state: 'active' | 'withdrawn', current?: ObjId) {
    entry.heads.push({ kind: 'feed_head', entry: entry.entry, seq: latestHead(entry).seq + 1, state, ...(state === 'active' && current ? { current } : {}), updated_at_ms: now() })
    appendChange(entry)
  }

  function createEntry(entryUrl: EntryUrl, object: FeedObject, audience: AudienceSpec, kind: StoredEntry['kind']): { objId: ObjId; entry: StoredEntry } {
    runtimeCounter += 1
    const objId = oid(`runtime-${id}-${runtimeCounter}-${entryUrl}`)
    db.objects.set(objId, { objId, object: { ...object, entry: entryUrl }, verification: 'verified' })
    const entry: StoredEntry = { entry: entryUrl, publisher: db.owner, audience, kind, publishedAt: now(), heads: [{ kind: 'feed_head', entry: entryUrl, seq: 1, state: 'active', current: objId, updated_at_ms: now() }] }
    db.entries.set(entryUrl, entry)
    appendChange(entry)
    return { objId, entry }
  }

  function newObjectVersion(entry: StoredEntry, object: FeedObject): ObjId {
    runtimeCounter += 1
    const objId = oid(`runtime-${id}-${runtimeCounter}-${entry.entry}`)
    db.objects.set(objId, { objId, object: { ...object, entry: entry.entry, iat: Math.floor(now() / 1000) }, verification: 'verified' })
    signHead(entry, 'active', objId)
    return objId
  }

  function nextKey(prefix: string) {
    runtimeCounter += 1
    return `${prefix}-${(db.entries.size + runtimeCounter).toString(36)}`
  }

  function deliveryTotal(audience: AudienceSpec) {
    switch (audience.kind) {
      case 'public':
        return 5
      case 'followers':
        return 4
      case 'friends':
        return db.friends.get(db.owner)?.size ?? 1
      case 'group':
        return db.groups.find(group => group.id === audience.groupId)?.members.length ?? 1
      case 'dids':
        return audience.dids.length
    }
  }

  function runDelivery(task: PublishTask) {
    const tick = () => {
      if (!task.delivery || task.delivery.state !== 'delivering') return
      task.delivery = { ...task.delivery, delivered: task.delivery.delivered + 1 }
      if (task.delivery.delivered >= task.delivery.total) task.delivery = { ...task.delivery, state: 'delivered' }
      emit('published')
      if (task.delivery.state === 'delivering') window.setTimeout(tick, 600)
    }
    window.setTimeout(tick, 600)
  }

  function interactionRecord(objId: ObjId, audience: AudienceSpec, withdrawn = false) {
    const restricted = audience.kind !== 'public'
    db.records.set(objId, { objId, seen: { author: true, collector: !restricted, push: false }, withdrawn })
  }

  function reactionToggle(target: ObjId, type: Extract<CommentType, 'like' | 'bookmark' | 'repost'>, on: boolean) {
    const targetEntry = entryOfObject(target)
    const restricted = (targetEntry?.audience.kind ?? 'public') !== 'public'
    const audience: AudienceSpec = restricted && targetEntry ? { kind: 'dids', dids: [targetEntry.publisher] } : { kind: 'public' }
    const entryUrl = reactionEntry(db.owner, target, type)
    let entry = db.entries.get(entryUrl)
    let objectId: ObjId | undefined
    if (!entry) {
      if (!on) return { entryUrl, seq: 0, restricted, audience }
      const object: FeedObject = {
        kind: 'comment',
        comment_type: type,
        publisher: db.owner,
        iat: Math.floor(now() / 1000),
        ...(type === 'repost' ? { wraps: target } : { references: [{ relation: 'comment_on' as const, object_id: target }] }),
      }
      const created = createEntry(entryUrl, object, audience, type)
      entry = created.entry
      objectId = created.objId
    } else {
      objectId = versionsOf(entry)[0]
      signHead(entry, on ? 'active' : 'withdrawn', objectId)
    }
    if (objectId) interactionRecord(objectId, audience, !on)
    return { entryUrl, seq: latestHead(entry).seq, restricted, audience }
  }

  function settleDelivery(objId: ObjId, key: 'like' | 'bookmark' | 'repost') {
    const current = personal(objId)[key]
    if (current.delivery === 'delivering') setPersonal(objId, { [key]: { ...current, delivery: 'delivered' } })
  }

  function contentFromInput(input: PublishInput): Pick<FeedObject, 'content' | 'wraps' | 'link'> {
    const text = input.text || undefined
    if (input.link) {
      return { content: { type: 'link', title: input.link.title, summary: input.link.summary || undefined, text }, link: input.link.url }
    }
    const video = input.attachments.find(attachment => attachment.kind === 'video')
    if (video?.object) {
      return { content: { type: 'video', title: text?.slice(0, 80), text }, wraps: video.object }
    }
    const audio = input.attachments.filter(attachment => attachment.kind === 'audio')
    if (audio.length > 0) return { content: { type: 'audio', text, media: audio.map(attachment => ({ object: attachment.object! })) } }
    const images = input.attachments.filter(attachment => attachment.kind === 'image')
    if (images.length > 0) return { content: { type: 'image', text, media: images.map(attachment => ({ object: attachment.object!, alt: attachment.name })) } }
    return { content: { type: 'text', text: text ?? '' } }
  }

  async function runPublish(key: string): Promise<PublishTask> {
    const stored = taskInputs.get(key)!
    const task = db.tasks.get(key)!
    task.stage = 'uploading'
    task.error = undefined
    emit('published')
    await delay()
    if (scenario.publishFail && !publishFailed) {
      publishFailed = true
      task.stage = 'failed'
      task.error = 'upload_failed'
      emit('published')
      return { ...task }
    }
    const input = stored.input
    let object: FeedObject
    if (stored.shareOf) {
      const capture = db.objects.get(stored.shareOf)!.object
      object = { kind: 'post', publisher: db.owner, iat: Math.floor(now() / 1000), content: capture.content, ...(capture.wraps ? { wraps: capture.wraps } : {}), ...(capture.link ? { link: capture.link } : {}), source: capture.source, tags: capture.tags }
    } else {
      object = { kind: 'post', publisher: db.owner, iat: Math.floor(now() / 1000), ...contentFromInput(input) }
    }
    const { objId, entry } = createEntry(feedEntry(db.owner, nextKey(stored.shareOf ? 'clip' : 'post')), object, input.audience, 'post')
    if (stored.shareOf) captureShares.set(stored.shareOf, objId)
    task.stage = 'published'
    task.entry = entry.entry
    task.objId = objId
    task.delivery = { state: 'delivering', delivered: 0, total: deliveryTotal(input.audience) }
    entry.task = task
    emit('published', 'profile', 'reading')
    runDelivery(task)
    return { ...task }
  }

  function startTask(key: string, input: PublishInput, shareOf?: ObjId): Promise<PublishTask> {
    const existing = db.tasks.get(key)
    if (existing && existing.stage !== 'failed') return Promise.resolve({ ...existing })
    taskInputs.set(key, { input, shareOf })
    if (!existing) db.tasks.set(key, { key, stage: 'uploading', createdAt: now() })
    return runPublish(key)
  }

  function publishedEntries(owner: Did, reader: ReaderIdentity, kind: PublishedKindFilter): PublishedEntryView[] {
    const did = readerDid(reader)
    const isOwner = reader.kind === 'owner' && owner === db.owner
    const rows: PublishedEntryView[] = []
    for (const entry of db.entries.values()) {
      if (entry.publisher !== owner) continue
      if (!isOwner && !audienceAllows(owner, entry.audience, did)) continue
      const head = latestHead(entry)
      if (!isOwner && head.state === 'withdrawn') continue
      const objId = head.current ?? versionsOf(entry).at(-1)
      const object = objId ? db.objects.get(objId)?.object : undefined
      const category = object?.publication_category
      const matches = kind === 'all'
        || (kind === 'posts' && entry.kind === 'post' && !category)
        || (kind === 'comments' && entry.kind === 'comment')
        || (kind === 'reposts' && (entry.kind === 'repost' || entry.kind === 'quote'))
        || (kind === 'reactions' && (entry.kind === 'like' || entry.kind === 'bookmark'))
        || (kind === 'work' && category === 'work')
        || (kind === 'product' && category === 'product')
      if (!matches) continue
      if (kind === 'all' && !isOwner && (entry.kind === 'like' || entry.kind === 'bookmark')) continue
      rows.push({
        entry: entry.entry,
        objId,
        head: objId ? entryState(objId)! : { entry: entry.entry, seq: head.seq, state: head.state, isLatest: false, version: 1, versionCount: 1 },
        audience: { spec: entry.audience, restricted: entry.audience.kind !== 'public' },
        task: isOwner ? entry.task : undefined,
        kind: entry.kind,
        publishedAt: entry.publishedAt,
      })
    }
    return rows.sort((left, right) => right.publishedAt - left.publishedAt)
  }

  function feedKinds(owner: Did, reader: ReaderIdentity) {
    return publishedEntries(owner, reader, 'all').filter(row => row.head.state !== 'withdrawn').filter(row => (row.kind === 'post' && !(row.objId && db.objects.get(row.objId)?.object.publication_category)) || row.kind === 'comment' || row.kind === 'repost' || row.kind === 'quote')
  }

  for (const entry of [...db.entries.values()].filter(entry => entry.publisher === db.owner).sort((left, right) => left.publishedAt - right.publishedAt)) appendChange(entry)

  const store = {
    id,
    owner: OWNER_DID as Did,
    subscribe(listener: () => void) {
      listeners.add(listener)
      return () => {
        listeners.delete(listener)
      }
    },
    getVersion: () => version,
    onDomains(domains: StoreDomain[], listener: () => void) {
      const entry = { domains, listener }
      domainListeners.add(entry)
      return () => {
        domainListeners.delete(entry)
      }
    },
    connect: () => () => {},
    watchCard: () => () => {},
    peekStatus: () => 'ready' as const,
    peekCollector: () => db.collector,
    peekPreviewReaders: () => ({ follower: 'did:bns:sarah', friend: 'did:bns:bob' }),
    peekMuteCandidates: (): Did[] => [...new Set([...(db.friends.get(db.owner) ?? []), 'did:bns:sarah', 'did:bns:david'])],

    peekCard: card,
    peekIdentity: identityView,
    peekPersonal: personal,
    peekSettings: () => ({ ...db.settings }),
    peekGroups: (): ContactGroup[] => db.groups,
    peekMuteRules: (): MuteRule[] => db.muteRules,
    peekFilterRules: (): FilterRule[] => db.filterRules,
    peekTopics(): TopicView[] {
      const weekAgo = db.now - 7 * 24 * 3600_000
      return db.topics.map(topic => ({
        ...topic,
        recentCount: db.reading.filter(entry => {
          if (entry.admittedAt < weekAgo) return false
          const reading = readingEntry(entry.objId)
          const object = db.objects.get(entry.objId)?.object
          return !!reading && !!object && reading.topics.includes(topic.id) && reading.filteredBy.length === 0 && !isMuted(object.publisher)
        }).length,
      }))
    },
    peekSyncStatus(): SyncStatus {
      const candidates = visibleCandidates(false).entries
      const preparing = candidates.filter(candidate => candidate.selection === 'preparing' || candidate.resources === 'preparing').length
        + db.reading.filter(entry => db.resources.get(entry.objId) === 'preparing').length
      return {
        candidates: candidates.length,
        preparing,
        lastFetchAt: db.lastFetchAt,
        sources: db.sources.length,
        failingSources: db.sources.filter(source => source.lastError && !source.paused).length,
      }
    },
    peekHiddenSummary() {
      const { hiddenByRules, hiddenByMute } = computeReading({ filter: 'all', topicId: null, search: '', showFiltered: false })
      return { hiddenByRules, hiddenByMute }
    },
    peekReadingSummary(query: ReadingQuery) {
      const { visible, hiddenByRules, hiddenByMute } = computeReading({ ...query, showFiltered: false })
      return { visible: visible.length, hiddenByRules, hiddenByMute }
    },
    peekEntries(): EntryDebugView[] {
      return [...db.entries.values()]
        .filter(entry => entry.publisher === db.owner)
        .sort((left, right) => latestHead(right).updated_at_ms - latestHead(left).updated_at_ms)
        .map(entry => ({ entry: entry.entry, kind: entry.kind, audience: entry.audience, heads: entry.heads.map(head => ({ seq: head.seq, state: head.state, current: head.current, at: head.updated_at_ms })) }))
    },
    peekTask: (key: string) => db.tasks.get(key),
    peekProfileName: (did: Did) => identityView(did).name,

    listReading(query: ReadingQuery, cursor?: string | null): Promise<ReadingPage> {
      return guarded('reading', () => {
        const { visible, hiddenByRules, hiddenByMute } = computeReading(query)
        const { slice, nextCursor } = pageAfter(visible, cursor, objId => objId, PAGE_SIZE)
        return { objIds: slice, nextCursor, hiddenByRules, hiddenByMute, total: visible.length }
      })
    },

    listFollowedCandidates(cursor?: string | null, options: { includeRead?: boolean } = {}): Promise<CandidatePage> {
      return guarded('candidates', () => {
        const { entries, readCount } = visibleCandidates(!!options.includeRead)
        const { slice, nextCursor } = pageAfter(entries, cursor, entry => entry.objId, PAGE_SIZE)
        return { entries: slice, nextCursor, readCount, retentionDays: CANDIDATE_RETENTION_DAYS, lastFetchAt: db.lastFetchAt }
      })
    },

    async openCandidate(objId: ObjId): Promise<{ ok: boolean }> {
      const candidate = db.candidates.find(entry => entry.objId === objId)
      if (!candidate) return { ok: false }
      candidate.selection = 'preparing'
      emit('candidates')
      await delay()
      await delay()
      if (candidate.failOpenTimes > 0) {
        candidate.failOpenTimes -= 1
        candidate.selection = 'unscreened'
        db.resources.set(objId, 'unavailable')
        emit('candidates')
        return { ok: false }
      }
      db.resources.set(objId, 'local')
      candidate.openedAt = now()
      emit('candidates')
      return { ok: true }
    },

    listPublished(owner: Did, options: { reader: ReaderIdentity; kind?: PublishedKindFilter; cursor?: string | null }): Promise<PublishedPage> {
      return guarded('published', () => {
        const rows = options.kind === undefined ? feedKinds(owner, options.reader) : publishedEntries(owner, options.reader, options.kind)
        const { slice, nextCursor } = pageAfter(rows, options.cursor, row => row.entry, PUBLISHED_PAGE)
        return { entries: slice, nextCursor, changeCursor: db.changes.length }
      })
    },

    async listChanges(owner: Did, options: { reader: ReaderIdentity; after: number }): Promise<StreamChange[]> {
      await delay()
      const did = readerDid(options.reader)
      return db.changes.filter(change => {
        if (change.cursor <= options.after) return false
        const entry = db.entries.get(change.entry)
        return !!entry && entry.publisher === owner && (options.reader.kind === 'owner' || audienceAllows(owner, entry.audience, did))
      })
    },

    async getItem(objId: ObjId, reader: ReaderIdentity = { kind: 'owner' }): Promise<CardView | null> {
      await delay()
      if (!canRead(objId, reader)) return null
      return card(objId, reader)
    },

    async getWrappedBody(objId: ObjId): Promise<WrappedBody> {
      await delay()
      const wraps = db.objects.get(objId)?.object.wraps
      const body = wraps ? db.bodies.get(wraps) : undefined
      const file = wraps ? db.files.get(wraps) : undefined
      if (!body || !file) return { state: 'unavailable', reason: 'no_known_source' }
      if (body.state === 'preparing' && body.pendingReads > 0) {
        body.pendingReads -= 1
        return { state: 'preparing' }
      }
      if (body.state === 'unavailable') return { state: 'unavailable', reason: 'no_known_source' }
      body.state = 'ready'
      return { state: 'ready', markdown: body.markdown, file }
    },

    async retryResources(objId: ObjId): Promise<ResourceState> {
      db.resources.set(objId, 'preparing')
      emit('reading', 'candidates')
      await delay()
      await delay()
      const next = db.resourceRetry.get(objId) ?? 'local'
      db.resources.set(objId, next)
      emit('reading', 'candidates')
      return next
    },

    listComments(objId: ObjId, options: { view: StatsViewKey; type: CommentTypeFilter }): Promise<CommentList> {
      return guarded('comments', () => {
        const state = entryState(objId)
        const entry = entryOfObject(objId)
        const versions = entry ? versionsOf(entry) : [objId]
        const ownIndex = Math.max(0, versions.indexOf(objId))
        const related = new Set(entry ? versions.slice(0, ownIndex + 1) : [objId])
        const comments: CommentView[] = []
        const seenKeys = new Set<string>()
        const records = [...db.records.values()]
          .map(record => ({ record, object: db.objects.get(record.objId)?.object }))
          .filter(({ object }) => !!object)
          .sort((left, right) => right.object!.iat - left.object!.iat)
        for (const { record, object } of records) {
          if (!object || record.withdrawn || object.comment_type !== options.type) continue
          const target = commentTarget(object)
          if (!target || !related.has(target)) continue
          if (options.view === 'author' && !record.seen.author) continue
          if (options.view.startsWith('collector:') && !record.seen.collector) continue
          if (options.type !== 'text' && options.type !== 'quote') {
            const key = `${object.publisher}|${target}`
            if (seenKeys.has(key)) continue
            seenKeys.add(key)
          }
          const item = buildItem(record.objId, { kind: 'owner' })
          if (!item) continue
          const sourcePaths: CommentSourcePath[] = []
          if (record.seen.author) sourcePaths.push({ kind: 'author_list', label: identityView(db.objects.get(objId)!.object.publisher).name })
          if (record.seen.collector) sourcePaths.push({ kind: 'collector', label: db.collector.name })
          if (record.seen.push) sourcePaths.push({ kind: 'push', label: item.publisher.name })
          if (object.publisher === db.owner) sourcePaths.push({ kind: 'participant', label: identityView(db.owner).name })
          comments.push({
            objId: record.objId,
            item,
            commentType: object.comment_type!,
            targetObjId: target,
            onOldVersion: target !== objId,
            targetVersion: versions.indexOf(target) + 1 || 1,
            sourcePaths,
            listedByAuthor: record.seen.author,
            listedByCollector: record.seen.collector,
          })
        }
        return { comments, stats: stats(objId, options.view), targetVersion: state?.version ?? 1, versionCount: state?.versionCount ?? 1 }
      })
    },

    async listSaved(kind: 'bookmark' | 'read_later'): Promise<SavedItem[]> {
      await delay()
      const rows: SavedItem[] = []
      for (const [objId, saved] of db.savedAt) {
        const state = personal(objId)
        const at = kind === 'bookmark' ? saved.bookmark : saved.readLater
        if (!at || (kind === 'bookmark' ? !state.bookmark.on : !state.readLater)) continue
        const entry = entryState(objId)
        rows.push({ objId, savedAt: at, visibility: kind === 'bookmark' ? state.bookmark.visibility : 'private', targetState: entry?.state === 'withdrawn' ? 'withdrawn' : entry && !entry.isLatest ? 'updated' : 'active' })
      }
      return rows.sort((left, right) => right.savedAt - left.savedAt)
    },

    async getProfile(did: Did, reader: ReaderIdentity): Promise<ProfileView | null> {
      await delay()
      const identity = db.identities.get(did)
      if (!identity) return null
      const viewer: ReaderIdentity = reader.kind === 'owner' && did !== db.owner ? { kind: 'did', did: db.owner } : reader
      return {
        did,
        name: identity.name,
        bio: identity.bio,
        hue: identity.hue,
        followers: db.followers.get(did)?.size ?? 0,
        following: identity.following,
        posts: feedKinds(did, viewer).length,
        featured: (db.featured.get(did) ?? []).filter(objId => canRead(objId, viewer) && !isWithdrawn(objId)),
      }
    },

    async setLike(objId: ObjId, on: boolean): Promise<PersonalState> {
      const { entryUrl, seq, restricted } = reactionToggle(objId, 'like', on)
      setPersonal(objId, { like: { on, visibility: restricted ? 'author_only' : 'public', delivery: 'delivering', entry: entryUrl, seq } })
      if (on && !restricted) db.settings.likeNoticeShown = true
      emit('published', 'comments')
      await delay()
      settleDelivery(objId, 'like')
      emit('published')
      return personal(objId)
    },

    async setBookmark(objId: ObjId, options: { on: boolean; public: boolean }): Promise<PersonalState> {
      const current = personal(objId).bookmark
      const wasPublic = current.on && current.visibility !== 'private'
      const makePublic = options.on && options.public
      let entry = current.entry
      let seq = current.seq
      let visibility: PersonalState['bookmark']['visibility'] = 'private'
      if (makePublic || wasPublic) {
        const toggled = reactionToggle(objId, 'bookmark', makePublic)
        entry = toggled.entryUrl
        seq = toggled.seq
        if (makePublic) visibility = toggled.restricted ? 'author_only' : 'public'
      }
      setPersonal(objId, { bookmark: { on: options.on, visibility, delivery: makePublic || wasPublic ? 'delivering' : 'none', entry, seq } })
      const saved = db.savedAt.get(objId) ?? {}
      db.savedAt.set(objId, { ...saved, bookmark: options.on ? saved.bookmark ?? now() : undefined })
      emit('saved', 'published', 'comments')
      await delay()
      settleDelivery(objId, 'bookmark')
      emit('saved', 'published')
      return personal(objId)
    },

    async setReadLater(objId: ObjId, on: boolean): Promise<PersonalState> {
      setPersonal(objId, { readLater: on })
      const saved = db.savedAt.get(objId) ?? {}
      db.savedAt.set(objId, { ...saved, readLater: on ? now() : undefined })
      emit('saved')
      await delay()
      return personal(objId)
    },

    // 待确认（TODO §13）：点踩语义未定，临时只作本地反馈，不发表、不计入任何统计
    async setDislike(objId: ObjId, on: boolean): Promise<PersonalState> {
      setPersonal(objId, { dislike: on })
      emit('saved')
      await delay()
      return personal(objId)
    },

    async repost(objId: ObjId, on: boolean): Promise<PersonalState> {
      const view = card(objId)
      if (on && !view?.canRepost) throw new Error(`repost_blocked:${view?.repostBlockedReason}`)
      const { entryUrl, seq } = reactionToggle(objId, 'repost', on)
      setPersonal(objId, { repost: { on, visibility: 'public', delivery: 'delivering', entry: entryUrl, seq } })
      emit('published', 'comments', 'profile')
      await delay()
      settleDelivery(objId, 'repost')
      emit('published')
      return personal(objId)
    },

    async quote(objId: ObjId, text: string, audience: AudienceSpec): Promise<PublishTask> {
      const view = card(objId)
      if (!view?.canRepost) throw new Error(`repost_blocked:${view?.repostBlockedReason}`)
      await delay()
      const object: FeedObject = { kind: 'comment', comment_type: 'quote', publisher: db.owner, iat: Math.floor(now() / 1000), wraps: objId, content: { type: 'text', text } }
      const { objId: created, entry } = createEntry(feedEntry(db.owner, nextKey('q')), object, audience, 'quote')
      interactionRecord(created, audience)
      const task: PublishTask = { key: `quote-${created}`, stage: 'published', entry: entry.entry, objId: created, delivery: { state: 'delivering', delivered: 0, total: deliveryTotal(audience) }, createdAt: now() }
      entry.task = task
      db.tasks.set(task.key, task)
      emit('published', 'comments', 'profile')
      runDelivery(task)
      return task
    },

    async comment(objId: ObjId, text: string): Promise<{ task: PublishTask; audience: AudienceSpec }> {
      const target = entryOfObject(objId)
      const restricted = !!target && target.audience.kind !== 'public'
      const audience: AudienceSpec = restricted ? { kind: 'dids', dids: [target!.publisher] } : db.settings.defaultAudience
      await delay()
      const object: FeedObject = { kind: 'comment', comment_type: 'text', publisher: db.owner, iat: Math.floor(now() / 1000), content: { type: 'text', text }, references: [{ relation: 'comment_on', object_id: objId }] }
      const { objId: created, entry } = createEntry(feedEntry(db.owner, nextKey('c')), object, audience, 'comment')
      db.records.set(created, { objId: created, seen: { author: false, collector: !restricted && audience.kind === 'public', push: false }, withdrawn: false })
      const task: PublishTask = { key: `comment-${created}`, stage: 'published', entry: entry.entry, objId: created, delivery: { state: 'delivering', delivered: 0, total: restricted ? 1 : deliveryTotal(audience) }, createdAt: now() }
      entry.task = task
      db.tasks.set(task.key, task)
      emit('published', 'comments', 'profile')
      runDelivery(task)
      window.setTimeout(() => {
        const record = db.records.get(created)
        if (record) record.seen.author = true
        emit('comments')
      }, 900)
      return { task, audience }
    },

    async withdraw(entryUrl: EntryUrl): Promise<void> {
      await delay()
      const entry = db.entries.get(entryUrl)
      if (!entry || entry.publisher !== db.owner || latestHead(entry).state === 'withdrawn') return
      signHead(entry, 'withdrawn')
      for (const objId of versionsOf(entry)) {
        const record = db.records.get(objId)
        if (record) record.withdrawn = true
        const object = db.objects.get(objId)?.object
        const target = object ? commentTarget(object) : undefined
        if (target && object?.comment_type && object.comment_type !== 'text' && object.comment_type !== 'quote') {
          const key = object.comment_type as 'like' | 'bookmark' | 'repost'
          const state = personal(target)[key]
          if (key === 'bookmark') setPersonal(target, { bookmark: { ...state, visibility: 'private', delivery: 'none' } })
          else setPersonal(target, { [key]: { ...state, on: false, delivery: 'delivered', seq: latestHead(entry).seq } })
        }
      }
      emit('published', 'profile', 'comments', 'reading', 'saved')
    },

    publish(input: PublishInput, key: string): Promise<PublishTask> {
      return startTask(key, input)
    },

    retryPublish(key: string): Promise<PublishTask> {
      const stored = taskInputs.get(key)
      if (!stored) return Promise.reject(new Error('unknown_task'))
      return startTask(key, stored.input, stored.shareOf)
    },

    async retryDelivery(entryUrl: EntryUrl): Promise<void> {
      const entry = db.entries.get(entryUrl)
      const task = entry?.task
      if (!task?.delivery || task.delivery.state !== 'partially_failed') return
      task.delivery = { ...task.delivery, state: 'delivering' }
      emit('published')
      await delay()
      runDelivery(task)
    },

    async editPost(entryUrl: EntryUrl, text: string): Promise<ObjId | null> {
      await delay()
      const entry = db.entries.get(entryUrl)
      if (!entry || entry.publisher !== db.owner || latestHead(entry).state === 'withdrawn') return null
      const current = latestHead(entry).current
      const previous = current ? db.objects.get(current)?.object : undefined
      if (!previous || !current) return null
      const content: FeedContent | undefined = previous.content ? { ...previous.content, text } : { type: 'text', text }
      const objId = newObjectVersion(entry, { ...previous, content, base_on: current })
      emit('published', 'profile', 'comments', 'reading')
      return objId
    },

    async setAudience(entryUrl: EntryUrl, audience: AudienceSpec): Promise<void> {
      await delay()
      const entry = db.entries.get(entryUrl)
      if (!entry || entry.publisher !== db.owner) return
      entry.audience = audience
      appendChange(entry, 'audience')
      emit('published', 'profile')
    },

    shareCapture(objId: ObjId): Promise<PublishTask> {
      return startTask(`share-${objId}`, { text: '', attachments: [], link: null, audience: db.settings.defaultAudience }, objId)
    },

    async uploadAttachment(file: File, kind: AttachmentInput['kind']): Promise<ObjId> {
      await delay()
      runtimeCounter += 1
      const objId = fid(`upload-${id}-${runtimeCounter}-${file.name}`)
      db.files.set(objId, { kind: 'file', name: file.name, meta: { mime: file.type || (kind === 'image' ? 'image/png' : kind === 'video' ? 'video/mp4' : 'audio/mp4'), size: file.size, ...(kind === 'image' ? { width: 1600, height: 1200 } : { duration_ms: 42_000 }) } })
      return objId
    },

    async fetchLinkPreview(url: string): Promise<{ title: string; summary: string }> {
      await delay()
      const host = hostOf(url)
      const path = url.split('/').filter(Boolean).pop() ?? host
      const words = path.replace(/[-_]+/g, ' ').replace(/\.[a-z]+$/i, '').trim()
      return { title: words ? words.charAt(0).toUpperCase() + words.slice(1) : host, summary: host }
    },

    async resolveSourceInput(kind: 'follow' | 'url' | 'natural', text: string): Promise<SourceResolution> {
      await delay()
      const query = text.trim()
      if (kind === 'follow') {
        const lower = query.toLowerCase()
        const matches = [...db.identities.values()].filter(identity => identity.did !== db.owner && identity.kind !== 'self' && (identity.did.toLowerCase() === lower || identity.name.toLowerCase().includes(lower))).slice(0, 4)
        const candidates: SourceView[] = matches.map(identity => existingSource(identity.did) ?? { id: `src-${identity.did}`, name: identity.name, kind: 'person', did: identity.did, description: identity.bio, basis: [], notify: 'pending', paused: false, credibility: 'verified_did', updateMode: 'Home feed changes + push' })
        if (candidates.length === 0 && /^did:[a-z0-9]+:.+/i.test(query)) candidates.push({ id: `src-${query}`, name: query, kind: 'person', did: query, basis: [], notify: 'pending', paused: false, credibility: 'verified_did', updateMode: 'Home feed changes + push' })
        return { inputKind: kind, input: query, candidates, notifyHint: 'pending' }
      }
      if (kind === 'url') {
        const host = hostOf(query)
        const isHomeStation = host.endsWith('.buckyos.io')
        const isFeed = /rss|feed|\.xml/i.test(query)
        const known = ['techcrunch.com', 'sfchronicle.com', 'sfgate.com', 'news.ycombinator.com', 'theverge.com', 'arstechnica.com'].some(site => host.endsWith(site))
        const candidate: SourceView = isHomeStation
          ? { id: `src-${host}`, name: host.replace('.buckyos.io', ''), kind: 'person', did: `did:bns:${host.split('.')[0]}`, url: query, basis: [], notify: 'pending', paused: false, credibility: 'verified_did', updateMode: 'Home feed changes + push' }
          : { id: `src-${host}`, name: host, kind: isFeed ? 'rss' : 'website', url: query, basis: [], notify: 'unsupported', paused: false, credibility: known ? 'known_site' : 'unknown', updateMode: isFeed ? 'RSS poll every 30 min' : 'Spider crawl every hour' }
        return { inputKind: kind, input: query, candidates: [existingSource(candidate.did ?? '') ?? candidate], notifyHint: candidate.notify }
      }
      const words = query.toLowerCase()
      const mapped: SourceView[] = words.includes('second') || words.includes('二手') || words.includes('used')
        ? [
            { id: `src-nl-${runtimeCounter}-craigslist`, name: 'Craigslist SF · for sale', kind: 'website', url: 'https://sfbay.craigslist.org/search/sss', basis: [], notify: 'unsupported', paused: false, credibility: 'known_site', updateMode: 'Agent-maintained crawl' },
            { id: `src-nl-${runtimeCounter}-nextdoor`, name: 'Nextdoor · Free & for sale', kind: 'channel', url: 'https://nextdoor.com', basis: [], notify: 'unsupported', paused: false, credibility: 'unknown', updateMode: 'Agent-maintained crawl' },
          ]
        : [
            { id: `src-nl-${runtimeCounter}-news`, name: `News search · ${query.slice(0, 32)}`, kind: 'rss', url: `https://news.example/search?q=${encodeURIComponent(query)}`, basis: [], notify: 'unsupported', paused: false, credibility: 'unknown', updateMode: 'Agent-maintained search' },
            { id: `src-nl-${runtimeCounter}-social`, name: `Public posts · ${query.slice(0, 32)}`, kind: 'channel', basis: [], notify: 'unsupported', paused: false, credibility: 'unknown', updateMode: 'Agent-maintained crawl' },
            { id: `src-nl-${runtimeCounter}-hs`, name: 'HomeStation collectors · Open Index', kind: 'website', url: 'https://open-index.example', basis: [], notify: 'unsupported', paused: false, credibility: 'known_site', updateMode: 'Collector query' },
          ]
      runtimeCounter += 1
      return { inputKind: kind, input: query, candidates: mapped, notifyHint: 'unsupported', intentText: query }
    },

    async follow(resolution: SourceResolution): Promise<SourceView[]> {
      await delay()
      const added: SourceView[] = []
      let intentId: string | undefined
      if (resolution.inputKind === 'natural') {
        intentId = `intent-${db.intents.length + 1}-${runtimeCounter}`
        db.intents.push({ id: intentId, text: resolution.intentText ?? resolution.input, sourceIds: resolution.candidates.map(candidate => candidate.id), status: 'mapping', updatedAt: now() })
      }
      for (const candidate of resolution.candidates) {
        const existing = db.sources.find(source => source.id === candidate.id || (!!candidate.did && source.did === candidate.did))
        if (existing) {
          if (!existing.basis.includes('active')) existing.basis = [...existing.basis, 'active']
          added.push(existing)
          continue
        }
        const source: SourceView = { ...candidate, basis: ['active'], notify: candidate.kind === 'person' ? 'pending' : 'unsupported', intentId }
        db.sources.push(source)
        added.push(source)
      }
      emit('sources')
      window.setTimeout(() => {
        for (const source of added) if (source.notify === 'pending') source.notify = 'acknowledged'
        for (const source of added) if (!source.lastSuccessAt) source.lastSuccessAt = Date.now()
        const intent = intentId ? db.intents.find(entry => entry.id === intentId) : undefined
        if (intent) intent.status = 'collecting'
        emit('sources')
      }, 1200)
      return added
    },

    async unfollow(sourceId: string): Promise<'removed' | 'friend_basis'> {
      await delay()
      const source = db.sources.find(entry => entry.id === sourceId)
      if (!source) return 'removed'
      if (source.basis.includes('friend')) {
        source.basis = source.basis.filter(basis => basis !== 'active')
        emit('sources')
        return 'friend_basis'
      }
      db.sources = db.sources.filter(entry => entry.id !== sourceId)
      for (const intent of db.intents) intent.sourceIds = intent.sourceIds.filter(id => id !== sourceId)
      db.intents = db.intents.filter(intent => intent.sourceIds.length > 0)
      emit('sources', 'candidates')
      return 'removed'
    },

    async pauseSource(sourceId: string, paused: boolean): Promise<void> {
      await delay()
      const source = db.sources.find(entry => entry.id === sourceId)
      if (source) source.paused = paused
      emit('sources')
    },

    async listSources(): Promise<{ sources: SourceView[]; intents: MockDb['intents'] }> {
      await delay()
      return { sources: db.sources.map(source => ({ ...source })), intents: db.intents.map(intent => ({ ...intent })) }
    },

    async setMuteRule(rule: MuteRule, on: boolean): Promise<void> {
      const same = (candidate: MuteRule) => candidate.kind === rule.kind && (candidate.kind === 'person' ? candidate.did === (rule as Extract<MuteRule, { kind: 'person' }>).did : candidate.groupId === (rule as Extract<MuteRule, { kind: 'group' }>).groupId)
      db.muteRules = on ? [...db.muteRules.filter(candidate => !same(candidate)), rule] : db.muteRules.filter(candidate => !same(candidate))
      emit('prefs', 'reading', 'candidates')
      await delay()
    },

    async setFilterRule(rule: FilterRule): Promise<void> {
      db.filterRules = db.filterRules.some(candidate => candidate.id === rule.id) ? db.filterRules.map(candidate => (candidate.id === rule.id ? rule : candidate)) : [...db.filterRules, rule]
      emit('prefs', 'reading', 'candidates')
      await delay()
    },

    async setTagOverride(objId: ObjId, tag: string, override: TagOverride | null): Promise<void> {
      const overrides = db.tagOverrides.get(objId) ?? new Map<string, TagOverride>()
      if (override) overrides.set(tag, override)
      else overrides.delete(tag)
      db.tagOverrides.set(objId, overrides)
      emit('prefs', 'reading', 'candidates')
      await delay()
    },

    async markLessLike(objId: ObjId): Promise<void> {
      db.lessLike.add(objId)
      await delay()
    },

    async setFeatured(order: ObjId[]): Promise<void> {
      db.featured.set(db.owner, order)
      emit('profile')
      await delay()
    },

    async setDefaultAudience(audience: AudienceSpec): Promise<void> {
      db.settings.defaultAudience = audience
      emit('prefs')
      await delay()
    },
  }

  function existingSource(did: Did) {
    return did ? db.sources.find(source => source.did === did) : undefined
  }

  return store
}
