import {
  getMessageStableId,
  getMessageStatusType,
  isStatusMessageObject,
  type MessageObject,
} from '../../protocol/msgobj'
import type {
  AppendableConversationMessageReader,
  ConversationListIndexEntry,
  ConversationListItem,
  ConversationMaterializedWindow,
  ConversationMessageReader,
  ConversationProjection,
  ConversationStatusDescriptor,
} from './types'

const DEFAULT_PAGE_SIZE = 32
const INDEX_SCAN_PAGE_SIZE = 128
const TIMESTAMP_GAP_MS = 30 * 60_000

export class InMemoryConversationMessageReader
implements AppendableConversationMessageReader {
  readonly readerKey: string
  readonly totalCount: number
  private readonly pages: ReadonlyMap<number, readonly MessageObject[]>
  private readonly pageSize: number

  constructor(
    pages: ReadonlyMap<number, readonly MessageObject[]>,
    pageSize = DEFAULT_PAGE_SIZE,
    totalCount?: number,
    readerKey = 'memory:default',
  ) {
    this.pages = pages
    this.pageSize = pageSize
    this.totalCount = totalCount ?? countMessagesInPages(pages)
    this.readerKey = readerKey
  }

  static empty(pageSize = DEFAULT_PAGE_SIZE, readerKey = 'memory:empty') {
    return new InMemoryConversationMessageReader(new Map(), pageSize, 0, readerKey)
  }

  static fromMessages(
    messages: readonly MessageObject[],
    pageSize = DEFAULT_PAGE_SIZE,
    readerKey = deriveMemoryReaderKey(messages),
  ) {
    const pages = new Map<number, readonly MessageObject[]>()

    for (let start = 0; start < messages.length; start += pageSize) {
      pages.set(start / pageSize, messages.slice(start, start + pageSize))
    }

    return new InMemoryConversationMessageReader(
      pages,
      pageSize,
      messages.length,
      readerKey,
    )
  }

  append(message: MessageObject) {
    const nextPages = new Map(this.pages)
    const pageIndex = Math.floor(this.totalCount / this.pageSize)
    const pageOffset = this.totalCount % this.pageSize
    const currentPage = nextPages.get(pageIndex) ?? []

    if (pageOffset === 0) {
      nextPages.set(pageIndex, [message])
    } else {
      nextPages.set(pageIndex, [...currentPage, message])
    }

    return new InMemoryConversationMessageReader(
      nextPages,
      this.pageSize,
      this.totalCount + 1,
      this.readerKey,
    )
  }

  async readRange(startIndex: number, count: number) {
    if (count <= 0 || startIndex >= this.totalCount) {
      return []
    }

    const safeStart = Math.max(0, startIndex)
    const safeEnd = Math.min(this.totalCount, safeStart + count)
    const items: MessageObject[] = []

    for (let index = safeStart; index < safeEnd; index += 1) {
      const pageIndex = Math.floor(index / this.pageSize)
      const pageOffset = index % this.pageSize
      const page = this.pages.get(pageIndex)
      const item = page?.[pageOffset]

      if (item) {
        items.push(item)
      }
    }

    return items
  }
}

interface ConversationStorageMeta {
  version: string
  totalCount: number
  updatedAtMs: number
}

interface IndexedDbReaderInit {
  databaseName: string
  namespace: string
  sessionId: string
  totalCount: number
  pageSize?: number
  version?: string
  overlays?: ReadonlyMap<number, MessageObject>
}

interface IndexedDbSeedInit {
  databaseName: string
  namespace: string
  sessionId: string
  messages: readonly MessageObject[]
  pageSize?: number
  version: string
}

