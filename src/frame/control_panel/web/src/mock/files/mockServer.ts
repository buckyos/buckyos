import {
  CONTROL_PANEL_MOCK_SESSION_TOKEN,
  CONTROL_PANEL_MOCK_USERNAME,
  isMockRuntime,
  waitForMockLatency,
} from '@/config/runtime'

type MockDirectoryNode = {
  kind: 'dir'
  path: string
  modified: number
}

type MockTextFileNode = {
  kind: 'text'
  path: string
  modified: number
  mime: string
  content: string
}

type MockBinaryFileNode = {
  kind: 'binary'
  path: string
  modified: number
  mime: string
  bytes: Uint8Array
}

type MockNode = MockDirectoryNode | MockTextFileNode | MockBinaryFileNode

type MockShare = {
  id: string
  owner: string
  path: string
  created_at: number
  expires_at?: number | null
  password_required: boolean
  password?: string
}

type MockRecentEntry = {
  path: string
  last_accessed_at: number
  access_count: number
}

type MockTrashEntry = {
  item_id: string
  path: string
  original_path: string
  deleted_at: number
  node: MockNode
}

type MockUploadSession = {
  session: {
    id: string
    owner: string
    path: string
    size: number
    chunk_size: number
    uploaded_size: number
    override_existing: boolean
    created_at: number
    updated_at: number
  }
  chunks: Uint8Array[]
}

type MockFilesState = {
  nodes: Map<string, MockNode>
  favorites: Set<string>
  recent: Map<string, MockRecentEntry>
  recycleBin: Map<string, MockTrashEntry>
  shares: Map<string, MockShare>
  uploads: Map<string, MockUploadSession>
}

const encoder = new TextEncoder()
let installed = false
let state: MockFilesState | null = null

const nowSeconds = () => Math.floor(Date.now() / 1000)

const normalizePath = (path: string) => {
  if (!path || path.trim() === '') {
    return '/'
  }
  let normalized = path.trim()
  if (!normalized.startsWith('/')) {
    normalized = `/${normalized}`
  }
  normalized = normalized.replace(/\/{2,}/g, '/')
  if (normalized.length > 1 && normalized.endsWith('/')) {
    normalized = normalized.slice(0, -1)
  }
  return normalized || '/'
}

const parentPath = (path: string) => {
  const normalized = normalizePath(path)
  if (normalized === '/') {
    return '/'
  }
  const index = normalized.lastIndexOf('/')
  return index <= 0 ? '/' : normalized.slice(0, index)
}

const fileNameFromPath = (path: string) => {
  const parts = normalizePath(path).split('/').filter(Boolean)
  return parts[parts.length - 1] ?? ''
}

const joinPath = (base: string, name: string) => {
  const normalizedBase = normalizePath(base)
  return normalizedBase === '/' ? `/${name}` : `${normalizedBase}/${name}`
}

const isChildOf = (candidate: string, parent: string) => {
  const normalizedCandidate = normalizePath(candidate)
  const normalizedParent = normalizePath(parent)
  if (normalizedParent === '/') {
    return normalizedCandidate !== '/' && parentPath(normalizedCandidate) === '/'
  }
  return parentPath(normalizedCandidate) === normalizedParent
}

const isInSubtree = (candidate: string, root: string) => {
  const normalizedCandidate = normalizePath(candidate)
  const normalizedRoot = normalizePath(root)
  if (normalizedRoot === '/') {
    return normalizedCandidate !== '/'
  }
  return normalizedCandidate === normalizedRoot || normalizedCandidate.startsWith(`${normalizedRoot}/`)
}

const getNodeSize = (node: MockNode) => {
  if (node.kind === 'dir') {
    return 0
  }
  if (node.kind === 'text') {
    return encoder.encode(node.content).byteLength
  }
  return node.bytes.byteLength
}

const getNodeMime = (node: MockNode) => {
  if (node.kind === 'dir') {
    return 'inode/directory'
  }
  return node.mime
}

const makeFileEntry = (node: MockNode) => ({
  name: fileNameFromPath(node.path),
  path: normalizePath(node.path),
  is_dir: node.kind === 'dir',
  size: getNodeSize(node),
  modified: node.modified,
})

const jsonResponse = (body: unknown, status = 200) =>
  new Response(JSON.stringify(body), {
    status,
    headers: {
      'content-type': 'application/json',
      'cache-control': 'no-store',
    },
  })

