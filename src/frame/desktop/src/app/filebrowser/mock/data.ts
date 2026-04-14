import type {
  BrowserTab,
  DeviceNode,
  DfsNode,
  FileBrowserSnapshot,
  FileEntry,
  Topic,
  TriggerRule,
} from '../types'

const publicBaseUrl = 'https://alice.personal.buckyos.dev'

const entries: FileEntry[] = [
  // ─── Home ───
  {
    id: 'home-documents',
    name: 'Documents',
    kind: 'folder',
    path: '/home/Documents',
    modifiedAt: '2026-04-12T09:20:00Z',
    triggersActive: true,
  },
  {
    id: 'home-pictures',
    name: 'Pictures',
    kind: 'folder',
    path: '/home/Pictures',
    modifiedAt: '2026-04-14T14:02:00Z',
    triggersActive: true,
  },
  {
    id: 'home-downloads',
    name: 'Downloads',
    kind: 'folder',
    path: '/home/Downloads',
    modifiedAt: '2026-04-13T23:58:00Z',
  },
  {
    id: 'home-projects',
    name: 'Projects',
    kind: 'folder',
    path: '/home/Projects',
    modifiedAt: '2026-04-11T18:40:00Z',
    triggersActive: true,
  },
  {
    id: 'home-private',
    name: 'Private',
    kind: 'folder',
    path: '/home/Private',
    modifiedAt: '2026-03-28T08:11:00Z',
  },

  // ─── Documents ───
  {
    id: 'doc-trip-plan',
    name: 'Kyoto Trip Plan.md',
    kind: 'document',
    path: '/home/Documents/Kyoto Trip Plan.md',
    sizeBytes: 18_432,
    modifiedAt: '2026-04-10T11:04:00Z',
    summary:
      'Day-by-day itinerary drafted with Mika covering temples, tea houses, and the March cherry-blossom viewing windows.',
    tags: ['trip', 'travel', 'kyoto', 'itinerary'],
    topicIds: ['topic-kyoto'],
    source: { type: 'local', label: 'Drafted locally' },
  },
  {
    id: 'doc-quarter-review',
    name: '2026 Q1 Review.docx',
    kind: 'document',
    path: '/home/Documents/2026 Q1 Review.docx',
    sizeBytes: 204_800,
    modifiedAt: '2026-04-03T16:44:00Z',
    summary: 'Quarterly review covering Personal Server rollout and AI middleware stabilization.',
    tags: ['work', 'quarterly-review'],
    source: { type: 'local', label: 'Uploaded from laptop' },
  },
  {
    id: 'doc-friend-contract',
    name: 'Friend Contract Draft.pdf',
    kind: 'document',
    path: '/home/Documents/Friend Contract Draft.pdf',
    sizeBytes: 512_000,
    modifiedAt: '2026-03-22T13:20:00Z',
    summary: 'PDF shared by Linh via Telegram; revised three times before signing.',
    tags: ['contract', 'friends'],
    topicIds: ['topic-contracts'],
    source: { type: 'telegram', label: 'Telegram · Linh' },
    story: [
      {
        id: 'story-1',
        kind: 'chat',
        title: 'Telegram thread with Linh',
        excerpt: '“Let me know before Friday so legal can countersign.”',
        occurredAt: '2026-03-19T22:05:00Z',
        source: 'Telegram',
      },
    ],
  },

  // ─── Pictures ───
  {
    id: 'pic-kyoto-temple',
    name: 'kyoto-temple-0412.jpg',
    kind: 'image',
    path: '/home/Pictures/Trips/Kyoto/kyoto-temple-0412.jpg',
    sizeBytes: 4_820_000,
    modifiedAt: '2026-04-12T07:12:00Z',
    summary:
      'Early-morning shot of Kiyomizu-dera — soft mist, warm lantern light, strong diagonal lines leading to the stage.',
    tags: ['temple', 'kyoto', 'sunrise', 'architecture'],
    topicIds: ['topic-kyoto'],
    exif: {
      camera: 'Fujifilm X-T5',
      takenAt: '2026-04-12 05:48',
      location: 'Kyoto, Japan · 34.9949° N, 135.7850° E',
      lens: 'XF 16-55mm F2.8',
    },
    source: { type: 'local', label: 'Imported from camera' },
    triggersActive: true,
    story: [
      {
        id: 'story-kyoto-1',
        kind: 'session',
        title: 'Kyoto trip with Mika',
        excerpt:
          'Captured before the temple gate opened; we planned to come back the next morning but it rained.',
        occurredAt: '2026-04-12T06:00:00Z',
      },
      {
        id: 'story-kyoto-2',
        kind: 'share',
        title: 'Shared with Mika',
        excerpt: 'Mika reused this photo as the cover image for the trip write-up.',
        occurredAt: '2026-04-13T11:20:00Z',
      },
    ],
  },
  {
    id: 'pic-group-photo',
    name: 'team-offsite-group.jpg',
    kind: 'image',
    path: '/home/Pictures/Work/team-offsite-group.jpg',
    sizeBytes: 3_210_000,
    modifiedAt: '2026-03-27T18:55:00Z',
    summary: 'Team group photo during the spring offsite — balanced framing with everybody facing camera.',
    tags: ['people', 'group-photo', 'offsite', 'work'],
    topicIds: ['topic-offsite'],
    exif: {
      camera: 'iPhone 16 Pro',
      takenAt: '2026-03-27 18:41',
      location: 'Hangzhou · West Lake Lodge',
    },
    source: { type: 'friend-upload', label: 'Uploaded by Hiro via shared folder' },
  },
  {
    id: 'pic-sketch',
    name: 'design-sketch.png',
    kind: 'image',
    path: '/home/Pictures/sketches/design-sketch.png',
    sizeBytes: 860_000,
    modifiedAt: '2026-04-09T14:22:00Z',
    tags: ['sketch', 'design'],
    source: { type: 'local', label: 'Exported from tablet' },
  },

  // ─── Public folder files ───
  {
    id: 'pub-resume',
    name: 'resume.pdf',
    kind: 'document',
    path: '/public/resume.pdf',
    publicUrl: `${publicBaseUrl}/public/resume.pdf`,
    sizeBytes: 186_000,
    modifiedAt: '2026-03-02T12:00:00Z',
    summary: 'Resume shared publicly for conference bio requests.',
    tags: ['public', 'resume'],
    source: { type: 'local', label: 'Synced from laptop' },
    triggersActive: false,
  },
  {
    id: 'pub-talk-slides',
    name: 'personal-server-talk.pdf',
    kind: 'document',
    path: '/public/talks/personal-server-talk.pdf',
    publicUrl: `${publicBaseUrl}/public/talks/personal-server-talk.pdf`,
    sizeBytes: 2_320_000,
    modifiedAt: '2026-03-18T09:44:00Z',
    summary: 'Slide deck for the Personal Server meetup talk — public mirror.',
    tags: ['public', 'talk', 'slides'],
    topicIds: ['topic-contracts'],
    source: { type: 'local', label: 'Exported from Keynote' },
  },

  // ─── Shared incoming ───
  {
    id: 'shared-invoice',
    name: 'march-invoice.xlsx',
    kind: 'document',
    path: '/shared/incoming/march-invoice.xlsx',
    sizeBytes: 92_000,
    modifiedAt: '2026-03-31T20:04:00Z',
    summary: 'Invoice uploaded by accounting via the shared folder quota.',
    tags: ['work', 'invoice'],
    source: { type: 'shared', label: 'Accounting · shared drop' },
  },

  // ─── Code ───
  {
    id: 'code-buckyos',
    name: 'buckyos-notes.md',
    kind: 'code',
    path: '/home/Projects/buckyos/buckyos-notes.md',
    sizeBytes: 14_800,
    modifiedAt: '2026-04-11T17:22:00Z',
    summary: 'Working notes on the BuckyOS frame rewrite.',
    tags: ['work', 'notes'],
    source: { type: 'local', label: 'Hand-written' },
  },

  // ─── Downloads ───
  {
    id: 'dl-report',
    name: 'market-report-2026.pdf',
    kind: 'document',
    path: '/home/Downloads/market-report-2026.pdf',
    sizeBytes: 3_180_000,
    modifiedAt: '2026-04-13T23:58:00Z',
    source: { type: 'local', label: 'Downloaded from browser' },
  },

  // ─── Sub folders (for tree expansion) ───
  {
    id: 'pic-trips',
    name: 'Trips',
    kind: 'folder',
    path: '/home/Pictures/Trips',
    modifiedAt: '2026-04-14T07:22:00Z',
  },
  {
    id: 'pic-trips-kyoto',
    name: 'Kyoto',
    kind: 'folder',
    path: '/home/Pictures/Trips/Kyoto',
    modifiedAt: '2026-04-14T07:22:00Z',
  },
  {
    id: 'pic-work',
    name: 'Work',
    kind: 'folder',
    path: '/home/Pictures/Work',
    modifiedAt: '2026-03-27T18:55:00Z',
  },
  {
    id: 'pic-sketches',
    name: 'sketches',
    kind: 'folder',
    path: '/home/Pictures/sketches',
    modifiedAt: '2026-04-09T14:22:00Z',
  },
  {
    id: 'projects-buckyos',
    name: 'buckyos',
    kind: 'folder',
    path: '/home/Projects/buckyos',
    modifiedAt: '2026-04-11T17:22:00Z',
  },
  {
    id: 'public-talks',
    name: 'talks',
    kind: 'folder',
    path: '/public/talks',
    modifiedAt: '2026-03-18T09:44:00Z',
  },
  {
    id: 'shared-incoming',
    name: 'incoming',
    kind: 'folder',
    path: '/shared/incoming',
    modifiedAt: '2026-03-31T20:04:00Z',
  },
]