export class IndexedDbConversationMessageReader
implements AppendableConversationMessageReader {
  readonly readerKey: string
  readonly totalCount: number
  private readonly databaseName: string
  private readonly namespace: string
  private readonly sessionId: string
  private readonly pageSize: number
  private readonly version: string
  private readonly overlays: ReadonlyMap<number, MessageObject>
  private readonly dbPromise: Promise<IDBDatabase>

  constructor({
    databaseName,
    namespace,
    sessionId,
    totalCount,
    pageSize = DEFAULT_PAGE_SIZE,
    version = 'v1',
    overlays = new Map(),
  }: IndexedDbReaderInit) {
    this.databaseName = databaseName
    this.namespace = namespace
    this.sessionId = sessionId
    this.totalCount = totalCount
    this.pageSize = pageSize
    this.version = version
    this.overlays = overlays
    this.dbPromise = openConversationHistoryDatabase(databaseName)
    this.readerKey = `indexeddb:${databaseName}:${namespace}:${sessionId}`
  }

  static seed({
    databaseName,
    namespace,
    sessionId,
    messages,
    pageSize = DEFAULT_PAGE_SIZE,
    version,
  }: IndexedDbSeedInit) {
    return seedIndexedDbConversationSession({
      databaseName,
      namespace,
      sessionId,
      messages,
      version,
    }).then((meta) => new IndexedDbConversationMessageReader({
      databaseName,
      namespace,
      sessionId,
      totalCount: meta.totalCount,
      pageSize,
      version: meta.version,
    }))
  }

  append(message: MessageObject) {
    const nextIndex = this.totalCount
    const nextOverlays = new Map(this.overlays)
    nextOverlays.set(nextIndex, message)

    void this.persistAppend(nextIndex, message)

    return new IndexedDbConversationMessageReader({
      databaseName: this.databaseName,
      namespace: this.namespace,
      sessionId: this.sessionId,
      totalCount: this.totalCount + 1,
      pageSize: this.pageSize,
      version: this.version,
      overlays: nextOverlays,
    })
  }

  async readRange(startIndex: number, count: number) {
    if (count <= 0 || startIndex >= this.totalCount) {
      return []
    }

    const safeStart = Math.max(0, startIndex)
    const safeEnd = Math.min(this.totalCount, safeStart + count)
    const db = await this.dbPromise
    const items = new Map<number, MessageObject>()

    const lowerBound = [this.namespace, this.sessionId, safeStart]
    const upperBound = [this.namespace, this.sessionId, safeEnd - 1]
    const range = IDBKeyRange.bound(lowerBound, upperBound)
    const transaction = db.transaction(messageStoreName, 'readonly')
    const store = transaction.objectStore(messageStoreName)
    const records = await requestToPromise<
      Array<{ namespace: string; sessionId: string; index: number; message: MessageObject }>
    >(store.getAll(range))

    records.forEach((record) => {
      items.set(record.index, record.message)
    })

    for (let index = safeStart; index < safeEnd; index += 1) {
      const overlay = this.overlays.get(index)
      if (overlay) {
        items.set(index, overlay)
      }
    }

    const ordered: MessageObject[] = []
    for (let index = safeStart; index < safeEnd; index += 1) {
      const item = items.get(index)
      if (item) {
        ordered.push(item)
      }
    }

    return ordered
  }

  private async persistAppend(index: number, message: MessageObject) {
    const db = await this.dbPromise
    const transaction = db.transaction(
      [metaStoreName, messageStoreName],
      'readwrite',
    )
    const metaStore = transaction.objectStore(metaStoreName)
    const messageStore = transaction.objectStore(messageStoreName)

    messageStore.put({
      namespace: this.namespace,
      sessionId: this.sessionId,
      index,
      message,
    })
    metaStore.put({
      namespace: this.namespace,
      sessionId: this.sessionId,
      version: this.version,
      totalCount: this.totalCount + 1,
      updatedAtMs: Date.now(),
    } satisfies ConversationStorageMetaRecord)

    await waitForTransaction(transaction)
  }
}

interface ConversationStorageMetaRecord extends ConversationStorageMeta {
  namespace: string
  sessionId: string
}

const databaseVersion = 1
const metaStoreName = 'conversation_meta'
const messageStoreName = 'conversation_messages'