const errorResponse = (status: number, message: string) => jsonResponse({ error: message }, status)

const createSvgBytes = (label: string, accent = '#0f766e') =>
  encoder.encode(
    `<svg xmlns="http://www.w3.org/2000/svg" width="640" height="420" viewBox="0 0 640 420">
      <defs>
        <linearGradient id="bg" x1="0%" y1="0%" x2="100%" y2="100%">
          <stop offset="0%" stop-color="#ecfeff" />
          <stop offset="100%" stop-color="#ccfbf1" />
        </linearGradient>
      </defs>
      <rect width="640" height="420" fill="url(#bg)" rx="28" />
      <circle cx="140" cy="126" r="64" fill="${accent}" opacity="0.14" />
      <circle cx="520" cy="308" r="96" fill="${accent}" opacity="0.12" />
      <rect x="72" y="80" width="496" height="260" rx="24" fill="#ffffff" stroke="#d7e1df" />
      <text x="102" y="182" fill="#0f172a" font-size="34" font-family="Work Sans, sans-serif" font-weight="700">${label}</text>
      <text x="102" y="228" fill="#52606d" font-size="20" font-family="Work Sans, sans-serif">BuckyOS mock image preview</text>
    </svg>`,
  )

const createInitialState = (): MockFilesState => {
  const seedTime = nowSeconds()
  const nodes = new Map<string, MockNode>()
  const addNode = (node: MockNode) => {
    nodes.set(normalizePath(node.path), {
      ...node,
      path: normalizePath(node.path),
    })
  }

  addNode({ kind: 'dir', path: '/', modified: seedTime - 7200 })
  addNode({ kind: 'dir', path: '/Documents', modified: seedTime - 7200 })
  addNode({ kind: 'dir', path: '/Projects', modified: seedTime - 5400 })
  addNode({ kind: 'dir', path: '/Projects/ControlPanel', modified: seedTime - 3600 })
  addNode({ kind: 'dir', path: '/Pictures', modified: seedTime - 2800 })
  addNode({ kind: 'dir', path: '/Uploads', modified: seedTime - 1800 })

  addNode({
    kind: 'text',
    path: '/Documents/Welcome.md',
    modified: seedTime - 900,
    mime: 'text/markdown; charset=utf-8',
    content: `# Welcome\n\nThis is the mock Files workspace for control_panel.\n\n- Browse folders in desktop mode\n- Open files from the list\n- Create shares and test public preview\n- Upload files without a backend service\n`,
  })
  addNode({
    kind: 'text',
    path: '/Documents/Runbook.json',
    modified: seedTime - 1500,
    mime: 'application/json; charset=utf-8',
    content: JSON.stringify(
      {
        owner: 'mock.admin',
        services: ['control_panel', 'verify_hub'],
        checks: ['session bootstrap', 'monitor cards', 'files browse'],
      },
      null,
      2,
    ),
  })
  addNode({
    kind: 'text',
    path: '/Projects/ControlPanel/notes.txt',
    modified: seedTime - 700,
    mime: 'text/plain; charset=utf-8',
    content: 'Monitor and Files are the first validation slice.\nNext step: benchmark the frozen UI DataModel.\n',
  })
  addNode({
    kind: 'binary',
    path: '/Pictures/nebula.svg',
    modified: seedTime - 500,
    mime: 'image/svg+xml',
    bytes: createSvgBytes('Nebula Preview'),
  })

  const shares = new Map<string, MockShare>([
    [
      'share-welcome',
      {
        id: 'share-welcome',
        owner: CONTROL_PANEL_MOCK_USERNAME,
        path: '/Documents/Welcome.md',
        created_at: seedTime - 1200,
        expires_at: seedTime + 7 * 24 * 3600,
        password_required: false,
      },
    ],
    [
      'share-pictures',
      {
        id: 'share-pictures',
        owner: CONTROL_PANEL_MOCK_USERNAME,
        path: '/Pictures',
        created_at: seedTime - 1500,
        expires_at: null,
        password_required: true,
        password: 'bucky',
      },
    ],
  ])

  const recent = new Map<string, MockRecentEntry>([
    [
      '/Documents/Welcome.md',
      {
        path: '/Documents/Welcome.md',
        last_accessed_at: seedTime - 300,
        access_count: 4,
      },
    ],
    [
      '/Pictures/nebula.svg',
      {
        path: '/Pictures/nebula.svg',
        last_accessed_at: seedTime - 240,
        access_count: 2,
      },
    ],
  ])

  const recycleBin = new Map<string, MockTrashEntry>([
    [
      'trash-draft',
      {
        item_id: 'trash-draft',
        path: '/.trash/mock-draft.md',
        original_path: '/Documents/Draft.md',
        deleted_at: seedTime - 1800,
        node: {
          kind: 'text',
          path: '/Documents/Draft.md',
          modified: seedTime - 1900,
          mime: 'text/markdown; charset=utf-8',
          content: '# Draft\n\nThis file lives in the mock recycle bin.\n',
        },
      },
    ],
  ])

  return {
    nodes,
    favorites: new Set(['/Documents/Welcome.md', '/Pictures/nebula.svg']),
    recent,
    recycleBin,
    shares,
    uploads: new Map(),
  }
}