function buildIndex(list: FileEntry[]) {
  const byPath: Record<string, FileEntry[]> = {}
  const byId: Record<string, FileEntry> = {}
  for (const entry of list) {
    byId[entry.id] = entry
    const parent = entry.path.split('/').slice(0, -1).join('/') || '/'
    const key = parent === '' ? '/' : parent
    if (!byPath[key]) byPath[key] = []
    byPath[key].push(entry)
  }
  // Ensure each known parent folder has at least an empty bucket.
  for (const entry of list) {
    if (entry.kind === 'folder' && !byPath[entry.path]) {
      byPath[entry.path] = []
    }
  }
  return { byPath, byId }
}

const dfsRoots: DfsNode[] = [
  {
    id: 'dfs-home',
    name: 'Home',
    path: '/home',
    kind: 'home',
    children: [
      { id: 'dfs-home-docs', name: 'Documents', path: '/home/Documents', kind: 'generic' },
      {
        id: 'dfs-home-pics',
        name: 'Pictures',
        path: '/home/Pictures',
        kind: 'generic',
        children: [
          {
            id: 'dfs-home-pics-trips',
            name: 'Trips',
            path: '/home/Pictures/Trips',
            kind: 'generic',
            children: [
              {
                id: 'dfs-home-pics-trips-kyoto',
                name: 'Kyoto',
                path: '/home/Pictures/Trips/Kyoto',
                kind: 'generic',
              },
            ],
          },
          { id: 'dfs-home-pics-work', name: 'Work', path: '/home/Pictures/Work', kind: 'generic' },
          {
            id: 'dfs-home-pics-sketches',
            name: 'sketches',
            path: '/home/Pictures/sketches',
            kind: 'generic',
          },
        ],
      },
      { id: 'dfs-home-downloads', name: 'Downloads', path: '/home/Downloads', kind: 'generic' },
      {
        id: 'dfs-home-projects',
        name: 'Projects',
        path: '/home/Projects',
        kind: 'generic',
        children: [
          {
            id: 'dfs-home-projects-buckyos',
            name: 'buckyos',
            path: '/home/Projects/buckyos',
            kind: 'generic',
          },
        ],
      },
      { id: 'dfs-home-private', name: 'Private', path: '/home/Private', kind: 'privacy' },
    ],
  },
  {
    id: 'dfs-public',
    name: 'Public',
    path: '/public',
    kind: 'public',
    children: [
      { id: 'dfs-public-talks', name: 'talks', path: '/public/talks', kind: 'public' },
    ],
  },
  {
    id: 'dfs-shared',
    name: 'Shared',
    path: '/shared',
    kind: 'shared',
    children: [
      {
        id: 'dfs-shared-incoming',
        name: 'incoming',
        path: '/shared/incoming',
        kind: 'shared',
      },
    ],
  },
]