function openConversationHistoryDatabase(databaseName: string) {
  return new Promise<IDBDatabase>((resolve, reject) => {
    const request = window.indexedDB.open(databaseName, databaseVersion)

    request.onerror = () => {
      reject(request.error ?? new Error('Failed to open IndexedDB'))
    }
    request.onupgradeneeded = () => {
      const db = request.result

      if (!db.objectStoreNames.contains(metaStoreName)) {
        const metaStore = db.createObjectStore(metaStoreName, {
          keyPath: ['namespace', 'sessionId'],
        })
        metaStore.createIndex('by_namespace', 'namespace', { unique: false })
      }

      if (!db.objectStoreNames.contains(messageStoreName)) {
        const messageStore = db.createObjectStore(messageStoreName, {
          keyPath: ['namespace', 'sessionId', 'index'],
        })
        messageStore.createIndex('by_session', ['namespace', 'sessionId'], {
          unique: false,
        })
      }
    }
    request.onsuccess = () => {
      resolve(request.result)
    }
  })
}

async function seedIndexedDbConversationSession({
  databaseName,
  namespace,
  sessionId,
  messages,
  version,
}: {
  databaseName: string
  namespace: string
  sessionId: string
  messages: readonly MessageObject[]
  version: string
}) {
  const db = await openConversationHistoryDatabase(databaseName)
  const existingMeta = await readConversationMeta(db, namespace, sessionId)

  if (existingMeta?.version === version && existingMeta.totalCount > 0) {
    return existingMeta
  }

  await clearIndexedDbConversationSession(db, namespace, sessionId)

  const batchSize = 250
  for (let start = 0; start < messages.length; start += batchSize) {
    const transaction = db.transaction(messageStoreName, 'readwrite')
    const messageStore = transaction.objectStore(messageStoreName)

    messages.slice(start, start + batchSize).forEach((message, offset) => {
      messageStore.put({
        namespace,
        sessionId,
        index: start + offset,
        message,
      })
    })

    await waitForTransaction(transaction)
  }

  const metaRecord = {
    namespace,
    sessionId,
    version,
    totalCount: messages.length,
    updatedAtMs: Date.now(),
  } satisfies ConversationStorageMetaRecord
  const metaTransaction = db.transaction(metaStoreName, 'readwrite')
  metaTransaction.objectStore(metaStoreName).put(metaRecord)
  await waitForTransaction(metaTransaction)

  return metaRecord
}

export async function buildConversationProjection(
  reader: ConversationMessageReader,
  statusItems: readonly ConversationStatusDescriptor[] = [],
): Promise<ConversationProjection> {
  const entries: ConversationListIndexEntry[] = []
  const { headStatuses, tailStatuses } = splitStatusItems(statusItems)
  appendStatuses(entries, headStatuses)

  let previousMessage: MessageObject | undefined

  for (let startIndex = 0; startIndex < reader.totalCount; startIndex += INDEX_SCAN_PAGE_SIZE) {
    const messages = await reader.readRange(startIndex, INDEX_SCAN_PAGE_SIZE)

    messages.forEach((message, offset) => {
      const messageIndex = startIndex + offset
      previousMessage = appendMessageEntries(
        entries,
        message,
        messageIndex,
        previousMessage,
      )
    })
  }

  appendStatuses(entries, tailStatuses)

  return {
    readerKey: reader.readerKey,
    messageCount: reader.totalCount,
    tailStatusCount: tailStatuses.length,
    statusItemsSignature: getStatusItemsSignature(statusItems),
    lastMessage: previousMessage,
    totalCount: entries.length,
    entries,
  }
}