const getState = () => {
  if (!state) {
    state = createInitialState()
  }
  return state
}

const cloneNode = (node: MockNode, nextPath = node.path): MockNode => {
  if (node.kind === 'dir') {
    return { ...node, path: normalizePath(nextPath) }
  }
  if (node.kind === 'text') {
    return { ...node, path: normalizePath(nextPath) }
  }
  return { ...node, path: normalizePath(nextPath), bytes: node.bytes.slice() }
}

const setNode = (nextNode: MockNode) => {
  const current = getState()
  current.nodes.set(normalizePath(nextNode.path), cloneNode(nextNode))
}

const getNode = (path: string) => getState().nodes.get(normalizePath(path)) ?? null

const listChildren = (path: string) =>
  Array.from(getState().nodes.values())
    .filter((node) => isChildOf(node.path, path))
    .sort((left, right) => {
      const leftEntry = makeFileEntry(left)
      const rightEntry = makeFileEntry(right)
      if (leftEntry.is_dir !== rightEntry.is_dir) {
        return leftEntry.is_dir ? -1 : 1
      }
      return leftEntry.name.localeCompare(rightEntry.name)
    })

const updateRecent = (path: string) => {
  const normalized = normalizePath(path)
  const current = getState()
  const existing = current.recent.get(normalized)
  current.recent.set(normalized, {
    path: normalized,
    last_accessed_at: nowSeconds(),
    access_count: (existing?.access_count ?? 0) + 1,
  })
}

const replacePathReferences = (sourcePath: string, targetPath: string) => {
  const current = getState()
  const normalizedSource = normalizePath(sourcePath)
  const normalizedTarget = normalizePath(targetPath)

  const nextFavorites = new Set<string>()
  for (const path of current.favorites) {
    if (isInSubtree(path, normalizedSource)) {
      nextFavorites.add(path.replace(normalizedSource, normalizedTarget))
    } else {
      nextFavorites.add(path)
    }
  }
  current.favorites = nextFavorites

  const nextRecent = new Map<string, MockRecentEntry>()
  for (const item of current.recent.values()) {
    if (isInSubtree(item.path, normalizedSource)) {
      const nextPath = item.path.replace(normalizedSource, normalizedTarget)
      nextRecent.set(nextPath, { ...item, path: nextPath })
    } else {
      nextRecent.set(item.path, item)
    }
  }
  current.recent = nextRecent

  for (const share of current.shares.values()) {
    if (isInSubtree(share.path, normalizedSource)) {
      share.path = share.path.replace(normalizedSource, normalizedTarget)
    }
  }
}

const removeSubtree = (path: string) => {
  const current = getState()
  for (const key of Array.from(current.nodes.keys())) {
    if (isInSubtree(key, path)) {
      current.nodes.delete(key)
    }
  }
  current.favorites.delete(normalizePath(path))
  current.recent.delete(normalizePath(path))
}

const moveSubtree = (sourcePath: string, targetPath: string, copyOnly: boolean) => {
  const current = getState()
  const normalizedSource = normalizePath(sourcePath)
  const normalizedTarget = normalizePath(targetPath)
  const items = Array.from(current.nodes.values())
    .filter((node) => isInSubtree(node.path, normalizedSource))
    .sort((left, right) => left.path.length - right.path.length)

  for (const item of items) {
    const nextPath = item.path === normalizedSource
      ? normalizedTarget
      : item.path.replace(normalizedSource, normalizedTarget)
    setNode(cloneNode(item, nextPath))
  }

  if (!copyOnly) {
    removeSubtree(normalizedSource)
    replacePathReferences(normalizedSource, normalizedTarget)
  }
}