const devices: DeviceNode[] = [
  {
    id: 'device-alpha',
    name: 'Workstation Alpha',
    host: 'alpha.local',
    status: 'online',
    roots: [
      { path: '/opt', label: '/opt' },
      { path: '/usr/local', label: '/usr/local' },
    ],
  },
  {
    id: 'device-node-2',
    name: 'Node 2 · NAS',
    host: 'nas-2.local',
    status: 'syncing',
    roots: [{ path: '/mnt/pool0', label: '/mnt/pool0' }],
  },
  {
    id: 'device-mini',
    name: 'Travel Mini',
    host: 'mini.local',
    status: 'offline',
    roots: [{ path: '/data', label: '/data' }],
  },
]

const topics: Topic[] = [
  {
    id: 'topic-kyoto',
    title: 'Kyoto trip · April',
    description: '6 days across Kyoto and Osaka with Mika.',
    reason: 'Detected from recent photos, chat history with Mika, and an itinerary document.',
    coverageCount: 42,
    updatedAt: '2026-04-13T21:00:00Z',
    groups: [
      {
        id: 'topic-kyoto-source',
        label: 'Sources',
        axis: 'source',
        fileIds: ['pic-kyoto-temple', 'doc-trip-plan'],
      },
      {
        id: 'topic-kyoto-location',
        label: 'Locations',
        axis: 'location',
        fileIds: ['pic-kyoto-temple'],
      },
      {
        id: 'topic-kyoto-kind',
        label: 'Kinds',
        axis: 'kind',
        fileIds: ['pic-kyoto-temple', 'doc-trip-plan'],
      },
    ],
  },
  {
    id: 'topic-offsite',
    title: 'Spring Offsite',
    description: 'Photos + retro notes from the West Lake offsite.',
    reason: 'Grouped by date window and shared folder activity from Hiro.',
    coverageCount: 18,
    updatedAt: '2026-03-29T10:00:00Z',
    groups: [
      {
        id: 'topic-offsite-people',
        label: 'People',
        axis: 'people',
        fileIds: ['pic-group-photo'],
      },
      {
        id: 'topic-offsite-source',
        label: 'Sources',
        axis: 'source',
        fileIds: ['pic-group-photo'],
      },
    ],
  },
  {
    id: 'topic-contracts',
    title: 'Contracts & Talks · March',
    description: 'Legal PDFs and talk decks bundled for the March cycle.',
    reason: 'Linked by Telegram sender and shared folder naming.',
    coverageCount: 9,
    updatedAt: '2026-03-22T13:30:00Z',
    groups: [
      {
        id: 'topic-contracts-source',
        label: 'Sources',
        axis: 'source',
        fileIds: ['doc-friend-contract', 'pub-talk-slides'],
      },
    ],
  },
]