export function extendConversationProjection(
  projection: ConversationProjection,
  appendedMessages: readonly MessageObject[],
  statusItems: readonly ConversationStatusDescriptor[] = [],
): ConversationProjection {
  if (appendedMessages.length === 0) {
    return projection
  }

  const { tailStatuses } = splitStatusItems(statusItems)
  const baseEntries = projection.tailStatusCount > 0
    ? projection.entries.slice(0, projection.entries.length - projection.tailStatusCount)
    : projection.entries.slice()
  let previousMessage = projection.lastMessage
  let messageIndex = projection.messageCount

  appendedMessages.forEach((message) => {
    previousMessage = appendMessageEntries(
      baseEntries,
      message,
      messageIndex,
      previousMessage,
    )
    messageIndex += 1
  })

  appendStatuses(baseEntries, tailStatuses)

  return {
    readerKey: projection.readerKey,
    messageCount: projection.messageCount + appendedMessages.length,
    tailStatusCount: tailStatuses.length,
    statusItemsSignature: getStatusItemsSignature(statusItems),
    lastMessage: previousMessage,
    totalCount: baseEntries.length,
    entries: baseEntries,
  }
}

export async function materializeConversationWindow(
  projection: ConversationProjection,
  reader: ConversationMessageReader,
  startIndex: number,
  endIndex: number,
): Promise<ConversationMaterializedWindow> {
  const safeStart = Math.max(0, startIndex)
  const safeEnd = Math.min(projection.totalCount, endIndex)
  const windowEntries = projection.entries.slice(safeStart, safeEnd)
  const ranges = groupContiguousMessageRanges(windowEntries)
  const loadedRanges = await Promise.all(
    ranges.map(async (range) => ({
      start: range.start,
      items: await reader.readRange(range.start, range.count),
    })),
  )
  const messageByIndex = new Map<number, MessageObject>()

  loadedRanges.forEach(({ start, items }) => {
    items.forEach((item, offset) => {
      messageByIndex.set(start + offset, item)
    })
  })

  const items = windowEntries.reduce<ConversationListItem[]>((result, entry, offset) => {
    const index = safeStart + offset

    if (entry.kind === 'timestamp') {
      result.push({
        kind: 'timestamp',
        key: entry.key,
        index,
        date: new Date(entry.dateMs),
      })
      return result
    }

    if (entry.kind === 'status') {
      result.push({
        kind: 'status',
        key: entry.key,
        index,
        status: entry.status,
        label: entry.label,
        createdAtMs: entry.createdAtMs,
      })
      return result
    }

    const message = messageByIndex.get(entry.messageIndex)
    if (message) {
      result.push({
        kind: 'message',
        key: entry.key,
        index,
        messageIndex: entry.messageIndex,
        data: message,
      })
    }

    return result
  }, [])

  return {
    startIndex: safeStart,
    endIndex: safeEnd,
    items,
  }
}

function shouldInsertTimestamp(
  current: MessageObject,
  previous?: MessageObject,
) {
  if (!previous) {
    return true
  }

  const currentDate = new Date(current.created_at_ms).toDateString()
  const previousDate = new Date(previous.created_at_ms).toDateString()

  return (
    currentDate !== previousDate
    || current.created_at_ms - previous.created_at_ms >= TIMESTAMP_GAP_MS
  )
}

function appendMessageEntries(
  entries: ConversationListIndexEntry[],
  message: MessageObject,
  messageIndex: number,
  previousMessage?: MessageObject,
) {
  if (shouldInsertTimestamp(message, previousMessage)) {
    entries.push({
      kind: 'timestamp',
      key: `ts:${message.created_at_ms}:${messageIndex}`,
      dateMs: message.created_at_ms,
      anchorMessageIndex: messageIndex,
    })
  }

  if (isStatusMessageObject(message)) {
    entries.push({
      kind: 'status',
      key: `message-status:${getMessageStableId(message, messageIndex)}`,
      status: getMessageStatusType(message) ?? 'info',
      label: message.content.content,
      anchorMessageIndex: messageIndex,
      createdAtMs: message.created_at_ms,
    })
    return message
  }

  entries.push({
    kind: 'message',
    key: `message:${getMessageStableId(message, messageIndex)}`,
    messageIndex,
  })
  return message
}

function appendStatuses(
  entries: ConversationListIndexEntry[],
  statuses: readonly ConversationStatusDescriptor[],
) {
  statuses.forEach((status) => {
    entries.push({
      kind: 'status',
      key: `status:${status.id}`,
      status: status.status,
      label: status.label,
      createdAtMs: status.createdAtMs,
    })
  })
}