const contentForInlinePreview = (node: MockNode) => {
  if (node.kind !== 'text') {
    return null
  }
  if (node.mime.startsWith('text/') || node.mime.includes('json')) {
    return node.content
  }
  return null
}

const toBlob = (node: MockNode) => {
  if (node.kind === 'dir') {
    return null
  }
  if (node.kind === 'text') {
    return new Blob([node.content], { type: node.mime })
  }
  return new Blob([new Uint8Array(node.bytes)], { type: node.mime })
}

const buildDirectoryPayload = (path: string) => ({
  path: normalizePath(path),
  is_dir: true,
  items: listChildren(path).map(makeFileEntry),
})

const buildFilePayload = (node: MockNode, includeContent = false) => ({
  path: normalizePath(node.path),
  is_dir: false,
  size: getNodeSize(node),
  modified: node.modified,
  content: includeContent ? contentForInlinePreview(node) : undefined,
})

const resolveAuthFailure = () => {
  const cookie = document.cookie || ''
  const header = CONTROL_PANEL_MOCK_SESSION_TOKEN
  return cookie.includes(encodeURIComponent(header)) || cookie.includes(header)
}

const checkMockAuth = () => resolveAuthFailure()

const handleResourcesGet = async (_request: Request, url: URL, rawResourcePath: string) => {
  const targetPath = normalizePath(decodeURIComponent(rawResourcePath || '/'))
  const node = getNode(targetPath)
  if (!node) {
    return errorResponse(404, `Path not found: ${targetPath}`)
  }

  if (node.kind === 'dir') {
    return jsonResponse(buildDirectoryPayload(targetPath))
  }

  updateRecent(targetPath)
  const includeContent = url.searchParams.get('content') === '1'
  return jsonResponse(buildFilePayload(node, includeContent))
}

const handleResourcesPost = async (rawResourcePath: string) => {
  const targetPath = normalizePath(decodeURIComponent(rawResourcePath || '/'))
  if (getNode(targetPath)) {
    return errorResponse(409, `Path already exists: ${targetPath}`)
  }

  const parent = parentPath(targetPath)
  const parentNode = getNode(parent)
  if (!parentNode || parentNode.kind !== 'dir') {
    return errorResponse(404, `Parent directory not found: ${parent}`)
  }

  setNode({
    kind: 'dir',
    path: targetPath,
    modified: nowSeconds(),
  })
  return jsonResponse({ ok: true, path: targetPath }, 201)
}

const handleResourcesPut = async (request: Request, rawResourcePath: string) => {
  const targetPath = normalizePath(decodeURIComponent(rawResourcePath || '/'))
  const payload = (await request.json().catch(() => null)) as { content?: string } | null
  if (!payload || typeof payload.content !== 'string') {
    return errorResponse(400, 'Missing file content')
  }

  const parent = parentPath(targetPath)
  const parentNode = getNode(parent)
  if (!parentNode || parentNode.kind !== 'dir') {
    return errorResponse(404, `Parent directory not found: ${parent}`)
  }

  setNode({
    kind: 'text',
    path: targetPath,
    modified: nowSeconds(),
    mime: 'text/plain; charset=utf-8',
    content: payload.content,
  })
  updateRecent(targetPath)
  return jsonResponse({ ok: true, path: targetPath })
}

const handleResourcesPatch = async (request: Request, rawResourcePath: string) => {
  const sourcePath = normalizePath(decodeURIComponent(rawResourcePath || '/'))
  const sourceNode = getNode(sourcePath)
  if (!sourceNode) {
    return errorResponse(404, `Path not found: ${sourcePath}`)
  }

  const payload = (await request.json().catch(() => null)) as {
    action?: string
    destination?: string
    new_name?: string
    override_existing?: boolean
  } | null

  if (!payload?.action) {
    return errorResponse(400, 'Missing patch action')
  }

  if (payload.action === 'rename') {
    const nextName = String(payload.new_name ?? '').trim()
    if (!nextName || nextName.includes('/')) {
      return errorResponse(400, 'Invalid new_name')
    }
    const targetPath = joinPath(parentPath(sourcePath), nextName)
    if (getNode(targetPath)) {
      return errorResponse(409, `Target already exists: ${targetPath}`)
    }
    moveSubtree(sourcePath, targetPath, false)
    return jsonResponse({ ok: true, path: targetPath })
  }

  if (payload.action === 'move' || payload.action === 'copy') {
    const targetPath = normalizePath(String(payload.destination ?? ''))
    if (!targetPath || targetPath === '/') {
      return errorResponse(400, 'Invalid destination')
    }
    if (getNode(targetPath) && !payload.override_existing) {
      return errorResponse(409, `Target already exists: ${targetPath}`)
    }
    if (payload.override_existing) {
      removeSubtree(targetPath)
    }
    moveSubtree(sourcePath, targetPath, payload.action === 'copy')
    return jsonResponse({ ok: true, path: targetPath })
  }

  return errorResponse(400, `Unsupported patch action: ${payload.action}`)
}

