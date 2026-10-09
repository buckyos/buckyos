import type {
  AudienceSpec,
  ContactGroup,
  EffectiveTag,
  FilterRule,
  IdentityKind,
  MuteRule,
  PersonalState,
  PublishTask,
  PublishedEntryView,
  ReadingReason,
  ResourceState,
  SourcePath,
  SourceView,
  StreamChange,
  SubscriptionIntent,
  TopicView,
} from '../datamodel/types'
import { EMPTY_PERSONAL } from '../datamodel/types'
import type { CommentType, Did, EntryUrl, FeedContent, FeedHead, FeedObject, FeedSource, FileObject, ObjId, PublicationCategory } from '../protocol/feed'
import type { TagOverride } from '../store/types'

export interface MockIdentity {
  did: Did
  name: string
  kind: IdentityKind
  hue: number
  zone: string
  bio: string
  following: number
}

export interface StoredObject {
  objId: ObjId
  object: FeedObject
  verification: 'verified' | 'unverified' | 'wrapper_only'
  privateCapture?: boolean
}

export interface StoredEntry {
  entry: EntryUrl
  publisher: Did
  audience: AudienceSpec
  heads: FeedHead[]
  kind: PublishedEntryView['kind']
  publishedAt: number
  task?: PublishTask
}

export interface StoredReading {
  objId: ObjId
  admittedAt: number
  reason: ReadingReason
  modelTags: EffectiveTag[]
  classified: boolean
}

export interface StoredCandidate {
  objId: ObjId
  selection: 'unscreened' | 'not_selected' | 'preparing'
  sourcePaths: SourcePath[]
  arrivedAt: number
  firstAdmittedAt: number | null
  openedAt: number | null
  failOpenTimes: number
}

export interface StoredRecord {
  objId: ObjId
  seen: { author: boolean; collector: boolean; push: boolean }
  withdrawn: boolean
}

export interface StoredBody {
  markdown: string
  state: 'ready' | 'preparing' | 'unavailable'
  pendingReads: number
}

export interface MockDb {
  now: number
  owner: Did
  collector: { id: string; name: string }
  identities: Map<Did, MockIdentity>
  objects: Map<ObjId, StoredObject>
  files: Map<ObjId, FileObject>
  bodies: Map<ObjId, StoredBody>
  entries: Map<EntryUrl, StoredEntry>
  reading: StoredReading[]
  resources: Map<ObjId, ResourceState>
  resourceRetry: Map<ObjId, ResourceState>
  candidates: StoredCandidate[]
  records: Map<ObjId, StoredRecord>
  personal: Map<ObjId, PersonalState>
  savedAt: Map<ObjId, { bookmark?: number; readLater?: number }>
  claimed: Map<ObjId, { likes: number }>
  sources: SourceView[]
  intents: SubscriptionIntent[]
  groups: ContactGroup[]
  friends: Map<Did, Set<Did>>
  followers: Map<Did, Set<Did>>
  topics: TopicView[]
  muteRules: MuteRule[]
  filterRules: FilterRule[]
  tagOverrides: Map<ObjId, Map<string, TagOverride>>
  featured: Map<Did, ObjId[]>
  settings: { defaultAudience: AudienceSpec; likeNoticeShown: boolean }
  tasks: Map<string, PublishTask>
  changes: StreamChange[]
  dislikes: Set<ObjId>
  lessLike: Set<ObjId>
  lastFetchAt: number
}

export const OWNER_DID = 'did:bns:leo'
const HOUR = 3600_000
const MIN = 60_000

function hashHex(input: string): string {
  let a = 0x811c9dc5
  let b = 0x9747b28c
  for (let index = 0; index < input.length; index += 1) {
    const code = input.charCodeAt(index)
    a = Math.imul(a ^ code, 0x01000193)
    b = Math.imul(b ^ code, 0x5bd1e995)
    b ^= b >>> 15
  }
  return (a >>> 0).toString(16).padStart(8, '0') + (b >>> 0).toString(16).padStart(8, '0')
}

export const oid = (alias: string): ObjId => `cyfeed:${hashHex(`feed/${alias}`)}`
export const fid = (alias: string): ObjId => `cyfile:${hashHex(`file/${alias}`)}`
export const keyDigest = (input: string) => hashHex(input).slice(0, 12)

const people: Array<Omit<MockIdentity, 'zone'> & { slug: string }> = [
  { slug: 'leo', did: OWNER_DID, name: 'Leo Wang', kind: 'self', hue: 262, bio: 'Builder of decentralized systems. Exploring personal computing and AI-native experiences.', following: 9 },
  { slug: 'alice', did: 'did:bns:alice', name: 'Alice Chen', kind: 'person', hue: 205, bio: 'Full-stack developer & open source contributor.', following: 42 },
  { slug: 'bob', did: 'did:bns:bob', name: 'Bob Zhang', kind: 'person', hue: 145, bio: 'Weekend hiker, weekday SDK tinkerer.', following: 31 },
  { slug: 'sarah', did: 'did:bns:sarah', name: 'Sarah Kim', kind: 'person', hue: 330, bio: 'Product designer at BuckyOS.', following: 58 },
  { slug: 'david', did: 'did:bns:david', name: 'David Liu', kind: 'person', hue: 28, bio: 'Indie game developer.', following: 17 },
  { slug: 'foodie', did: 'did:bns:foodie', name: 'Foodie Explorer', kind: 'person', hue: 12, bio: 'Cooking from scratch, one broth at a time.', following: 120 },
]

const crowd = ['Mia Park', 'Omar Haddad', 'Yuki Tanaka', 'Lena Fischer', 'Carlos Ruiz', 'Priya Nair', 'Tom Becker', 'Ana Souza', 'Wei Zhang', 'Noah Smith', 'Elif Kaya', 'Sam Okafor', 'Ines Duarte', 'Kai Larsen', 'Rhea Kapoor', 'Jon Weber', 'Zoe Martin', 'Ravi Shah', 'Lucia Bianchi', 'Hugo Petit']

const slugOf = (name: string) => name.toLowerCase().replace(/[^a-z]+/g, '-')

export function zoneOf(did: Did) {
  return `${did.split(':').pop()}.buckyos.io`
}

export function feedEntry(did: Did, key: string): EntryUrl {
  return `cyfs://${zoneOf(did)}/home/feed/@/${key}`
}

export function reactionEntry(did: Did, target: ObjId, type: CommentType): EntryUrl {
  return `cyfs://${zoneOf(did)}/home/reactions/@/${keyDigest(`${did}|${target}|${type}`)}`
}

interface PostSpec {
  alias: string
  publisher: Did
  key?: string
  ageHours: number
  content?: FeedContent
  wraps?: ObjId
  comment?: { type: CommentType; target: ObjId }
  tags?: string[]
  source?: FeedSource
  link?: string
  category?: PublicationCategory
  audience?: AudienceSpec
  verification?: StoredObject['verification']
  privateCapture?: boolean
  baseOn?: ObjId
}

