export interface SoftwarePackage {
  id: string
  name: string
  version: string
  revision?: string
  ecosystem: string
  license: string | null
  description: string
  homepage: string
  repository: string
  scopes: string[]
  licenseFileCount: number
}

export interface SoftwareProject {
  id: string
  name: string
  packages: SoftwarePackage[]
  searchText: string
  isFamily: boolean
  projectCount: number
}

interface Repository {
  id: string
  name: string
}

function compareText(left: string, right: string): number {
  return left < right ? -1 : left > right ? 1 : 0
}

function repositoryOf(value: string, homepage = false): Repository | null {
  try {
    const url = new URL(value.replace(/^git\+/, '').replace(/^git:\/\//, 'https://'))
    if (!['http:', 'https:'].includes(url.protocol) || url.username || url.password) return null
    const host = url.hostname.toLowerCase().replace(/^www\./, '')
    const isGitLab = host === 'gitlab.com' || host.startsWith('gitlab.')
    const isSimpleForge = ['github.com', 'bitbucket.org', 'codeberg.org'].includes(host)
    if (homepage && !isGitLab && !isSimpleForge) return null
    let parts = url.pathname.split('/').filter(Boolean)
    if (isSimpleForge) parts = parts.slice(0, 2)
    if (isGitLab && parts.includes('-')) parts = parts.slice(0, parts.indexOf('-'))
    if ((isSimpleForge || isGitLab) && parts.length < 2) return null
    if (parts.length === 0) return null
    parts[parts.length - 1] = parts[parts.length - 1].replace(/\.git$/i, '')
    if (!parts[parts.length - 1]) return null
    const path = parts.join('/')
    const name = parts[parts.length - 1]
    return { id: `repository:${host}${url.port ? `:${url.port}` : ''}/${isSimpleForge ? path.toLowerCase() : path}`, name }
  } catch {
    return null
  }
}

function packageKey(software: SoftwarePackage): string {
  return `package:${JSON.stringify([software.ecosystem, software.name])}`
}

function isProcMacro(software: SoftwarePackage): boolean {
  return software.ecosystem === 'cargo' && /^proc-macro(?:-|\d)/.test(software.name)
}

function groupRepositories(packages: SoftwarePackage[]): SoftwareProject[] {
  const ordered = [...packages].sort((left, right) => compareText(left.name.toLowerCase(), right.name.toLowerCase())
    || compareText(left.name, right.name) || compareText(left.version, right.version) || compareText(left.id, right.id))
  const roots = ordered.map((_, index) => index)
  const identities = new Map<string, number>()
  const repositories = ordered.map((software) => repositoryOf(software.repository) || (!software.repository ? repositoryOf(software.homepage, true) : null))
  const rootOf = (index: number): number => {
    while (roots[index] !== index) {
      roots[index] = roots[roots[index]]
      index = roots[index]
    }
    return index
  }
  ordered.forEach((software, index) => {
    const keys = [packageKey(software)]
    if (repositories[index]) keys.push(repositories[index].id)
    if (isProcMacro(software)) keys.push('family:cargo:proc-macro')
    for (const key of keys) {
      const previous = identities.get(key)
      if (previous !== undefined) roots[rootOf(index)] = rootOf(previous)
      else identities.set(key, index)
    }
  })
  const groups = new Map<number, number[]>()
  ordered.forEach((_, index) => {
    const root = rootOf(index)
    const members = groups.get(root) ?? []
    members.push(index)
    groups.set(root, members)
  })
  return [...groups.values()].map((members) => {
    const software = members.map((index) => ordered[index])
    const sources = members.map((index) => repositories[index]).filter((repository): repository is Repository => repository !== null)
      .sort((left, right) => compareText(left.id, right.id) || compareText(left.name, right.name))
    const family = software.some(isProcMacro)
    const matchingName = software.find((item) => sources.some((source) => item.name.toLowerCase() === source.name.toLowerCase()))?.name
    const name = family ? 'proc-macro' : matchingName ?? sources[0]?.name ?? software[0].name
    const id = family ? 'family:cargo:proc-macro' : sources[0]?.id ?? packageKey(software[0])
    return {
      id,
      name,
      packages: software,
      isFamily: family,
      projectCount: family ? Math.max(1, new Set(sources.map((source) => source.id)).size) : 1,
      searchText: [name, ...software.flatMap((item) => [item.name, item.version, item.license ?? '', item.ecosystem])].join(' ').toLowerCase(),
    }
  }).sort((left, right) => compareText(left.name.toLowerCase(), right.name.toLowerCase()) || compareText(left.name, right.name) || compareText(left.id, right.id))
}


const genericPrefixes = new Set(['lib', 'node', 'rust', 'is', 'has', 'get', 'set', 'to', 'no', 'core', 'std', 'sys', 'type', 'types', 'object', 'state', 'common', 'base', 'fast'])
const ownerFamilies: Record<string, string> = {
  rustcrypto: 'RustCrypto',
  'serde-rs': 'serde',
  'tokio-rs': 'tokio',
  codemirror: 'codemirror',
  'lezer-parser': 'lezer',
  'svar-widgets': 'svar-ui',
  prosemirror: 'prosemirror',
  d3: 'd3',
}
const nativeNames = new Set(['python', 'deno', 'node', 'uv', 'jq', 'pandoc', 'openssl', 'sqlite', 'zlib', 'ffmpeg', 'libreoffice'])

function normalizedName(name: string): string {
  return name.toLowerCase().replace(/[_.]+/g, '-').replace(/^-+|-+$/g, '')
}

function familyOf(project: SoftwareProject, availableNames: Map<string, Set<string>>): { key: string; name: string } | null {
  if (project.id === 'family:cargo:proc-macro') return { key: project.id, name: project.name }
  const ecosystems = [...new Set(project.packages.map((software) => software.ecosystem))]
  const ecosystem = ecosystems.length === 1 ? ecosystems[0] : ''
  const packageNames = [...new Set(project.packages.map((software) => software.name))]
  const scopes = packageNames.map((name) => name.startsWith('@') ? name.split('/')[0].slice(1) : null)
  if (ecosystem === 'npm' && scopes[0] && scopes[0] !== 'types' && scopes.every((scope) => scope === scopes[0])) {
    return { key: `series:npm:${scopes[0]}`, name: scopes[0] }
  }
  const owners = project.packages.map((software) => {
    const source = repositoryOf(software.repository) || repositoryOf(software.homepage, true)
    return source?.id.startsWith('repository:github.com/') ? source.id.split('/')[1] : null
  }).filter((owner): owner is string => owner !== null)
  const owner = owners[0]
  if (ecosystem && owner && ownerFamilies[owner] && owners.every((item) => item === owner)) {
    const name = ownerFamilies[owner]
    return { key: `series:${ecosystem}:${name.toLowerCase()}`, name }
  }
  if (!ecosystem) return null
  const names = availableNames.get(ecosystem) ?? new Set<string>()
  const tokenOf = (name: string): string | null => {
    if (name.startsWith('@')) return null
    let token = normalizedName(name).split('-')[0]
    if (token.startsWith('lib') && token.length > 5 && names.has(token.slice(3))) token = token.slice(3)
    const unnumbered = token.replace(/\d+$/, '')
    if (unnumbered.length >= 3 && unnumbered !== token
      && (names.has(unnumbered) || [...names].some((candidate) => candidate !== token && /\d$/.test(candidate) && candidate.replace(/\d+$/, '') === unnumbered))) {
      token = unnumbered
    }
    if (token.startsWith('lib') && token.length > 5 && names.has(token.slice(3))) token = token.slice(3)
    return genericPrefixes.has(token) || (token.length < 3 && !/^[a-z]\d$/.test(token)) ? null : token
  }
  const displayName = normalizedName(project.name).replace(/^rust-/, '').replace(/-rs$/, '')
  const anchor = packageNames.find((name) => normalizedName(name) === displayName)
  let name = anchor ? tokenOf(anchor) : null
  if (!anchor) {
    const counts = new Map<string, number>()
    for (const packageName of packageNames) {
      const token = tokenOf(packageName)
      if (token) counts.set(token, (counts.get(token) ?? 0) + 1)
    }
    const ranked = [...counts].sort((left, right) => right[1] - left[1] || compareText(left[0], right[0]))
    if (ranked[0] && ranked[0][1] > packageNames.length / 2) name = ranked[0][0]
  }
  if (!name) return null
  const domain = nativeNames.has(name) && ecosystems.every((item) => ['runtime', 'conda'].includes(item)) ? 'native' : ecosystem
  return { key: `series:${domain}:${name}`, name }
}

export function groupSoftwareProjects(packages: SoftwarePackage[]): SoftwareProject[] {
  const projects = groupRepositories(packages)
  const availableNames = new Map<string, Set<string>>()
  for (const software of packages) {
    const names = availableNames.get(software.ecosystem) ?? new Set<string>()
    names.add(normalizedName(software.name))
    availableNames.set(software.ecosystem, names)
  }
  const families = new Map<string, { name: string; projects: SoftwareProject[] }>()
  for (const project of projects) {
    const identity = familyOf(project, availableNames)
    const key = identity?.key ?? project.id
    const family = families.get(key) ?? { name: identity?.name ?? project.name, projects: [] }
    family.projects.push(project)
    families.set(key, family)
  }
  return [...families].map(([id, family]) => {
    if (family.projects.length === 1) return family.projects[0]
    const software = family.projects.flatMap((project) => project.packages)
      .sort((left, right) => compareText(left.name.toLowerCase(), right.name.toLowerCase()) || compareText(left.name, right.name)
        || compareText(left.version, right.version) || compareText(left.id, right.id))
    return {
      id,
      name: family.name,
      packages: software,
      isFamily: true,
      projectCount: family.projects.reduce((sum, project) => sum + project.projectCount, 0),
      searchText: [family.name, ...family.projects.map((project) => project.searchText)].join(' '),
    }
  }).sort((left, right) => compareText(left.name.toLowerCase(), right.name.toLowerCase()) || compareText(left.name, right.name) || compareText(left.id, right.id))
}