const handleResourcesDelete = async (rawResourcePath: string) => {
  const targetPath = normalizePath(decodeURIComponent(rawResourcePath || '/'))
  const node = getNode(targetPath)
  if (!node) {
    return errorResponse(404, `Path not found: ${targetPath}`)
  }

  const itemId = `trash-${Date.now()}`
  getState().recycleBin.set(itemId, {
    item_id: itemId,
    path: `/.trash/${fileNameFromPath(targetPath)}`,
    original_path: targetPath,
    deleted_at: nowSeconds(),
    node: cloneNode(node),
  })
  removeSubtree(targetPath)
  return jsonResponse({ ok: true, recycled: true })
}

const handleFavorites = async (request: Request, url: URL) => {
  const current = getState()
  if (request.method === 'GET') {
    const items = Array.from(current.favorites)
      .map((path) => getNode(path))
      .filter((node): node is MockNode => Boolean(node))
      .map(makeFileEntry)
    return jsonResponse({ items })
  }

  if (request.method === 'POST') {
    const payload = (await request.json().catch(() => null)) as { path?: string } | null
    const targetPath = normalizePath(String(payload?.path ?? ''))
    if (!getNode(targetPath)) {
      return errorResponse(404, `Path not found: ${targetPath}`)
    }
    current.favorites.add(targetPath)
    return jsonResponse({ ok: true, path: targetPath }, 201)
  }

  if (request.method === 'DELETE') {
    const targetPath = normalizePath(url.searchParams.get('path') || '')
    current.favorites.delete(targetPath)
    return jsonResponse({ ok: true, path: targetPath })
  }

  return errorResponse(405, 'Unsupported favorites method')
}

const handleRecent = async () => {
  const items = Array.from(getState().recent.values())
    .sort((left, right) => right.last_accessed_at - left.last_accessed_at)
    .map((item) => {
      const node = getNode(item.path)
      if (!node) {
        return null
      }
      return {
        ...makeFileEntry(node),
        last_accessed_at: item.last_accessed_at,
        access_count: item.access_count,
      }
    })
    .filter(Boolean)
  return jsonResponse({ items })
}

const handleRecycleBin = async (request: Request, pathname: string) => {
  const current = getState()

  if (request.method === 'GET' && pathname === '/api/recycle-bin') {
    const items = Array.from(current.recycleBin.values())
      .sort((left, right) => right.deleted_at - left.deleted_at)
      .map((item) => ({
        item_id: item.item_id,
        ...makeFileEntry(item.node),
        original_path: item.original_path,
        deleted_at: item.deleted_at,
      }))
    return jsonResponse({ items })
  }

  if (request.method === 'POST' && pathname === '/api/recycle-bin/restore') {
    const payload = (await request.json().catch(() => null)) as { item_id?: string } | null
    const itemId = String(payload?.item_id ?? '')
    const target = current.recycleBin.get(itemId)
    if (!target) {
      return errorResponse(404, `Recycle item not found: ${itemId}`)
    }
    if (getNode(target.original_path)) {
      return errorResponse(409, `Restore target already exists: ${target.original_path}`)
    }
    setNode(cloneNode(target.node, target.original_path))
    current.recycleBin.delete(itemId)
    return jsonResponse({ ok: true, path: target.original_path })
  }

  if (request.method === 'DELETE' && pathname.startsWith('/api/recycle-bin/item/')) {
    const itemId = decodeURIComponent(pathname.slice('/api/recycle-bin/item/'.length))
    current.recycleBin.delete(itemId)
    return jsonResponse({ ok: true, item_id: itemId })
  }

  return errorResponse(405, 'Unsupported recycle bin method')
}