const triggers: TriggerRule[] = [
  {
    id: 'trigger-new-upload',
    name: 'Knowledge base — ingest on upload',
    event: 'on_new_file_upload',
    appliesTo: ['/home/Documents', '/home/Projects', '/home/Pictures'],
    pipeline: 'kb.ingest.default',
    enabled: true,
    reason:
      'Documents, Projects and Pictures feed the default knowledge base pipeline for semantic search.',
  },
  {
    id: 'trigger-topic-created',
    name: 'Summarize topics',
    event: 'on_new_topic_created',
    appliesTo: ['*'],
    pipeline: 'ai.topic.summarize',
    enabled: true,
    reason: 'Auto-summarize newly discovered topics into a short description.',
  },
  {
    id: 'trigger-private-block',
    name: 'Private — skip AI processing',
    event: 'on_new_file_upload',
    appliesTo: ['/home/Private'],
    pipeline: 'kb.ingest.default',
    enabled: false,
    reason: 'Private folder is excluded from any AI pipeline by privacy policy.',
  },
]

const { byPath, byId } = buildIndex(entries)

export const fileBrowserSnapshot: FileBrowserSnapshot = {
  dfsRoots,
  devices,
  topics,
  triggers,
  entriesByPath: byPath,
  entriesById: byId,
}

export const defaultTab: BrowserTab = {
  id: 'tab-home',
  title: 'Home',
  path: '/home',
}

export const defaultTabs: BrowserTab[] = [
  defaultTab,
  { id: 'tab-pictures', title: 'Pictures', path: '/home/Pictures' },
]

/** Search over the mock snapshot — returns grouped hits with explainability. */
export function searchFiles(query: string) {
  const q = query.trim().toLowerCase()
  if (!q) {
    return [] as { entry: FileEntry; reason: string; detail: string }[]
  }

  const hits: { entry: FileEntry; reason: string; detail: string }[] = []
  for (const entry of entries) {
    if (entry.kind === 'folder') {
      if (entry.name.toLowerCase().includes(q)) {
        hits.push({ entry, reason: 'folder', detail: `Folder name contains “${query}”` })
      }
      continue
    }
    if (entry.name.toLowerCase().includes(q)) {
      hits.push({ entry, reason: 'filename', detail: `File name match — ${entry.name}` })
      continue
    }
    const tagHit = entry.tags?.find((tag) => tag.toLowerCase().includes(q))
    if (tagHit) {
      hits.push({ entry, reason: 'ai_semantic', detail: `Tag match — #${tagHit}` })
      continue
    }
    if (entry.summary?.toLowerCase().includes(q)) {
      hits.push({
        entry,
        reason: 'ai_semantic',
        detail: 'AI summary mentions this term',
      })
      continue
    }
    if (entry.story?.some((story) => story.excerpt.toLowerCase().includes(q))) {
      hits.push({
        entry,
        reason: 'ai_topic',
        detail: 'Found inside an attached story excerpt',
      })
    }
  }
  return hits
}