export function seedDatabase(now: number, options: { empty?: boolean } = {}): MockDb {
  const db: MockDb = {
    now,
    owner: OWNER_DID,
    collector: { id: 'open-index', name: 'Open Index' },
    identities: new Map(),
    objects: new Map(),
    files: new Map(),
    bodies: new Map(),
    entries: new Map(),
    reading: [],
    resources: new Map(),
    resourceRetry: new Map(),
    candidates: [],
    records: new Map(),
    personal: new Map(),
    savedAt: new Map(),
    claimed: new Map(),
    sources: [],
    intents: [],
    groups: [
      { id: 'group-hiking', name: 'Hiking buddies', members: ['did:bns:bob'] },
      { id: 'group-colleagues', name: 'Colleagues', members: ['did:bns:alice'] },
    ],
    friends: new Map([
      [OWNER_DID, new Set(['did:bns:alice', 'did:bns:bob'])],
      ['did:bns:alice', new Set([OWNER_DID, 'did:bns:bob'])],
      ['did:bns:bob', new Set([OWNER_DID, 'did:bns:alice', 'did:bns:david'])],
      ['did:bns:david', new Set(['did:bns:bob'])],
    ]),
    followers: new Map(),
    topics: [
      { id: 'topic-tech', name: 'Tech', tags: ['tech', 'personal-server', 'buckyos'], subscribed: true, recentCount: 0 },
      { id: 'topic-ai', name: 'AI & ML', tags: ['ai'], subscribed: true, recentCount: 0 },
      { id: 'topic-local', name: 'Local News', tags: ['local', 'sf'], subscribed: true, recentCount: 0 },
      { id: 'topic-finance', name: 'Finance', tags: ['finance'], subscribed: true, recentCount: 0 },
      { id: 'topic-design', name: 'Design', tags: ['design'], subscribed: true, recentCount: 0 },
      { id: 'topic-food', name: 'Food & Cooking', tags: ['food'], subscribed: false, recentCount: 0 },
      { id: 'topic-gaming', name: 'Gaming', tags: ['gaming'], subscribed: false, recentCount: 0 },
      { id: 'topic-outdoors', name: 'Outdoors', tags: ['hiking', 'photography'], subscribed: false, recentCount: 0 },
    ],
    muteRules: [],
    filterRules: [
      { id: 'rule-ai-full', enabled: true, conditions: ['ai_full'], acceptInferred: true, minConfidence: 0.8, unknown: 'show' },
      { id: 'rule-ai-assisted-low', enabled: false, conditions: ['ai_assisted', 'low_quality'], acceptInferred: true, minConfidence: 0.6, unknown: 'show' },
    ],
    tagOverrides: new Map(),
    featured: new Map(),
    // 待确认（TODO §13）：正文、评论、转发的默认受众，临时为公开，可在偏好设置中修改
    settings: { defaultAudience: { kind: 'public' }, likeNoticeShown: false },
    tasks: new Map(),
    changes: [],
    dislikes: new Set(),
    lessLike: new Set(),
    lastFetchAt: now - 4 * MIN,
  }

  for (const person of people) db.identities.set(person.did, { ...person, zone: zoneOf(person.did) })
  crowd.forEach((name, index) => {
    const did = `did:bns:${slugOf(name)}`
    db.identities.set(did, { did, name, kind: 'person', hue: (index * 47) % 360, zone: zoneOf(did), bio: '', following: 10 + index })
  })

  const crowdDids = crowd.map(name => `did:bns:${slugOf(name)}`)
  const generatedFollowers = Array.from({ length: 125 }, (_, index) => `did:bns:reader-${index}`)
  db.followers.set(OWNER_DID, new Set(['did:bns:alice', 'did:bns:bob', 'did:bns:sarah', ...generatedFollowers]))
  db.followers.set('did:bns:alice', new Set([OWNER_DID, 'did:bns:bob', 'did:bns:sarah', ...crowdDids.slice(0, 12)]))
  db.followers.set('did:bns:sarah', new Set([OWNER_DID, ...crowdDids]))
  db.followers.set('did:bns:david', new Set([OWNER_DID, 'did:bns:bob', ...crowdDids.slice(4, 9)]))

  const file = (alias: string, mime: string, meta: Partial<FileObject['meta']> = {}, name = alias) => {
    const objId = fid(alias)
    db.files.set(objId, { kind: 'file', name, meta: { mime, size: meta.size ?? 240_000, ...meta } })
    return objId
  }

  const iatOf = (ageHours: number) => Math.floor((now - ageHours * HOUR) / 1000)

  const put = (spec: PostSpec): ObjId => {
    const objId = oid(spec.alias)
    const entry = spec.key ? (spec.comment && ['like', 'bookmark', 'repost'].includes(spec.comment.type) ? reactionEntry(spec.publisher, spec.comment.target, spec.comment.type) : feedEntry(spec.publisher, spec.key)) : undefined
    const object: FeedObject = {
      kind: spec.comment ? 'comment' : 'post',
      ...(spec.comment ? { comment_type: spec.comment.type } : {}),
      publisher: spec.publisher,
      iat: iatOf(spec.ageHours),
      ...(entry ? { entry } : {}),
      ...(spec.content ? { content: spec.content } : {}),
      ...(spec.wraps ? { wraps: spec.wraps } : {}),
      ...(spec.comment && spec.comment.type !== 'repost' && spec.comment.type !== 'quote' ? { references: [{ relation: 'comment_on' as const, object_id: spec.comment.target }] } : {}),
      ...(spec.tags ? { tags: spec.tags } : {}),
      ...(spec.source ? { source: spec.source } : {}),
      ...(spec.link ? { link: spec.link } : {}),
      ...(spec.category ? { publication_category: spec.category } : {}),
      ...(spec.baseOn ? { base_on: spec.baseOn } : {}),
    }
    db.objects.set(objId, { objId, object, verification: spec.verification ?? (spec.source ? 'wrapper_only' : 'verified'), privateCapture: spec.privateCapture })
    if (entry) {
      const existing = db.entries.get(entry)
      const updatedAt = now - spec.ageHours * HOUR
      if (existing) {
        existing.heads.push({ kind: 'feed_head', entry, seq: existing.heads.length + 1, state: 'active', current: objId, updated_at_ms: updatedAt })
      } else {
        const kind: PublishedEntryView['kind'] = spec.comment ? (spec.comment.type === 'text' ? 'comment' : spec.comment.type) : 'post'
        db.entries.set(entry, {
          entry,
          publisher: spec.publisher,
          audience: spec.audience ?? { kind: 'public' },
          heads: [{ kind: 'feed_head', entry, seq: 1, state: 'active', current: objId, updated_at_ms: updatedAt }],
          kind,
          publishedAt: updatedAt,
        })
      }
    }
    return objId
  }

  const withdraw = (objId: ObjId, ageHours: number) => {
    const entry = db.objects.get(objId)?.object.entry
    const stored = entry ? db.entries.get(entry) : undefined
    if (!stored) return
    stored.heads.push({ kind: 'feed_head', entry: stored.entry, seq: stored.heads.length + 1, state: 'withdrawn', updated_at_ms: now - ageHours * HOUR })
  }

  const ALICE = 'did:bns:alice'
  const BOB = 'did:bns:bob'
  const SARAH = 'did:bns:sarah'
  const DAVID = 'did:bns:david'
  const FOODIE = 'did:bns:foodie'
  const ME = OWNER_DID

  /* ── Alice ── */
  const a1v1 = put({ alias: 'alice-did-module-v1', publisher: ALICE, key: 'did-module', ageHours: 9, tags: ['tech', 'buckyos'], content: { type: 'text', text: 'Just shipped the new decentralized identity module for BuckyOS. The DID resolver now handles cross-zone verification in under 200ms. Huge milestone for the team!' } })
  const a1v2 = put({ alias: 'alice-did-module-v2', publisher: ALICE, key: 'did-module', ageHours: 2, baseOn: a1v1, tags: ['tech', 'buckyos'], content: { type: 'text', text: 'Update: after caching resolver results per zone, cross-zone DID verification now finishes in under 120ms. Thanks to everyone who profiled it with me.' } })
  const a2Body = file('alice-personal-servers.md', 'text/markdown', { size: 18_400 }, 'personal-servers.md')
  db.bodies.set(a2Body, { state: 'ready', pendingReads: 0, markdown: '# Why Personal Servers Will Replace Cloud Subscriptions\n\nThe cloud computing paradigm has dominated for over a decade, but several converging trends suggest a fundamental shift is coming.\n\n## Hardware got cheap\n\nA $500 device can now run multiple AI models, serve web applications, and store terabytes of personal data.\n\n## Compliance got expensive\n\nPrivacy regulations make cloud services increasingly complex to operate. GDPR, CCPA, and emerging AI governance frameworks create overhead that personal servers simply avoid.\n\n## Your data is your most valuable asset\n\nKeeping it on your own hardware is not just a privacy choice — it is an economic one.\n\nIn the next decade we will see a renaissance in personal computing infrastructure.' })
  const a2 = put({ alias: 'alice-personal-servers', publisher: ALICE, key: 'personal-servers', ageHours: 30, wraps: a2Body, tags: ['tech', 'personal-server'], content: { type: 'article', title: 'Why Personal Servers Will Replace Cloud Subscriptions', summary: 'An essay on the economics and philosophy of personal computing infrastructure. As hardware costs drop and AI capabilities grow, the case for personal servers gets stronger.' } })
  const a3Video = file('alice-setup-demo.mp4', 'video/mp4', { duration_ms: 312_000, width: 1920, height: 1080, size: 48_000_000 }, 'setup-demo.mp4')
  const a3 = put({ alias: 'alice-setup-demo', publisher: ALICE, key: 'setup-demo', ageHours: 34, wraps: a3Video, tags: ['tech', 'buckyos'], content: { type: 'video', title: 'BuckyOS Demo: Setting up your personal server in 5 minutes', text: 'Quick walkthrough from unboxing a mini PC to having your personal cloud ready.', cover: file('alice-setup-cover.jpg', 'image/jpeg', { width: 1280, height: 720 }) } })
  const a4 = put({ alias: 'alice-family-bbq', publisher: ALICE, key: 'family-bbq', ageHours: 14, audience: { kind: 'friends' }, tags: ['life'], content: { type: 'image', text: 'Family BBQ this weekend — sharing the photos with friends only.', media: [{ object: file('alice-bbq-1.jpg', 'image/jpeg', { width: 1080, height: 1080 }), alt: 'Grill with skewers' }, { object: file('alice-bbq-2.jpg', 'image/jpeg', { width: 1080, height: 1080 }), alt: 'Garden table at sunset' }] } })
  const a5 = put({ alias: 'alice-reading-list', publisher: ALICE, key: 'reading-list', ageHours: 70, tags: ['tech'], content: { type: 'text', text: 'Reading list for the week: three papers on content-addressed storage and one on gossip protocols. Happy to share notes.' } })

  /* ── Sarah ── */
  const s1 = put({ alias: 'sarah-card-explorations', publisher: SARAH, key: 'card-explorations', ageHours: 4, tags: ['design'], content: { type: 'image', text: 'New design explorations for the HomeStation feed card. Playing with information density and visual hierarchy. Thoughts?', media: [1, 2, 3].map(n => ({ object: file(`sarah-card-v${n}.png`, 'image/png', { width: 1080, height: 1350 }), alt: `Feed card design v${n}` })) } })
  const s2 = put({ alias: 'sarah-design-tip', publisher: SARAH, key: 'design-tip', ageHours: 60, tags: ['design'], content: { type: 'text', text: 'Design tip: keep the recommendation reason visually distinct from the content itself. People need to see WHY something is in their feed, not just WHAT it is.' } })
  const s3 = put({ alias: 'sarah-quote-servers', publisher: SARAH, key: 'q-0001', ageHours: 20, comment: { type: 'quote', target: a2 }, wraps: a2, content: { type: 'text', text: 'Agree with most of this. The missing piece is making backups as boring as cloud sync.' } })
  const s4 = put({ alias: 'sarah-portfolio', publisher: SARAH, key: 'work-portfolio', ageHours: 50, category: 'work', tags: ['design'], link: 'https://sarah.design/portfolio-2026', content: { type: 'link', title: 'My 2026 design portfolio', summary: 'Case studies on feeds, readers and quiet notifications.', cover: file('sarah-portfolio-cover.png', 'image/png', { width: 1600, height: 900 }) } })

  /* ── David ── */
  const d1Video = file('david-devlog-24.mp4', 'video/mp4', { duration_ms: 632_000, width: 1920, height: 1080, size: 96_000_000 }, 'devlog-24.mp4')
  const d1 = put({ alias: 'david-devlog-24', publisher: DAVID, key: 'devlog-24', ageHours: 11, wraps: d1Video, tags: ['gaming'], content: { type: 'video', title: 'Indie game devlog #24: Procedural world generation', text: 'Mountains, rivers and biomes all generated from a single seed value.', cover: file('david-devlog-24-cover.jpg', 'image/jpeg', { width: 1280, height: 720 }) } })
  const d2 = put({ alias: 'david-echoes', publisher: DAVID, key: 'unused', ageHours: 40, category: 'product', tags: ['gaming'], link: 'https://david-liu.example/games/echoes-of-the-void', content: { type: 'product', title: 'Echoes of the Void — early access', summary: 'A roguelike exploration game with procedural worlds. $14.99 for early supporters; details on my page.', media: [{ object: file('echoes-1.png', 'image/png', { width: 1920, height: 1080 }), alt: 'Alien landscape' }, { object: file('echoes-2.png', 'image/png', { width: 1920, height: 1080 }), alt: 'Base building' }] } })
  const productEntry = 'did:bns:echoes-of-the-void'
  const d2Stored = db.objects.get(d2)!
  const d2Old = d2Stored.object.entry!
  d2Stored.object = { ...d2Stored.object, entry: productEntry }
  const d2Entry = db.entries.get(d2Old)!
  db.entries.delete(d2Old)
  db.entries.set(productEntry, { ...d2Entry, entry: productEntry, heads: d2Entry.heads.map(head => ({ ...head, entry: productEntry })) })
  const d3 = put({ alias: 'david-playtest', publisher: DAVID, key: 'playtest', ageHours: 16, audience: { kind: 'friends' }, tags: ['gaming'], content: { type: 'text', text: 'Playtest build for friends: the new cave biome is in. Please don\'t share outside this circle yet.' } })
  const d4Video = file('david-boss-fight.mp4', 'video/mp4', { duration_ms: 95_000, width: 1920, height: 1080, size: 22_000_000 }, 'boss-fight.mp4')
  const d4 = put({ alias: 'david-boss-fight', publisher: DAVID, key: 'boss-fight', ageHours: 44, wraps: d4Video, tags: ['gaming'], content: { type: 'video', title: 'Boss fight preview', text: 'Thirty seconds of the void warden. Sound on.', cover: file('david-boss-cover.jpg', 'image/jpeg', { width: 1280, height: 720 }) } })

  /* ── Bob ── */
  const b1Video = file('bob-assistant-tutorial.mp4', 'video/mp4', { duration_ms: 845_000, width: 1920, height: 1080, size: 120_000_000 }, 'assistant-tutorial.mp4')
  const b1 = put({ alias: 'bob-assistant-tutorial', publisher: BOB, key: 'assistant-tutorial', ageHours: 6, wraps: b1Video, tags: ['tech', 'ai'], content: { type: 'video', title: 'Building a personal AI assistant with the BuckyOS SDK', text: 'Step-by-step: setup, configuration and wiring it to MessageHub.', cover: file('bob-tutorial-cover.jpg', 'image/jpeg', { width: 1280, height: 720 }) } })
  const b2 = put({ alias: 'bob-voice-trail', publisher: BOB, key: 'voice-trail', ageHours: 13, tags: ['hiking', 'life'], content: { type: 'audio', text: 'Quick voice note about this weekend\'s trail plan.', media: [{ object: file('bob-trail-note.m4a', 'audio/mp4', { duration_ms: 48_000, size: 420_000 }, 'trail-note.m4a') }] } })
  const b3 = put({ alias: 'bob-golden-gate', publisher: BOB, key: 'golden-gate', ageHours: 26, tags: ['photography', 'local'], content: { type: 'image', text: 'Golden Gate Bridge at sunset, captured from the Lands End trail.', media: [{ object: file('bob-golden-gate.jpg', 'image/jpeg', { width: 2048, height: 1365 }), alt: 'Golden Gate Bridge at sunset' }] } })
  withdraw(b3, 3)
  const b4Snapshot = file('garden-guide-snapshot.html', 'text/html', { size: 64_000 }, 'garden-guide.html')
  db.bodies.set(b4Snapshot, { state: 'ready', pendingReads: 0, markdown: '# Home Gardening Guide\n\nStart small: two planters and herbs that tolerate shade.\n\n## Light\n\nMost leafy greens need four hours of direct light.\n\n## Water\n\nWater in the morning; check the soil two knuckles deep.' })
  const b4 = put({ alias: 'bob-shared-garden', publisher: BOB, key: 'clip-0001', ageHours: 46, wraps: b4Snapshot, tags: ['life'], source: { kind: 'web', original_url: 'https://example.org/garden-guide', original_author: 'Garden Weekly editors', captured_at_ms: now - 47 * HOUR }, content: { type: 'article', title: 'Home Gardening Guide', summary: 'A public gardening article Bob saved and shared.' } })

  /* ── Foodie ── */
  const f1 = put({ alias: 'foodie-ramen', publisher: FOODIE, key: 'ramen', ageHours: 42, tags: ['food'], content: { type: 'image', text: 'Homemade ramen from scratch! The broth took 12 hours but was absolutely worth it.', media: ['Ramen bowl', 'Broth preparation', 'Noodle making', 'Final plating'].map((alt, index) => ({ object: file(`ramen-${index}.jpg`, 'image/jpeg', { width: 1080, height: 1080 }), alt })) } })
  const f2Body = file('sourdough-guide.md', 'text/markdown', { size: 22_000 }, 'sourdough-guide.md')
  db.bodies.set(f2Body, { state: 'preparing', pendingReads: 1, markdown: '# The Complete Guide to Sourdough Bread\n\n## Understanding your starter\n\nFeed it equal parts flour and water by weight and keep it at room temperature.\n\n## Bulk fermentation\n\nTemperature and time are your two most important variables.\n\n## Baking\n\nDutch oven at 260°C for 20 minutes covered, then 25 minutes uncovered at 230°C.' })
  const f2 = put({ alias: 'foodie-sourdough', publisher: FOODIE, key: 'sourdough', ageHours: 52, wraps: f2Body, verification: 'unverified', tags: ['food'], content: { type: 'article', title: 'The Complete Guide to Sourdough Bread', summary: 'Three years of daily baking compiled into one guide, from starter care to shaping.' } })

  /* ── Bob's interactions shown as their own cards ── */
  const b5 = put({ alias: 'bob-repost-ramen', publisher: BOB, key: 'repost-ramen', ageHours: 10, comment: { type: 'repost', target: f1 }, wraps: f1 })
  const b6 = put({ alias: 'bob-comment-servers', publisher: BOB, key: 'c-0001', ageHours: 5, comment: { type: 'text', target: a2 }, content: { type: 'text', text: 'The economics section is spot on. My NAS paid for itself in 14 months.' } })
  const b7 = put({ alias: 'bob-quote-playtest', publisher: BOB, key: 'q-0002', ageHours: 15, comment: { type: 'quote', target: d3 }, wraps: d3, content: { type: 'text', text: 'This cave biome is gorgeous. David, ship it!' } })

  /* ── Captures by my Spider (private, no entry) ── */
  const captureSource = (url: string, author: string, kind: FeedSource['kind'], ageHours: number): FeedSource => ({ kind, original_url: url, original_author: author, captured_at_ms: now - ageHours * HOUR })
  const x1Snapshot = file('gpt5-snapshot.html', 'text/html', { size: 88_000 }, 'gpt5-article.html')
  db.bodies.set(x1Snapshot, { state: 'ready', pendingReads: 0, markdown: '# OpenAI announces GPT-5 with native multimodal reasoning\n\nOpenAI has unveiled its latest model, featuring breakthrough capabilities in multimodal reasoning, real-time tool use, and improved factual accuracy.\n\n- Native vision, audio, and code understanding in a single model\n- Real-time web browsing and tool integration\n- New fine-tuning API for enterprise customers' })
  const x1 = put({ alias: 'capture-gpt5', publisher: ME, ageHours: 1, privateCapture: true, wraps: x1Snapshot, source: captureSource('https://techcrunch.com/2026/10/08/openai-gpt5', 'Kyle Wiggers', 'rss', 1), content: { type: 'article', title: 'OpenAI announces GPT-5 with native multimodal reasoning', summary: 'The model shows a 40% improvement on complex reasoning benchmarks compared to its predecessor.', cover: file('gpt5-cover.jpg', 'image/jpeg', { width: 1200, height: 630 }) } })
  const x2 = put({ alias: 'capture-soma-housing', publisher: ME, ageHours: 7, privateCapture: true, source: captureSource('https://sfchronicle.com/housing/soma-development', 'J.K. Dineen', 'web', 7), link: 'https://sfchronicle.com/housing/soma-development', content: { type: 'link', title: 'San Francisco approves new affordable housing project in SoMa', summary: 'The Board of Supervisors voted 8-3 for a mixed-use development with 200 affordable units.', cover: file('soma-cover.jpg', 'image/jpeg', { width: 800, height: 450 }) } })
  const x3 = put({ alias: 'capture-bart', publisher: ME, ageHours: 36, privateCapture: true, source: captureSource('https://sfgate.com/bart-san-jose-extension', 'SFGate staff', 'web', 36), link: 'https://sfgate.com/bart-san-jose-extension', content: { type: 'link', title: 'New BART extension to San Jose opens ahead of schedule', summary: 'The Silicon Valley Phase II extension connects Milpitas, downtown San Jose and Santa Clara.', cover: file('bart-cover.jpg', 'image/jpeg', { width: 960, height: 540 }) } })
  const x5 = put({ alias: 'capture-ai-digest', publisher: ME, ageHours: 5.5, privateCapture: true, source: captureSource('https://aidigest.example/daily/2026-10-09', 'AI Daily Digest', 'rss', 5.5), link: 'https://aidigest.example/daily/2026-10-09', content: { type: 'link', title: 'Daily AI Digest: three model launches and a safety paper', summary: 'Anthropic, Meta and DeepMind updates summarized in five bullet points.' } })
  const x6 = put({ alias: 'capture-show-hn', publisher: ME, ageHours: 18, privateCapture: true, source: captureSource('https://news.ycombinator.com/item?id=41800001', 'hn user tinkerer42', 'web', 18), link: 'https://news.ycombinator.com/item?id=41800001', content: { type: 'link', title: 'Show HN: A 200-line static site generator for personal servers', summary: 'Markdown in, HTML out, deploys to your own box over SSH.' } })
  const x7 = put({ alias: 'capture-weekly-ai', publisher: ME, ageHours: 48, privateCapture: true, source: captureSource('https://reddit.com/r/programming/comments/weekly-ai', 'u/summary_bot', 'platform', 48), link: 'https://reddit.com/r/programming/comments/weekly-ai', content: { type: 'link', title: 'Weekly AI summary: everything that shipped this week', summary: 'Major releases from three frontier labs and two open models in the top 10.' } })
  const x8 = put({ alias: 'capture-bitcoin', publisher: ME, ageHours: 56, privateCapture: true, source: captureSource('https://bloomberg.example/markets/bitcoin-150k', 'Bloomberg Markets', 'rss', 56), link: 'https://bloomberg.example/markets/bitcoin-150k', content: { type: 'link', title: 'Bitcoin breaks $150k as institutional adoption accelerates', summary: 'Major banks announce custody solutions while new ETF products see record inflows.' } })

  /* ── Me ── */
  const nvidiaSnapshot = file('nvidia-snapshot.html', 'text/html', { size: 72_000 }, 'nvidia-earnings.html')
  db.bodies.set(nvidiaSnapshot, { state: 'ready', pendingReads: 0, markdown: '# NVIDIA stock surges 12% after record quarterly earnings\n\nRevenue reached $48.2 billion, beating estimates by 15%. The data center segment generated $38.1 billion.' })
  const m1 = put({ alias: 'me-ood-wiring', publisher: ME, key: 'ood-wiring', ageHours: 3, tags: ['buckyos'], content: { type: 'text', text: 'Spent the weekend wiring HomeStation into my OOD. Feeds now keep syncing even when my laptop is closed.' } })
  const m2 = put({ alias: 'me-followers-digest', publisher: ME, key: 'followers-digest', ageHours: 6, audience: { kind: 'followers' }, content: { type: 'text', text: 'Notes for followers: I am testing a weekly digest of what my HomeStation filtered out, and why.' } })
  const m3 = put({ alias: 'me-share-nvidia', publisher: ME, key: 'clip-0001', ageHours: 10, wraps: nvidiaSnapshot, tags: ['finance'], source: captureSource('https://bloomberg.example/markets/nvidia-q1', 'Bloomberg Markets', 'web', 11), content: { type: 'article', title: 'NVIDIA stock surges 12% after record quarterly earnings', summary: 'Data center demand keeps growing; I shared this for the margin numbers.' } })
  const m4 = put({ alias: 'me-mission-peak', publisher: ME, key: 'mission-peak', ageHours: 26, audience: { kind: 'friends' }, tags: ['hiking'], content: { type: 'image', text: 'Hiking trip photos from Mission Peak.', media: [{ object: file('mission-peak-1.jpg', 'image/jpeg', { width: 1600, height: 1067 }), alt: 'Summit pole' }, { object: file('mission-peak-2.jpg', 'image/jpeg', { width: 1600, height: 1067 }), alt: 'Trail at dawn' }] } })
  const m5 = put({ alias: 'me-trail-plan', publisher: ME, key: 'trail-plan', ageHours: 30, audience: { kind: 'group', groupId: 'group-hiking' }, tags: ['hiking'], content: { type: 'text', text: 'Saturday trail plan: meet at the Ohlone trailhead at 7am. Bring layers.' } })
  const m6 = put({ alias: 'me-comment-servers', publisher: ME, key: 'c-0001', ageHours: 12, comment: { type: 'text', target: a2 }, content: { type: 'text', text: 'Great essay — the cost curve argument convinced me to move my photos home.' } })
  const m7 = put({ alias: 'me-comment-bbq', publisher: ME, key: 'c-0002', ageHours: 13, audience: { kind: 'dids', dids: [ALICE] }, comment: { type: 'text', target: a4 }, content: { type: 'text', text: 'Looks like a great BBQ! Save me a skewer next time.' } })
  const m8 = put({ alias: 'me-repost-cards', publisher: ME, key: 'repost', ageHours: 3.5, comment: { type: 'repost', target: s1 }, wraps: s1 })
  const m9 = put({ alias: 'me-like-servers', publisher: ME, key: 'like', ageHours: 12, comment: { type: 'like', target: a2 } })
  put({ alias: 'me-setup-notes', publisher: ME, key: 'work-setup-notes', ageHours: 80, category: 'work', tags: ['buckyos'], link: 'https://leo.buckyos.io/notes/homestation-setup', content: { type: 'link', title: 'HomeStation setup notes', summary: 'How I run HomeStation on a mini PC with two disks.', cover: file('leo-notes-cover.png', 'image/png', { width: 1600, height: 900 }) } })
  put({ alias: 'me-pi-kit', publisher: ME, key: 'product-pi-kit', ageHours: 100, category: 'product', link: 'https://leo.buckyos.io/market/pi-kit', content: { type: 'product', title: 'Raspberry Pi 5 kit (used)', summary: 'Pi 5, case, 128GB card. Pickup in San Jose.', media: [{ object: file('pi-kit.jpg', 'image/jpeg', { width: 1200, height: 900 }), alt: 'Pi kit on desk' }] } })

  const setTask = (objId: ObjId, delivery: PublishTask['delivery']) => {
    const entry = db.objects.get(objId)?.object.entry
    const stored = entry ? db.entries.get(entry) : undefined
    if (!stored || !entry) return
    const task: PublishTask = { key: `seed-${objId}`, stage: 'published', entry, objId, delivery, createdAt: stored.publishedAt }
    stored.task = task
    db.tasks.set(task.key, task)
  }
  setTask(m1, { state: 'delivered', delivered: 5, total: 5 })
  setTask(m2, { state: 'delivering', delivered: 3, total: 5 })
  setTask(m3, { state: 'partially_failed', delivered: 4, total: 6 })
  setTask(m4, { state: 'delivered', delivered: 2, total: 2 })
  setTask(m5, { state: 'delivered', delivered: 1, total: 1 })
  setTask(m7, { state: 'delivered', delivered: 1, total: 1 })

  db.featured.set(ME, [a2, m1, s4])
  db.featured.set(ALICE, [a2, a3])

  /* ── Personal state ── */
  const personal = (objId: ObjId, patch: Partial<PersonalState>) => db.personal.set(objId, { ...EMPTY_PERSONAL, ...db.personal.get(objId), ...patch })
  personal(a2, { like: { on: true, visibility: 'public', delivery: 'delivered', entry: db.objects.get(m9)!.object.entry, seq: 1 }, readLater: true })
  personal(s1, { repost: { on: true, visibility: 'public', delivery: 'delivered', entry: db.objects.get(m8)!.object.entry, seq: 1 }, bookmark: { on: true, visibility: 'private', delivery: 'none' } })
  personal(b3, { bookmark: { on: true, visibility: 'private', delivery: 'none' } })
  personal(x1, { readLater: true })
  db.savedAt.set(s1, { bookmark: now - 4 * HOUR })
  db.savedAt.set(b3, { bookmark: now - 20 * HOUR })
  db.savedAt.set(a2, { readLater: now - 9 * HOUR })
  db.savedAt.set(x1, { readLater: now - 0.8 * HOUR })

  /* ── Others' comments and reactions (records) ── */
  let recordIndex = 0
  const record = (publisher: Did, type: CommentType, target: ObjId, ageHours: number, seen: StoredRecord['seen'], text?: string, withdrawn = false) => {
    recordIndex += 1
    const alias = `record-${recordIndex}-${type}`
    const objId = text || type === 'quote'
      ? put({ alias, publisher, key: `r-${recordIndex}`, ageHours, comment: { type, target }, ...(type === 'quote' ? { wraps: target } : {}), ...(text ? { content: { type: 'text', text } } : {}) })
      : put({ alias, publisher, key: `r-${recordIndex}`, ageHours, comment: { type, target }, ...(type === 'repost' ? { wraps: target } : {}) })
    db.records.set(objId, { objId, seen, withdrawn })
    return objId
  }
  const crowdRecords = (target: ObjId, type: CommentType, count: number, baseAge: number) => {
    for (let index = 0; index < count; index += 1) {
      const did = index < crowdDids.length ? crowdDids[index] : `did:bns:reader-${index}`
      record(did, type, target, baseAge - index * 0.05, { author: index % 7 !== 3, collector: index % 3 !== 0, push: index % 5 === 0 })
    }
  }
  const everywhere = { author: true, collector: true, push: true }

  record(BOB, 'text', a1v1, 8, everywhere, 'Congrats! How did you get resolution that fast?')
  record(SARAH, 'text', a1v1, 7.5, { author: true, collector: false, push: false }, 'Huge milestone for the team.')
  record(SARAH, 'text', a1v2, 1.5, { author: true, collector: true, push: false }, '120ms is impressive. Is the cache shared across zones?')
  crowdRecords(a1v1, 'like', 12, 8)
  crowdRecords(a1v2, 'like', 3, 1.8)
  crowdRecords(a1v1, 'repost', 2, 8)

  db.records.set(b6, { objId: b6, seen: everywhere, withdrawn: false })
  db.records.set(m6, { objId: m6, seen: everywhere, withdrawn: false })
  db.records.set(s3, { objId: s3, seen: everywhere, withdrawn: false })
  db.records.set(m9, { objId: m9, seen: everywhere, withdrawn: false })
  record('did:bns:mia-park', 'text', a2, 22, { author: false, collector: true, push: false }, 'I disagree about cloud costs — egress is the real lock-in, not storage.')
  record('did:bns:omar-haddad', 'text', a2, 21, { author: true, collector: false, push: false }, 'Bookmarking this for my team.')
  record('did:bns:yuki-tanaka', 'text', a2, 19, { author: true, collector: true, push: false }, 'Would love a follow-up on backups.')
  crowdRecords(a2, 'like', 40, 28)
  crowdRecords(a2, 'repost', 9, 27)
  record('did:bns:noah-smith', 'like', a2, 26, everywhere, undefined, true)

  record(ALICE, 'text', s1, 3.5, everywhere, 'Version 2 reads best for me.')
  record('did:bns:yuki-tanaka', 'text', s1, 3, { author: true, collector: true, push: false }, 'Love the density in v3.')
  crowdRecords(s1, 'like', 18, 3.8)
  crowdRecords(s1, 'repost', 3, 3.7)
  db.records.set(m8, { objId: m8, seen: everywhere, withdrawn: false })

  record('did:bns:lena-fischer', 'text', d2, 30, everywhere, 'Wishlisted! Is there a Linux build?')
  crowdRecords(d2, 'like', 37, 39)
  db.claimed.set(d2, { likes: 1200 })

  record(BOB, 'text', a4, 12, { author: true, collector: false, push: false }, 'Fun! Next one at my place.')
  db.records.set(m7, { objId: m7, seen: { author: true, collector: false, push: false }, withdrawn: false })
  crowdRecords(f1, 'like', 25, 41)
  record('did:bns:ana-souza', 'text', f1, 40, everywhere, 'Recipe please!')
  record('did:bns:ravi-shah', 'text', f1, 39, { author: false, collector: true, push: false }, 'Twelve hours is commitment.')
  db.records.set(b5, { objId: b5, seen: everywhere, withdrawn: false })
  crowdRecords(b1, 'like', 9, 5.5)
  record(ALICE, 'text', b1, 5, everywhere, 'Great pacing. The MessageHub part saved me an afternoon.')
  crowdRecords(b3, 'like', 6, 25)
  crowdRecords(s2, 'like', 14, 59)
  crowdRecords(d1, 'like', 7, 10)
  crowdRecords(m1, 'like', 5, 2.5)
  record(ALICE, 'text', m1, 2, everywhere, 'Welcome to the always-on club.')
  crowdRecords(b4, 'like', 4, 45)

  /* ── Reading list ── */
  const reason = (code: ReadingReason['code'], text: string, refs: ReadingReason['refs']): ReadingReason => ({ code, text, refs })
  const followed = (name: string, sourceId: string, basis: string) => reason('followed', `From ${name}, whom you follow (${basis})`, [{ kind: 'source', id: sourceId, label: name }])
  const friend = (name: string, sourceId: string) => reason('friend', `From your friend ${name}`, [{ kind: 'source', id: sourceId, label: name }])
  const sub = (name: string, sourceId: string) => reason('subscription', `From your subscription to ${name}`, [{ kind: 'source', id: sourceId, label: name }])
  const intent = (sourceName: string, sourceId: string) => reason('subscription', `Matches your subscription intent “San Francisco local news and Bay Area transit”`, [{ kind: 'intent', id: 'intent-sf', label: 'San Francisco local news and Bay Area transit' }, { kind: 'source', id: sourceId, label: sourceName }])
  const viaCollector = (topic: string, topicId: string) => reason('collector', `Recommended by collector Open Index for the topic ${topic}`, [{ kind: 'collector', id: 'open-index', label: 'Open Index' }, { kind: 'topic', id: topicId, label: topic }])
  const genTag = (tag: 'ai_full' | 'ai_assisted' | 'low_quality', confidence: number, basis: string): EffectiveTag => ({ tag, label: tag, source: 'model', status: 'inferred', scope: 'whole_content', confidence, basis, classifierRevision: 'local-classifier 2026.10 · policy v3' })
  const topicTag = (tag: string): EffectiveTag => ({ tag, label: tag, source: 'model', status: 'inferred', scope: 'whole_content', classifierRevision: 'local-classifier 2026.10 · policy v3' })

  const readings: Array<[ObjId, number, ReadingReason, EffectiveTag[]?, boolean?]> = [
    [a1v1, 8.8, followed('Alice Chen', 'src-alice', 'friend + followed')],
    [x1, 0.9, sub('TechCrunch', 'src-techcrunch'), [topicTag('ai'), topicTag('tech')]],
    [s1, 3.9, followed('Sarah Kim', 'src-sarah', 'followed')],
    [b6, 4.9, friend('Bob Zhang', 'src-bob')],
    [x5, 5.4, sub('AI Daily Digest', 'src-ai-digest'), [topicTag('ai'), genTag('ai_full', 0.86, 'Uniform sentence templates and generator watermark phrases across the full text.')]],
    [b1, 5.9, friend('Bob Zhang', 'src-bob')],
    [a4, 13.8, friend('Alice Chen', 'src-alice')],
    [x2, 6.9, intent('SF Chronicle', 'src-sfchronicle'), [topicTag('local')]],
    [b5, 9.9, friend('Bob Zhang', 'src-bob')],
    [d1, 10.9, followed('David Liu', 'src-david', 'followed')],
    [s3, 19.9, followed('Sarah Kim', 'src-sarah', 'followed')],
    [b2, 12.9, friend('Bob Zhang', 'src-bob')],
    [x6, 17.9, sub('Hacker News', 'src-hn'), [topicTag('tech'), genTag('ai_full', 0.62, 'Some paragraphs match generator phrasing; the code samples look hand-written.')]],
    [a2, 29.5, followed('Alice Chen', 'src-alice', 'friend + followed')],
    [b3, 25.9, friend('Bob Zhang', 'src-bob')],
    [b7, 14.9, friend('Bob Zhang', 'src-bob')],
    [a3, 33.9, followed('Alice Chen', 'src-alice', 'friend + followed')],
    [x3, 35.9, intent('SFGate', 'src-sfgate'), [topicTag('local')]],
    [d2, 39.9, followed('David Liu', 'src-david', 'followed')],
    [f1, 41.9, viaCollector('Food & Cooking', 'topic-food')],
    [d4, 43.9, followed('David Liu', 'src-david', 'followed'), [], false],
    [b4, 45.9, friend('Bob Zhang', 'src-bob')],
    [x7, 47.9, sub('r/programming', 'src-reddit'), [topicTag('ai'), genTag('ai_assisted', 0.71, 'Summary sections read as model-written; the link list is curated by hand.'), genTag('low_quality', 0.82, 'Mostly restates headlines with no new information.')]],
    [f2, 51.9, viaCollector('Food & Cooking', 'topic-food'), [], false],
    [s4, 49.9, followed('Sarah Kim', 'src-sarah', 'followed')],
    [x8, 55.9, sub('Bloomberg Markets', 'src-bloomberg'), [topicTag('finance')]],
    [s2, 59.9, followed('Sarah Kim', 'src-sarah', 'followed')],
    [a5, 69.9, followed('Alice Chen', 'src-alice', 'friend + followed')],
  ]
  db.reading = readings
    .map(([objId, ageHours, readingReason, tags = [], classified = true]) => ({ objId, admittedAt: now - ageHours * HOUR, reason: readingReason, modelTags: tags, classified }))
    .sort((left, right) => right.admittedAt - left.admittedAt)

  const resources: Array<[ObjId, ResourceState]> = [[b1, 'local'], [a3, 'reachable'], [d1, 'preparing'], [d4, 'unavailable'], [f1, 'reachable'], [f2, 'preparing'], [b4, 'reachable']]
  for (const [objId, state] of resources) db.resources.set(objId, state)
  db.resourceRetry.set(d1, 'local')
  db.resourceRetry.set(d4, 'unavailable')
  db.resourceRetry.set(f2, 'local')

  /* ── Followed candidates not in the feed ── */
  const k1 = put({ alias: 'alice-coffee-notes', publisher: ALICE, key: 'coffee-notes', ageHours: 2, tags: ['tech'], content: { type: 'text', text: 'Coffee chat notes: what people actually want from a personal server is “it just keeps working”.' } })
  const k2 = put({ alias: 'sarah-palette', publisher: SARAH, key: 'palette', ageHours: 5, tags: ['design'], content: { type: 'image', text: 'Color palette study for the HomeStation cards.', media: [{ object: file('sarah-palette.png', 'image/png', { width: 1600, height: 1000 }), alt: 'Palette swatches' }] } })
  const k3Video = file('david-soundtrack.mp4', 'video/mp4', { duration_ms: 64_000, width: 1280, height: 720 }, 'soundtrack-teaser.mp4')
  const k3 = put({ alias: 'david-soundtrack', publisher: DAVID, key: 'soundtrack', ageHours: 7, wraps: k3Video, tags: ['gaming'], content: { type: 'video', title: 'Soundtrack teaser', text: 'First minute of the main theme.', cover: file('david-soundtrack-cover.jpg', 'image/jpeg', { width: 1280, height: 720 }) } })
  const k4 = put({ alias: 'bob-trail-conditions', publisher: BOB, key: 'trail-conditions', ageHours: 8, tags: ['hiking'], content: { type: 'text', text: 'Trail conditions update: the ridge section is muddy after the rain. Bring poles.' } })
  const k5Body = file('resolver-caching.md', 'text/markdown', { size: 12_000 }, 'resolver-caching.md')
  db.bodies.set(k5Body, { state: 'ready', pendingReads: 0, markdown: '# Notes on DID resolver caching\n\nCache per zone, invalidate on document sequence change, never cache negative answers for long.' })
  const k5 = put({ alias: 'alice-resolver-caching', publisher: ALICE, key: 'resolver-caching', ageHours: 20, wraps: k5Body, tags: ['tech'], content: { type: 'article', title: 'Notes on DID resolver caching', summary: 'What we learned while getting cross-zone verification under 120ms.' } })
  const k6 = put({ alias: 'sarah-typography', publisher: SARAH, key: 'typography', ageHours: 30, tags: ['design'], content: { type: 'text', text: 'Thoughts on typography in dense feeds: line height matters more than font size.' } })
  const pull = (name: string): SourcePath => ({ transport: 'pull', label: name })
  const push = (name: string): SourcePath => ({ transport: 'push', label: name })
  db.candidates = [
    { objId: k1, selection: 'unscreened', sourcePaths: [pull('Alice Chen')], arrivedAt: now - 1.9 * HOUR, firstAdmittedAt: null, openedAt: null, failOpenTimes: 0 },
    { objId: k2, selection: 'not_selected', sourcePaths: [pull('Sarah Kim'), push('Sarah Kim')], arrivedAt: now - 4.8 * HOUR, firstAdmittedAt: null, openedAt: null, failOpenTimes: 0 },
    { objId: k3, selection: 'preparing', sourcePaths: [pull('David Liu')], arrivedAt: now - 6.5 * HOUR, firstAdmittedAt: null, openedAt: null, failOpenTimes: 0 },
    { objId: k4, selection: 'not_selected', sourcePaths: [push('Bob Zhang')], arrivedAt: now - 7.5 * HOUR, firstAdmittedAt: null, openedAt: null, failOpenTimes: 0 },
    { objId: k5, selection: 'unscreened', sourcePaths: [pull('Alice Chen')], arrivedAt: now - 19 * HOUR, firstAdmittedAt: null, openedAt: null, failOpenTimes: 1 },
    { objId: k6, selection: 'not_selected', sourcePaths: [pull('Sarah Kim')], arrivedAt: now - 29 * HOUR, firstAdmittedAt: null, openedAt: now - 20 * HOUR, failOpenTimes: 0 },
  ]
  db.resources.set(k3, 'preparing')
  db.resources.set(k5, 'unavailable')

  /* ── Sources and intents ── */
  db.sources = [
    { id: 'src-alice', name: 'Alice Chen', kind: 'person', did: ALICE, description: 'Full-stack developer & open source contributor', basis: ['friend', 'active'], notify: 'acknowledged', lastSuccessAt: now - 6 * MIN, paused: false, credibility: 'verified_did', updateMode: 'Home feed changes + push' },
    { id: 'src-bob', name: 'Bob Zhang', kind: 'person', did: BOB, description: 'Friend in Message Center', basis: ['friend'], notify: 'acknowledged', lastSuccessAt: now - 9 * MIN, paused: false, credibility: 'verified_did', updateMode: 'Home feed changes + push' },
    { id: 'src-sarah', name: 'Sarah Kim', kind: 'person', did: SARAH, description: 'Product designer at BuckyOS', basis: ['active'], notify: 'acknowledged', lastSuccessAt: now - 14 * MIN, paused: false, credibility: 'verified_did', updateMode: 'Home feed changes + push' },
    { id: 'src-david', name: 'David Liu', kind: 'person', did: DAVID, description: 'Indie game developer', basis: ['active'], notify: 'retrying', lastSuccessAt: now - 3 * HOUR, lastError: { at: now - 20 * MIN, reason: 'Their OOD is offline; the gateway answered “cached”. The follow notice will be retried.' }, paused: false, credibility: 'verified_did', updateMode: 'Home feed changes' },
    { id: 'src-techcrunch', name: 'TechCrunch', kind: 'rss', url: 'https://techcrunch.com/feed/', description: 'Startup and technology news', basis: ['active'], notify: 'unsupported', lastSuccessAt: now - 31 * MIN, paused: false, credibility: 'known_site', updateMode: 'RSS poll every 30 min' },
    { id: 'src-hn', name: 'Hacker News', kind: 'website', url: 'https://news.ycombinator.com', description: 'Tech news aggregator', basis: ['active'], notify: 'unsupported', lastSuccessAt: now - 2 * HOUR, lastError: { at: now - 12 * MIN, reason: 'Rate limited (HTTP 429). Next attempt in 15 minutes.' }, paused: false, credibility: 'known_site', updateMode: 'Spider crawl every hour' },
    { id: 'src-reddit', name: 'r/programming', kind: 'channel', url: 'https://reddit.com/r/programming', description: 'Programming subreddit', basis: ['active'], notify: 'unsupported', lastSuccessAt: now - 30 * HOUR, paused: true, credibility: 'unknown', updateMode: 'Spider crawl every 2 hours' },
    { id: 'src-ai-digest', name: 'AI Daily Digest', kind: 'rss', url: 'https://aidigest.example/feed.xml', description: 'Daily newsletter about AI', basis: ['active'], notify: 'unsupported', lastSuccessAt: now - 5 * HOUR, paused: false, credibility: 'unknown', updateMode: 'RSS poll daily' },
    { id: 'src-bloomberg', name: 'Bloomberg Markets', kind: 'rss', url: 'https://bloomberg.example/markets.rss', description: 'Financial news', basis: ['active'], notify: 'unsupported', lastSuccessAt: now - 40 * MIN, paused: false, credibility: 'known_site', updateMode: 'RSS poll every 30 min' },
    { id: 'src-sfchronicle', name: 'SF Chronicle', kind: 'website', url: 'https://sfchronicle.com', description: 'Mapped by your agent', basis: ['active'], notify: 'unsupported', lastSuccessAt: now - 50 * MIN, paused: false, intentId: 'intent-sf', credibility: 'known_site', updateMode: 'Agent-maintained crawl' },
    { id: 'src-sfgate', name: 'SFGate', kind: 'website', url: 'https://sfgate.com', description: 'Mapped by your agent', basis: ['active'], notify: 'unsupported', lastSuccessAt: now - 55 * MIN, paused: false, intentId: 'intent-sf', credibility: 'known_site', updateMode: 'Agent-maintained crawl' },
    { id: 'src-511', name: '511.org transit alerts', kind: 'rss', url: 'https://511.org/alerts.rss', description: 'Mapped by your agent', basis: ['active'], notify: 'unsupported', paused: false, intentId: 'intent-sf', credibility: 'known_site', updateMode: 'RSS poll every 10 min', lastError: { at: now - 8 * MIN, reason: 'Feed returned an empty document; the agent is looking for an alternative.' } },
  ]
  db.intents = [{ id: 'intent-sf', text: 'San Francisco local news and Bay Area transit', sourceIds: ['src-sfchronicle', 'src-sfgate', 'src-511'], status: 'collecting', updatedAt: now - 2 * HOUR }]

  if (options.empty) {
    db.reading = []
    db.candidates = []
  }
  return db
}