const buildShareResponse = (share: MockShare, requestedPath = '/') => {
  const normalizedPath = normalizePath(requestedPath)
  const shareRoot = normalizePath(share.path)
  const targetPath = normalizedPath === '/' ? shareRoot : normalizePath(`${shareRoot}${normalizedPath}`)
  const node = getNode(targetPath)
  if (!node) {
    return null
  }

  if (node.kind === 'dir') {
    return {
      share,
      is_dir: true,
      path: normalizedPath,
      parent_path: normalizedPath === '/' ? null : parentPath(normalizedPath),
      items: listChildren(targetPath).map((child) => ({
        ...makeFileEntry(child),
        path: normalizePath(child.path.replace(shareRoot, '') || '/'),
      })),
    }
  }

  return {
    share,
    is_dir: false,
    path: normalizedPath,
    parent_path: parentPath(normalizedPath),
    size: getNodeSize(node),
    modified: node.modified,
    content: contentForInlinePreview(node),
    mime: getNodeMime(node),
  }
}

const handleShares = async (request: Request, pathname: string) => {
  const current = getState()

  if (request.method === 'GET' && pathname === '/api/share') {
    return jsonResponse({
      items: Array.from(current.shares.values()).sort((left, right) => right.created_at - left.created_at),
    })
  }

  if (request.method === 'POST' && pathname === '/api/share') {
    const payload = (await request.json().catch(() => null)) as {
      path?: string
      password?: string
      expires_in_seconds?: number
    } | null
    const targetPath = normalizePath(String(payload?.path ?? ''))
    if (!getNode(targetPath)) {
      return errorResponse(404, `Path not found: ${targetPath}`)
    }
    const shareId = `share-${Date.now()}`
    const expiresIn = Number(payload?.expires_in_seconds ?? 0)
    const share: MockShare = {
      id: shareId,
      owner: CONTROL_PANEL_MOCK_USERNAME,
      path: targetPath,
      created_at: nowSeconds(),
      expires_at: expiresIn > 0 ? nowSeconds() + expiresIn : null,
      password_required: Boolean(payload?.password?.trim()),
      password: payload?.password?.trim() || undefined,
    }
    current.shares.set(shareId, share)
    return jsonResponse({ share }, 201)
  }

  if (request.method === 'DELETE' && pathname.startsWith('/api/share/')) {
    const shareId = decodeURIComponent(pathname.slice('/api/share/'.length))
    current.shares.delete(shareId)
    return jsonResponse({ ok: true, id: shareId })
  }

  return errorResponse(405, 'Unsupported share method')
}

const handlePublicShare = async (url: URL, pathname: string) => {
  const shareId = decodeURIComponent(pathname.slice('/api/public/share/'.length))
  const share = getState().shares.get(shareId)
  if (!share) {
    return errorResponse(404, `Share not found: ${shareId}`)
  }
  const password = url.searchParams.get('password')?.trim() || ''
  if (share.password_required && password !== (share.password ?? '')) {
    return errorResponse(403, 'Password required or incorrect')
  }
  const payload = buildShareResponse(share, url.searchParams.get('path') || '/')
  if (!payload) {
    return errorResponse(404, 'Shared target not found')
  }
  return jsonResponse(payload)
}

const handlePublicDownload = async (url: URL, pathname: string) => {
  const shareId = decodeURIComponent(pathname.slice('/api/public/dl/'.length))
  const share = getState().shares.get(shareId)
  if (!share) {
    return errorResponse(404, `Share not found: ${shareId}`)
  }
  const password = url.searchParams.get('password')?.trim() || ''
  if (share.password_required && password !== (share.password ?? '')) {
    return errorResponse(403, 'Password required or incorrect')
  }

  const shareRoot = normalizePath(share.path)
  const relativePath = normalizePath(url.searchParams.get('path') || '/')
  const targetPath = relativePath === '/' ? shareRoot : normalizePath(`${shareRoot}${relativePath}`)
  const node = getNode(targetPath)
  if (!node || node.kind === 'dir') {
    return errorResponse(404, 'Shared file not found')
  }

  return new Response(toBlob(node), {
    status: 200,
    headers: {
      'content-type': getNodeMime(node),
      'content-length': String(getNodeSize(node)),
      'content-disposition': `${url.searchParams.get('download') === '1' ? 'attachment' : 'inline'}; filename="${fileNameFromPath(node.path)}"`,
    },
  })
}