function splitStatusItems(statusItems: readonly ConversationStatusDescriptor[]) {
  return {
    headStatuses: statusItems.filter((item) => item.position === 'head'),
    tailStatuses: statusItems.filter(
      (item) => !item.position || item.position === 'tail',
    ),
  }
}

function getStatusItemsSignature(
  statusItems: readonly ConversationStatusDescriptor[],
) {
  return statusItems.map((item) => (
    `${item.id}:${item.position ?? 'tail'}:${item.status}:${item.label}:${item.createdAtMs ?? ''}`
  )).join('|')
}

function groupContiguousMessageRanges(
  entries: readonly ConversationListIndexEntry[],
) {
  const messageIndexes = entries
    .filter((entry): entry is Extract<ConversationListIndexEntry, { kind: 'message' }> => (
      entry.kind === 'message'
    ))
    .map((entry) => entry.messageIndex)

  if (messageIndexes.length === 0) {
    return []
  }

  const ranges: Array<{ start: number; count: number }> = []
  let rangeStart = messageIndexes[0]
  let rangeEnd = rangeStart + 1

  for (let index = 1; index < messageIndexes.length; index += 1) {
    const messageIndex = messageIndexes[index]

    if (messageIndex === rangeEnd) {
      rangeEnd += 1
      continue
    }

    ranges.push({ start: rangeStart, count: rangeEnd - rangeStart })
    rangeStart = messageIndex
    rangeEnd = messageIndex + 1
  }

  ranges.push({ start: rangeStart, count: rangeEnd - rangeStart })
  return ranges
}

function countMessagesInPages(
  pages: ReadonlyMap<number, readonly MessageObject[]>,
) {
  let total = 0

  pages.forEach((page) => {
    total += page.length
  })

  return total
}

function deriveMemoryReaderKey(messages: readonly MessageObject[]) {
  const firstMessage = messages[0]
  const lastMessage = messages[messages.length - 1]

  if (!firstMessage || !lastMessage) {
    return 'memory:empty'
  }

  const sessionId = firstMessage.ui_session_id ?? 'unknown-session'
  return `memory:${sessionId}:${getMessageStableId(firstMessage, 0)}:${getMessageStableId(lastMessage, messages.length - 1)}`
}

async function readConversationMeta(
  db: IDBDatabase,
  namespace: string,
  sessionId: string,
) {
  const transaction = db.transaction(metaStoreName, 'readonly')
  const metaStore = transaction.objectStore(metaStoreName)
  const meta = await requestToPromise<ConversationStorageMetaRecord | undefined>(
    metaStore.get([namespace, sessionId]),
  )
  return meta ?? null
}

async function clearIndexedDbConversationSession(
  db: IDBDatabase,
  namespace: string,
  sessionId: string,
) {
  const transaction = db.transaction(
    [metaStoreName, messageStoreName],
    'readwrite',
  )
  transaction.objectStore(metaStoreName).delete([namespace, sessionId])
  transaction.objectStore(messageStoreName).delete(
    IDBKeyRange.bound(
      [namespace, sessionId, 0],
      [namespace, sessionId, Number.MAX_SAFE_INTEGER],
    ),
  )
  await waitForTransaction(transaction)
}

function requestToPromise<T>(request: IDBRequest<T>) {
  return new Promise<T>((resolve, reject) => {
    request.onerror = () => {
      reject(request.error ?? new Error('IndexedDB request failed'))
    }
    request.onsuccess = () => {
      resolve(request.result)
    }
  })
}

function waitForTransaction(transaction: IDBTransaction) {
  return new Promise<void>((resolve, reject) => {
    transaction.onerror = () => {
      reject(transaction.error ?? new Error('IndexedDB transaction failed'))
    }
    transaction.onabort = () => {
      reject(transaction.error ?? new Error('IndexedDB transaction aborted'))
    }
    transaction.oncomplete = () => {
      resolve()
    }
  })
}