const handleSearch = async (url: URL) => {
  const query = (url.searchParams.get('q') || '').trim().toLowerCase()
  const basePath = normalizePath(url.searchParams.get('path') || '/')
  const limit = Math.max(1, Number(url.searchParams.get('limit') || 200))
  const items = Array.from(getState().nodes.values())
    .filter((node) => node.path !== '/' && isInSubtree(node.path, basePath))
    .filter((node) => {
      if (!query) {
        return true
      }
      return node.path.toLowerCase().includes(query) || fileNameFromPath(node.path).toLowerCase().includes(query)
    })
    .map(makeFileEntry)
    .slice(0, limit)

  return jsonResponse({
    query,
    path: basePath,
    kind: 'all',
    limit,
    truncated: false,
    items,
  })
}

const inferMimeFromPath = (path: string) => {
  const lower = path.toLowerCase()
  if (lower.endsWith('.svg')) {
    return 'image/svg+xml'
  }
  if (lower.endsWith('.md')) {
    return 'text/markdown; charset=utf-8'
  }
  if (lower.endsWith('.json')) {
    return 'application/json; charset=utf-8'
  }
  if (lower.endsWith('.txt')) {
    return 'text/plain; charset=utf-8'
  }
  return 'application/octet-stream'
}

const handleUploadSessions = async (request: Request, url: URL, pathname: string) => {
  const current = getState()

  if (request.method === 'POST' && pathname === '/api/upload/session') {
    const payload = (await request.json().catch(() => null)) as {
      path?: string
      size?: number
      chunk_size?: number
      override_existing?: boolean
    } | null
    const targetPath = normalizePath(String(payload?.path ?? ''))
    const size = Math.max(0, Number(payload?.size ?? 0))
    const chunkSize = Math.max(64 * 1024, Number(payload?.chunk_size ?? 2 * 1024 * 1024))
    if (!targetPath || targetPath === '/') {
      return errorResponse(400, 'Invalid upload target path')
    }
    if (getNode(targetPath) && !payload?.override_existing) {
      return errorResponse(409, `Target already exists: ${targetPath}`)
    }
    const sessionId = `upload-${Date.now()}-${Math.random().toString(36).slice(2, 8)}`
    const session = {
      id: sessionId,
      owner: CONTROL_PANEL_MOCK_USERNAME,
      path: targetPath,
      size,
      chunk_size: chunkSize,
      uploaded_size: 0,
      override_existing: payload?.override_existing !== false,
      created_at: nowSeconds(),
      updated_at: nowSeconds(),
    }
    current.uploads.set(sessionId, { session, chunks: [] })
    return jsonResponse({ session }, 201)
  }

  if (!pathname.startsWith('/api/upload/session/')) {
    return errorResponse(404, 'Upload session not found')
  }

  if (pathname.endsWith('/complete') && request.method === 'POST') {
    const sessionId = decodeURIComponent(pathname.slice('/api/upload/session/'.length, -'/complete'.length))
    const upload = current.uploads.get(sessionId)
    if (!upload) {
      return errorResponse(404, `Upload session not found: ${sessionId}`)
    }
    const merged = new Uint8Array(upload.chunks.reduce((sum, chunk) => sum + chunk.byteLength, 0))
    let offset = 0
    for (const chunk of upload.chunks) {
      merged.set(chunk, offset)
      offset += chunk.byteLength
    }
    const mime = inferMimeFromPath(upload.session.path)
    if (mime.startsWith('text/') || mime.includes('json')) {
      setNode({
        kind: 'text',
        path: upload.session.path,
        modified: nowSeconds(),
        mime,
        content: new TextDecoder().decode(merged),
      })
    } else {
      setNode({
        kind: 'binary',
        path: upload.session.path,
        modified: nowSeconds(),
        mime,
        bytes: merged,
      })
    }
    current.uploads.delete(sessionId)
    updateRecent(upload.session.path)
    return jsonResponse({ ok: true, path: upload.session.path })
  }

  const sessionId = decodeURIComponent(pathname.slice('/api/upload/session/'.length))
  const upload = current.uploads.get(sessionId)
  if (!upload) {
    return errorResponse(404, `Upload session not found: ${sessionId}`)
  }

  if (request.method === 'GET') {
    return jsonResponse({ session: upload.session })
  }

  if (request.method === 'PUT') {
    const offset = Math.max(0, Number(url.searchParams.get('offset') || 0))
    if (offset !== upload.session.uploaded_size) {
      return jsonResponse(
        {
          error: 'Chunk offset mismatch',
          expected_offset: upload.session.uploaded_size,
        },
        409,
      )
    }
    const chunk = new Uint8Array(await request.arrayBuffer())
    upload.chunks.push(chunk)
    upload.session.uploaded_size += chunk.byteLength
    upload.session.updated_at = nowSeconds()
    return jsonResponse({ uploaded_size: upload.session.uploaded_size })
  }

  if (request.method === 'DELETE') {
    current.uploads.delete(sessionId)
    return jsonResponse({ ok: true, id: sessionId })
  }

  return errorResponse(405, 'Unsupported upload session method')
}

const handleRawFile = async (pathname: string) => {
  const filePath = normalizePath(decodeURIComponent(pathname.slice('/api/raw'.length) || '/'))
  const node = getNode(filePath)
  if (!node || node.kind === 'dir') {
    return errorResponse(404, `File not found: ${filePath}`)
  }
  updateRecent(filePath)
  return new Response(toBlob(node), {
    status: 200,
    headers: {
      'content-type': getNodeMime(node),
      'content-length': String(getNodeSize(node)),
      'cache-control': 'no-store',
    },
  })
}

const handleThumbnail = async (pathname: string) => {
  const filePath = normalizePath(decodeURIComponent(pathname.slice('/api/thumb'.length) || '/'))
  const node = getNode(filePath)
  if (!node || node.kind === 'dir' || !getNodeMime(node).startsWith('image/')) {
    return errorResponse(404, `Thumbnail not found: ${filePath}`)
  }
  return new Response(toBlob(node), {
    status: 200,
    headers: {
      'content-type': getNodeMime(node),
      'content-length': String(getNodeSize(node)),
      'cache-control': 'no-store',
    },
  })
}

const handleMockRequest = async (request: Request, url: URL) => {
  const { pathname } = url

  if (!pathname.startsWith('/api/')) {
    return null
  }

  await waitForMockLatency()

  if (!pathname.startsWith('/api/public/') && !checkMockAuth()) {
    return errorResponse(401, 'Mock session missing')
  }

  if (pathname.startsWith('/api/resources')) {
    const rawResourcePath = pathname.slice('/api/resources'.length)
    if (request.method === 'GET') {
      return handleResourcesGet(request, url, rawResourcePath)
    }
    if (request.method === 'POST') {
      return handleResourcesPost(rawResourcePath)
    }
    if (request.method === 'PUT') {
      return handleResourcesPut(request, rawResourcePath)
    }
    if (request.method === 'PATCH') {
      return handleResourcesPatch(request, rawResourcePath)
    }
    if (request.method === 'DELETE') {
      return handleResourcesDelete(rawResourcePath)
    }
  }

  if (pathname === '/api/favorites') {
    return handleFavorites(request, url)
  }
  if (pathname === '/api/recent') {
    return handleRecent()
  }
  if (pathname === '/api/recycle-bin' || pathname === '/api/recycle-bin/restore' || pathname.startsWith('/api/recycle-bin/item/')) {
    return handleRecycleBin(request, pathname)
  }
  if (pathname === '/api/share' || pathname.startsWith('/api/share/')) {
    return handleShares(request, pathname)
  }
  if (pathname.startsWith('/api/public/share/')) {
    return handlePublicShare(url, pathname)
  }
  if (pathname.startsWith('/api/public/dl/')) {
    return handlePublicDownload(url, pathname)
  }
  if (pathname === '/api/search') {
    return handleSearch(url)
  }
  if (pathname === '/api/upload/session' || pathname.startsWith('/api/upload/session/')) {
    return handleUploadSessions(request, url, pathname)
  }
  if (pathname.startsWith('/api/raw')) {
    return handleRawFile(pathname)
  }
  if (pathname.startsWith('/api/thumb')) {
    return handleThumbnail(pathname)
  }

  return errorResponse(404, `Mock API route not implemented: ${pathname}`)
}

export const installFilesMockServer = () => {
  if (!isMockRuntime() || installed || typeof window === 'undefined') {
    return
  }

  const originalFetch = window.fetch.bind(window)
  window.fetch = async (input: RequestInfo | URL, init?: RequestInit) => {
    const request = input instanceof Request ? input : new Request(input, init)
    const url = new URL(request.url, window.location.origin)
    const response = await handleMockRequest(request, url)
    if (response) {
      return response
    }
    return originalFetch(input, init)
  }
  installed = true
}
