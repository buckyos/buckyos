import { ndn } from 'buckyos'
import { basename, join, resolve } from 'node:path'
import { ToolError, UsageError } from '../core/errors.ts'
import {
  digestFile,
  type FileDigest,
  openZip,
  readZipEntry,
  sha256Bytes,
  verifyZipEntry,
  writeStoredZip,
} from './pikg_archive.ts'

export const PACKAGE_META_SCHEMA = 'buckyos.pikg.package-meta.v1'
export const DIST_MANIFEST_NAME = '.buckyos-pikg-dist.json'
export const APPDOC_ENTRY = 'APPDOC.json'
export const APPDOC_JWT_ENTRY = 'APPDOC.jwt'
export const PACKAGE_META_ENTRY = 'PACKAGE_META.json'
export const MAX_APPDOC_BYTES = 1024 * 1024
export const MAX_METADATA_ENTRY_BYTES = 8 * 1024 * 1024
export const MAX_METADATA_TOTAL_BYTES = 64 * 1024 * 1024

const SAFE_SUBPACKAGE = /^[A-Za-z0-9._-]+$/
const SAFE_BNS_LABEL = /^[a-z0-9](?:[a-z0-9-]*[a-z0-9])?$/
const SHA256_DIGEST = /^sha256:([0-9a-fA-F]{64})$/
const OBJECT_ID = /^(appdoc|pkg):[0-9a-f]{64}$/

export interface ContentIndexEntry {
  sub_pkg_name: string
  path: string
  format: 'tar.gz'
  size: number
  digest: string
}

export interface PackageMetaFile {
  '@schema': typeof PACKAGE_META_SCHEMA
  app_doc_id: string
  package_objects: Record<string, Record<string, unknown>>
  content_index: Record<string, ContentIndexEntry>
}

export interface ValidatedSubpackage {
  key: string
  selector: Record<string, string> | null
  required: boolean
  pkg_id: string
  pkg_objid: string
  payload_path: string
  payload_size: number
  payload_digest: string
  docker_image_name?: string
  docker_image_digest?: string
}

export interface ValidatedPikg {
  appDoc: Record<string, unknown>
  appDocObjectId: string
  packageMeta: PackageMetaFile
  subpackages: ValidatedSubpackage[]
  hasJsonAppDoc: boolean
  hasSignedAppDoc: boolean
  canonicalMatch: boolean
}

export interface PikgInspectionResult {
  schema_version: 1
  protocol: string
  pikg_path: string
  size: number
  pikg_digest: string
  valid: true
  app: {
    did: string
    app_id: string
    version: string
    owner: string
    app_doc_object_id: string
  }
  app_doc_form: 'unsigned' | 'signed' | 'both'
  canonical_match: boolean
  subpackages: Array<Record<string, unknown>>
  offline_content_validation: 'passed'
  signature_validation: 'not-present' | 'not-resolvable-offline'
  publication_validation: 'not-checked'
}

export function validateSubpackageName(name: string): void {
  if (!SAFE_SUBPACKAGE.test(name) || name === '.' || name === '..') {
    throw new UsageError('INVALID_SUBPACKAGE_NAME', `invalid subpackage name: ${name}`)
  }
}

export function normalizeArch(value: string): string {
  switch (value.trim().toLowerCase()) {
    case 'amd64':
    case 'x86_64':
    case 'x64':
      return 'x86_64'
    case 'arm64':
    case 'aarch64':
      return 'aarch64'
    default:
      return value.trim().toLowerCase()
  }
}

export function normalizeOs(value: string): string {
  switch (value.trim().toLowerCase()) {
    case 'darwin':
    case 'apple':
    case 'macos':
    case 'osx':
      return 'macos'
    case 'win':
    case 'win32':
    case 'windows':
      return 'windows'
    default:
      return value.trim().toLowerCase()
  }
}

export function derivedSelector(key: string): Record<string, string> | undefined {
  const selectors: Record<string, Record<string, string>> = {
    amd64_docker_image: { os: 'linux', arch: 'x86_64' },
    aarch64_docker_image: { os: 'linux', arch: 'aarch64' },
    amd64_linux_app: { os: 'linux', arch: 'x86_64' },
    aarch64_linux_app: { os: 'linux', arch: 'aarch64' },
    amd64_win_app: { os: 'windows', arch: 'x86_64' },
    aarch64_win_app: { os: 'windows', arch: 'aarch64' },
    amd64_apple_app: { os: 'macos', arch: 'x86_64' },
    aarch64_apple_app: { os: 'macos', arch: 'aarch64' },
    web: {},
    agent: {},
    agent_skills: {},
    agent_tools: {},
    script: {},
  }
  return selectors[key]
}

export function canonicalSelector(
  value: unknown,
  label: string,
): Record<string, string> | undefined {
  if (value === undefined) return undefined
  const selector = expectObject(value, label)
  rejectUnknown(selector, ['os', 'arch', 'min_kernel_version'], label)
  const output: Record<string, string> = {}
  for (const key of ['os', 'arch', 'min_kernel_version']) {
    if (selector[key] !== undefined) {
      output[key] = expectNonEmptyString(selector[key], `${label}.${key}`)
    }
  }
  if (output.os) output.os = normalizeOs(output.os)
  if (output.arch) output.arch = normalizeArch(output.arch)
  return output
}

export function assertSelectorCompatible(
  key: string,
  selector: Record<string, string> | undefined,
  label: string,
): void {
  const derived = derivedSelector(key)
  if (!derived || !selector) return
  for (const field of ['os', 'arch']) {
    if (derived[field] !== undefined && selector[field] !== derived[field]) {
      throw new UsageError(
        'SELECTOR_CONFLICT',
        `${label}.${field} conflicts with the selector derived from ${key}`,
      )
    }
  }
}

export function deriveAppNamespace(
  appDid: string,
): string {
  return appIdFromDid(appDid)
}

export function appIdFromDid(appDid: string): string {
  const app = parseDid(appDid, 'did')
  if (
    app.id.includes('#') || app.id.includes('/') || app.id.includes('%') || app.id.includes(':')
  ) {
    throw new UsageError(
      'INVALID_APP_ID',
      'App DID must use hostname form without path, fragment, port, or encoding',
    )
  }
  const labels = app.id.split('.')
  if (labels.some((label) => !SAFE_BNS_LABEL.test(label))) {
    throw new UsageError('INVALID_APP_ID', 'App DID must contain canonical lowercase DNS labels')
  }
  if (app.method === 'web') {
    if (labels.length >= 3 && labels.at(-1) === 'did') {
      throw new UsageError(
        'INVALID_APP_ID',
        'did:web hostname conflicts with the reserved non-Web .did form',
      )
    }
    return app.id
  }
  return `${app.id}.${app.method}.did`
}

export function createPackageMeta(
  packageName: string,
  version: string,
  author: string,
  owner: string,
  payload: FileDigest,
  timestamp: number,
): { value: Record<string, unknown>; objectId: string } {
  const content = ndn.ChunkId.fromMix256Result(
    payload.size,
    ndn.hexToBytes(payload.sha256),
  ).toString()
  const value: Record<string, unknown> = {
    name: packageName,
    author,
    owner,
    create_time: timestamp,
    last_update_time: timestamp,
    size: payload.size,
    content,
    version,
  }
  const objectId = ndn.buildNamedObjectByJson(ndn.OBJ_TYPE_PKG, value)[0].toString()
  return { value, objectId }
}

export function validatePermissions(value: unknown, label: string): unknown[] {
  if (!Array.isArray(value)) throw invalid('schema', `${label} must be an array`)
  return value.map((raw, index) => {
    const item = expectObject(raw, `${label}[${index}]`)
    assertKnownFields(item, ['scope_path', 'required', 'actions', 'exp'], `${label}[${index}]`)
    expectNonEmptyString(item.scope_path, `${label}[${index}].scope_path`)
    if (typeof item.required !== 'boolean') {
      throw invalid('schema', `${label}[${index}].required must be boolean`)
    }
    if (
      item.actions !== undefined &&
      (!Array.isArray(item.actions) || item.actions.some((action) => typeof action !== 'string'))
    ) {
      throw invalid('schema', `${label}[${index}].actions must be a string array`)
    }
    if (
      item.exp !== null && item.exp !== undefined &&
      (typeof item.exp !== 'number' || !Number.isInteger(item.exp) || item.exp < 0 ||
        item.exp > 0xffffffff)
    ) {
      throw invalid('schema', `${label}[${index}].exp must be null or a uint32`)
    }
    return raw
  })
}

export function validateServiceConfigTips(
  value: unknown,
  label: string,
): Record<string, unknown> {
  const config = expectObject(value, label)
  for (
    const field of [
      'service_endpoints',
      'data_mount_points',
      'local_cache_mount_points',
      'external_mount_points',
      'rdb_instances',
      'bash_envs',
      'runtime_caps',
    ]
  ) {
    if (config[field] !== undefined) expectObject(config[field], `${label}.${field}`)
  }
  for (
    const [name, raw] of Object.entries(
      expectObject(config.service_endpoints ?? {}, `${label}.service_endpoints`),
    )
  ) {
    const endpoint = expectObject(raw, `${label}.service_endpoints.${name}`)
    assertKnownFields(
      endpoint,
      ['protocol', 'inner_port', 'required', 'description', 'expose'],
      `${label}.service_endpoints.${name}`,
    )
    if (!['http', 'https', 'tcp', 'udp'].includes(String(endpoint.protocol))) {
      throw invalid('schema', `${label}.service_endpoints.${name}.protocol is invalid`)
    }
    expectUint(endpoint.inner_port, `${label}.service_endpoints.${name}.inner_port`, 0xffff)
    optionalBoolean(endpoint.required, `${label}.service_endpoints.${name}.required`)
    validateStringMap(
      endpoint.description ?? {},
      `${label}.service_endpoints.${name}.description`,
    )
    if (endpoint.expose !== undefined && endpoint.expose !== null) {
      const expose = expectObject(endpoint.expose, `${label}.service_endpoints.${name}.expose`)
      assertKnownFields(
        expose,
        ['route', 'scope', 'allow_guest'],
        `${label}.service_endpoints.${name}.expose`,
      )
      const route = expectObject(expose.route, `${label}.service_endpoints.${name}.expose.route`)
      if (route.type === 'web') {
        assertKnownFields(route, ['type'], `${label}.service_endpoints.${name}.expose.route`)
      } else if (route.type === 'port') {
        assertKnownFields(
          route,
          ['type', 'preferred_port'],
          `${label}.service_endpoints.${name}.expose.route`,
        )
        if (route.preferred_port !== undefined) {
          expectUint(
            route.preferred_port,
            `${label}.service_endpoints.${name}.expose.route.preferred_port`,
            0xffff,
          )
        }
      } else {
        throw invalid('schema', `${label}.service_endpoints.${name}.expose.route.type is invalid`)
      }
      optionalString(expose.scope, `${label}.service_endpoints.${name}.expose.scope`)
      optionalBoolean(
        expose.allow_guest,
        `${label}.service_endpoints.${name}.expose.allow_guest`,
      )
    }
  }
  for (
    const field of [
      'data_mount_points',
      'local_cache_mount_points',
      'external_mount_points',
    ]
  ) {
    for (
      const [name, raw] of Object.entries(expectObject(config[field] ?? {}, `${label}.${field}`))
    ) {
      if (raw === null) continue
      const mount = expectObject(raw, `${label}.${field}.${name}`)
      assertKnownFields(
        mount,
        ['mount_point_name', 'access', 'reason'],
        `${label}.${field}.${name}`,
      )
      expectNonEmptyString(mount.mount_point_name, `${label}.${field}.${name}.mount_point_name`)
      expectNonEmptyString(mount.access, `${label}.${field}.${name}.access`)
      validateStringMap(mount.reason, `${label}.${field}.${name}.reason`)
    }
  }
  for (
    const [name, raw] of Object.entries(
      expectObject(config.rdb_instances ?? {}, `${label}.rdb_instances`),
    )
  ) {
    const database = expectObject(raw, `${label}.rdb_instances.${name}`)
    assertKnownFields(
      database,
      ['backend', 'version', 'schema', 'connection'],
      `${label}.rdb_instances.${name}`,
    )
    if (!['sqlite', 'postgres'].includes(String(database.backend))) {
      throw invalid('schema', `${label}.rdb_instances.${name}.backend is invalid`)
    }
    if (database.version !== undefined) {
      expectUint(
        database.version,
        `${label}.rdb_instances.${name}.version`,
        Number.MAX_SAFE_INTEGER,
      )
    }
    const schemas = validateStringMap(
      database.schema ?? {},
      `${label}.rdb_instances.${name}.schema`,
    )
    for (const backend of Object.keys(schemas)) {
      if (!['sqlite', 'postgres'].includes(backend)) {
        throw invalid('schema', `${label}.rdb_instances.${name}.schema.${backend} is invalid`)
      }
    }
    optionalString(database.connection, `${label}.rdb_instances.${name}.connection`)
  }
  if (config.instance_volume !== undefined) {
    const volume = expectObject(config.instance_volume, `${label}.instance_volume`)
    assertKnownFields(
      volume,
      ['mode', 'quota_mib', 'ephemeral_contents'],
      `${label}.instance_volume`,
    )
    if (
      volume.mode !== undefined &&
      !['required', 'optional', 'disabled'].includes(String(volume.mode))
    ) {
      throw invalid('schema', `${label}.instance_volume.mode is invalid`)
    }
    if (volume.quota_mib !== undefined) {
      expectUint(volume.quota_mib, `${label}.instance_volume.quota_mib`, Number.MAX_SAFE_INTEGER)
    }
    if (
      volume.ephemeral_contents !== undefined &&
      (!Array.isArray(volume.ephemeral_contents) ||
        volume.ephemeral_contents.some((item) => typeof item !== 'string'))
    ) {
      throw invalid('schema', `${label}.instance_volume.ephemeral_contents must be a string array`)
    }
  }
  for (
    const [name, raw] of Object.entries(expectObject(config.bash_envs ?? {}, `${label}.bash_envs`))
  ) {
    const environment = expectObject(raw, `${label}.bash_envs.${name}`)
    assertKnownFields(environment, ['required', 'description'], `${label}.bash_envs.${name}`)
    if (typeof environment.required !== 'boolean') {
      throw invalid('schema', `${label}.bash_envs.${name}.required must be boolean`)
    }
    validateStringMap(environment.description ?? {}, `${label}.bash_envs.${name}.description`)
  }
  validateStringMap(config.runtime_caps ?? {}, `${label}.runtime_caps`)
  optionalString(config.container_param, `${label}.container_param`)
  optionalString(config.start_param, `${label}.start_param`)
  return config
}

export function appDocObjectId(value: Record<string, unknown>): string {
  return ndn.buildNamedObjectByJson('appdoc', value)[0].toString()
}

export async function validateSnapshot(
  distDir: string,
  appDoc: Record<string, unknown>,
  packageMeta: PackageMetaFile,
): Promise<ValidatedPikg> {
  const payloads = new Map<string, FileDigest>()
  for (const entryValue of Object.values(packageMeta.content_index)) {
    const entry = validateContentIndexEntry(entryValue, 'content_index')
    const path = join(distDir, entry.path)
    const digest = await digestFile(path)
    payloads.set(entry.path, digest)
  }
  return validateObjectGraph(appDoc, packageMeta, payloads, true, false, true)
}

export async function packSnapshot(
  distDir: string,
  destination: string,
  appDoc: Record<string, unknown>,
  packageMeta: PackageMetaFile,
): Promise<void> {
  const validated = await validateSnapshot(distDir, appDoc, packageMeta)
  const encoder = new TextEncoder()
  await writeStoredZip(destination, [
    { name: APPDOC_ENTRY, bytes: encoder.encode(`${JSON.stringify(appDoc, null, 2)}\n`) },
    {
      name: PACKAGE_META_ENTRY,
      bytes: encoder.encode(`${JSON.stringify(packageMeta, null, 2)}\n`),
    },
    ...validated.subpackages.map((subpackage) => ({
      name: subpackage.payload_path,
      path: join(distDir, subpackage.payload_path),
    })),
  ])
}

export async function inspectPikg(path: string): Promise<PikgInspectionResult> {
  const archive = await openZip(path)
  let metadataTotal = 0
  for (const entry of archive.entries) {
    if (
      entry.name === APPDOC_ENTRY || entry.name === APPDOC_JWT_ENTRY ||
      entry.name === PACKAGE_META_ENTRY ||
      entry.name.startsWith('objects/') && entry.name.endsWith('.json')
    ) {
      const limit = entry.name === APPDOC_ENTRY || entry.name === APPDOC_JWT_ENTRY
        ? MAX_APPDOC_BYTES
        : MAX_METADATA_ENTRY_BYTES
      if (entry.size > limit) throw invalid('limits', `metadata entry exceeds limit: ${entry.name}`)
      metadataTotal += entry.size
    }
  }
  if (metadataTotal > MAX_METADATA_TOTAL_BYTES) {
    throw invalid('limits', 'PIKG metadata exceeds the total size limit')
  }
  if (archive.byName.has('APPDOC.wt')) {
    throw invalid('appdoc', 'legacy APPDOC.wt is not supported')
  }
  const jsonEntry = archive.byName.get(APPDOC_ENTRY)
  const jwtEntry = archive.byName.get(APPDOC_JWT_ENTRY)
  if (!jsonEntry && !jwtEntry) throw invalid('appdoc', 'PIKG has no App Document')

  const jsonDoc = jsonEntry
    ? parseJsonObject(
      await readZipEntry(archive, jsonEntry, MAX_APPDOC_BYTES),
      APPDOC_ENTRY,
    )
    : undefined
  const jwt = jwtEntry
    ? new TextDecoder('utf-8', { fatal: true }).decode(
      await readZipEntry(archive, jwtEntry, MAX_APPDOC_BYTES),
    ).trim()
    : undefined
  const jwtDoc = jwt ? decodeJwtClaims(jwt) : undefined
  if (jsonDoc && jwtDoc && appDocObjectId(jsonDoc) !== appDocObjectId(jwtDoc)) {
    throw invalid('appdoc', 'APPDOC.json and APPDOC.jwt have different canonical documents')
  }
  const appDoc = jwtDoc ?? jsonDoc!
  const metaEntry = archive.byName.get(PACKAGE_META_ENTRY)
  if (!metaEntry) throw invalid('package-meta', 'PACKAGE_META.json is required')
  const packageMeta = parsePackageMeta(
    parseJsonObject(
      await readZipEntry(archive, metaEntry, MAX_METADATA_ENTRY_BYTES),
      PACKAGE_META_ENTRY,
    ),
  )
  for (
    const entry of archive.entries.filter((candidate) => candidate.name.startsWith('objects/'))
  ) {
    const match = /^objects\/([^/]+)\.json$/.exec(entry.name)
    if (!match) {
      throw invalid(
        'object-graph',
        `invalid object entry path: ${entry.name}`,
        entry.name,
      )
    }
    let objectId: InstanceType<typeof ndn.ObjId>
    try {
      objectId = ndn.ObjId.fromString(match[1])
    } catch {
      throw invalid('object-graph', `invalid object entry ID: ${entry.name}`, entry.name)
    }
    const value = parseJsonValue(
      await readZipEntry(archive, entry, MAX_METADATA_ENTRY_BYTES),
      entry.name,
    )
    const computed = ndn.buildNamedObjectByJson(objectId.objType, value)[0]
    if (!computed.equals(objectId)) {
      throw invalid('object-graph', `object entry ID mismatch: ${entry.name}`, entry.name)
    }
  }
  const payloads = new Map<string, FileDigest>()
  for (const value of Object.values(packageMeta.content_index)) {
    const content = validateContentIndexEntry(value, 'content_index')
    const entry = archive.byName.get(content.path)
    if (!entry) throw invalid('content-index', `content entry is missing: ${content.path}`)
    if (entry.size !== content.size) {
      throw invalid('content-index', `content entry size mismatch: ${content.path}`, content.path)
    }
    const expected = parseSha256(content.digest, `content_index.${content.digest}`)
    payloads.set(content.path, await verifyZipEntry(archive, entry, expected))
  }
  const validated = validateObjectGraph(
    appDoc,
    packageMeta,
    payloads,
    Boolean(jsonEntry),
    Boolean(jwtEntry),
    !jsonDoc || !jwtDoc || appDocObjectId(jsonDoc) === appDocObjectId(jwtDoc),
  )
  const pikgDigest = await digestFile(path)
  return {
    schema_version: 1,
    protocol: PACKAGE_META_SCHEMA,
    pikg_path: resolve(path),
    size: pikgDigest.size,
    pikg_digest: `sha256:${pikgDigest.sha256}`,
    valid: true,
    app: {
      did: String(appDoc.did),
      app_id: appIdFromDid(String(appDoc.did)),
      version: String(appDoc.version),
      owner: String(appDoc.owner),
      app_doc_object_id: validated.appDocObjectId,
    },
    app_doc_form: jsonEntry && jwtEntry ? 'both' : jwtEntry ? 'signed' : 'unsigned',
    canonical_match: validated.canonicalMatch,
    subpackages: validated.subpackages.map((subpackage) => ({
      key: subpackage.key,
      selector: subpackage.selector,
      required: subpackage.required,
      pkg_id: subpackage.pkg_id,
      pkg_objid: subpackage.pkg_objid,
      payload: {
        path: subpackage.payload_path,
        size: subpackage.payload_size,
        digest: subpackage.payload_digest,
      },
      ...(subpackage.docker_image_name ? { docker_image_name: subpackage.docker_image_name } : {}),
      ...(subpackage.docker_image_digest
        ? { docker_image_digest: subpackage.docker_image_digest }
        : {}),
    })),
    offline_content_validation: 'passed',
    signature_validation: jwtEntry ? 'not-resolvable-offline' : 'not-present',
    publication_validation: 'not-checked',
  }
}

export function parsePackageMeta(value: Record<string, unknown>): PackageMetaFile {
  try {
    rejectUnknown(
      value,
      ['@schema', 'app_doc_id', 'package_objects', 'content_index'],
      PACKAGE_META_ENTRY,
    )
  } catch (error) {
    if (error instanceof UsageError) throw invalid('package-meta', error.message)
    throw error
  }
  if (value['@schema'] !== PACKAGE_META_SCHEMA) {
    throw invalid('package-meta', `unsupported PACKAGE_META.json schema: ${value['@schema']}`)
  }
  const appDocId = expectNonEmptyString(value.app_doc_id, 'PACKAGE_META.json.app_doc_id')
  if (!OBJECT_ID.test(appDocId) || !appDocId.startsWith('appdoc:')) {
    throw invalid('package-meta', 'PACKAGE_META.json.app_doc_id is invalid')
  }
  const packageObjects = expectObject(value.package_objects, 'PACKAGE_META.json.package_objects')
  const contentIndex = expectObject(value.content_index, 'PACKAGE_META.json.content_index')
  return {
    '@schema': PACKAGE_META_SCHEMA,
    app_doc_id: appDocId,
    package_objects: packageObjects as Record<string, Record<string, unknown>>,
    content_index: contentIndex as Record<string, ContentIndexEntry>,
  }
}

function validateObjectGraph(
  appDoc: Record<string, unknown>,
  packageMeta: PackageMetaFile,
  payloads: Map<string, FileDigest>,
  hasJson: boolean,
  hasSigned: boolean,
  canonicalMatch: boolean,
): ValidatedPikg {
  validateAppDocShape(appDoc)
  const appId = appDocObjectId(appDoc)
  if (packageMeta.app_doc_id !== appId) {
    throw invalid('object-graph', 'PACKAGE_META.json app_doc_id does not match APPDOC')
  }
  const appVersion = String(appDoc.version)
  const namespace = deriveNamespaceForPackage(appDoc)
  const pkgList = appDoc.pkg_list as Record<string, unknown>
  const referenced = new Set<string>()
  const subpackages: ValidatedSubpackage[] = []
  const contentBySubpackage = new Map<string, ContentIndexEntry>()
  for (const [digest, raw] of Object.entries(packageMeta.content_index)) {
    const entry = validateContentIndexEntry(raw, `content_index.${digest}`)
    if (entry.digest !== digest) throw invalid('content-index', `digest key mismatch: ${digest}`)
    const expectedHex = parseSha256(digest, `content_index.${digest}`)
    const payload = payloads.get(entry.path)
    if (!payload) throw invalid('content-index', `payload is missing: ${entry.path}`, entry.path)
    if (payload.size !== entry.size || payload.sha256 !== expectedHex) {
      throw invalid('content', `payload does not match content index: ${entry.path}`, entry.path)
    }
    if (contentBySubpackage.has(entry.sub_pkg_name)) {
      throw invalid('content-index', `subpackage has more than one payload: ${entry.sub_pkg_name}`)
    }
    contentBySubpackage.set(entry.sub_pkg_name, entry)
  }

  const dependencyNames = new Set<string>()
  for (
    const [key, rawDesc] of Object.entries(pkgList).sort(([left], [right]) =>
      left.localeCompare(right)
    )
  ) {
    validateSubpackageNameForPackage(key)
    const desc = expectObject(rawDesc, `APPDOC.pkg_list.${key}`)
    rejectUnknown(
      desc,
      [
        'pkg_id',
        'pkg_objid',
        'docker_image_name',
        'docker_image_digest',
        'source_url',
        'selector',
        'required',
      ],
      `APPDOC.pkg_list.${key}`,
    )
    if (desc.source_url !== undefined) {
      throw invalid('self-contained', `source_url is not allowed for bundled App: ${key}`)
    }
    const pkgId = expectNonEmptyString(desc.pkg_id, `APPDOC.pkg_list.${key}.pkg_id`)
    const parsedId = parsePackageId(pkgId, key)
    if (parsedId.version !== appVersion) {
      throw invalid('namespace', `subpackage ${key} must use App version ${appVersion}`)
    }
    const uniqueName = packageUniqueName(parsedId.name, key)
    const namespacePrefix = uniqueName.endsWith(`.${namespace}`)
      ? uniqueName.slice(0, -namespace.length - 1)
      : undefined
    if (
      uniqueName !== namespace &&
      (!namespacePrefix || namespacePrefix.includes('.') ||
        !/^[a-z0-9][a-z0-9_-]*$/.test(namespacePrefix))
    ) {
      throw invalid('namespace', `subpackage ${key} is outside App package namespace`)
    }
    const pkgObjid = expectNonEmptyString(desc.pkg_objid, `APPDOC.pkg_list.${key}.pkg_objid`)
    if (!OBJECT_ID.test(pkgObjid) || !pkgObjid.startsWith('pkg:')) {
      throw invalid('object-graph', `subpackage ${key} has an invalid pkg_objid`)
    }
    const packageObject = packageMeta.package_objects[pkgObjid]
    if (!packageObject) throw invalid('object-graph', `PackageMeta is missing for ${key}`)
    const computed = ndn.buildNamedObjectByJson(ndn.OBJ_TYPE_PKG, packageObject)[0].toString()
    if (computed !== pkgObjid) {
      throw invalid('object-graph', `PackageMeta Object ID mismatch: ${key}`)
    }
    const metaName = expectNonEmptyString(packageObject.name, `PackageMeta(${key}).name`)
    const metaVersion = expectNonEmptyString(packageObject.version, `PackageMeta(${key}).version`)
    if (metaName !== parsedId.name || metaVersion !== parsedId.version) {
      throw invalid('namespace', `PackageMeta identity does not match pkg_id: ${key}`)
    }
    if (packageObject.owner !== appDoc.owner || packageObject.author !== appDoc.author) {
      throw invalid('namespace', `PackageMeta owner/author does not match AppDoc: ${key}`)
    }
    const metaDeps = packageObject.deps
    if (
      metaDeps !== undefined &&
      Object.keys(expectObject(metaDeps, `PackageMeta(${key}).deps`)).length
    ) {
      throw invalid(
        'self-contained',
        `third-party PackageMeta dependencies are not allowed: ${key}`,
      )
    }
    const entry = contentBySubpackage.get(key)
    if (!entry) throw invalid('content-index', `content index is missing for ${key}`)
    if (entry.path !== `${key}.tar.gz` || entry.format !== 'tar.gz') {
      throw invalid('content-index', `subpackage ${key} has a noncanonical payload path`)
    }
    const metaSize = expectSafeInteger(packageObject.size, `PackageMeta(${key}).size`)
    if (metaSize !== entry.size) throw invalid('content', `PackageMeta size mismatch: ${key}`)
    validateChunkContent(
      expectNonEmptyString(packageObject.content, `PackageMeta(${key}).content`),
      entry.size,
      parseSha256(entry.digest, `content_index.${entry.digest}`),
      key,
    )
    const selector = canonicalSelectorForPackage(desc.selector, `APPDOC.pkg_list.${key}.selector`)
    assertSelectorForPackage(key, selector)
    if (desc.required !== undefined && typeof desc.required !== 'boolean') {
      throw invalid('appdoc', `required must be boolean: ${key}`)
    }
    const dockerImageName = optionalNonEmptyString(
      desc.docker_image_name,
      `APPDOC.pkg_list.${key}.docker_image_name`,
    )
    const dockerImageDigest = optionalNonEmptyString(
      desc.docker_image_digest,
      `APPDOC.pkg_list.${key}.docker_image_digest`,
    )
    if (dockerImageDigest && !SHA256_DIGEST.test(dockerImageDigest)) {
      throw invalid('appdoc', `invalid Docker image digest: ${key}`)
    }
    referenced.add(pkgObjid)
    dependencyNames.add(metaName)
    subpackages.push({
      key,
      selector: selector ?? derivedSelector(key) ?? null,
      required: desc.required === undefined ? true : desc.required,
      pkg_id: pkgId,
      pkg_objid: pkgObjid,
      payload_path: entry.path,
      payload_size: entry.size,
      payload_digest: entry.digest,
      ...(dockerImageName ? { docker_image_name: dockerImageName } : {}),
      ...(dockerImageDigest ? { docker_image_digest: dockerImageDigest } : {}),
    })
  }
  if (subpackages.length === 0) throw invalid('appdoc', 'AppDoc.pkg_list must not be empty')
  for (const key of Object.keys(packageMeta.package_objects)) {
    if (!referenced.has(key)) {
      throw invalid('object-graph', `unreferenced PackageMeta object: ${key}`)
    }
  }
  if (contentBySubpackage.size !== subpackages.length) {
    throw invalid('content-index', 'content index and pkg_list have different subpackages')
  }
  if (dependencyNames.size !== subpackages.length) {
    throw invalid('self-contained', 'AppDoc package identities are not unique')
  }
  return {
    appDoc,
    appDocObjectId: appId,
    packageMeta,
    subpackages,
    hasJsonAppDoc: hasJson,
    hasSignedAppDoc: hasSigned,
    canonicalMatch,
  }
}

export function validateAppDocShape(appDoc: Record<string, unknown>): void {
  rejectUnknown(
    appDoc,
    [
      'schema_version',
      'doc_type',
      'did',
      'name',
      'copyright',
      'tags',
      'categories',
      'base_on',
      'directory',
      'references',
      'version',
      'version_tag',
      'app_type',
      'owner',
      'controller',
      'author',
      'create_time',
      'last_update_time',
      'exp',
      'pkg_list',
      'show_name',
      'presentation',
      'sdk_version',
      'req_capbilities',
      'permissions',
      'selector_type',
      'service_config_tips',
    ],
    'APPDOC',
  )
  if (appDoc.schema_version !== 1) {
    throw invalid('appdoc', 'APPDOC.schema_version must be 1')
  }
  for (
    const field of [
      'did',
      'version',
      'app_type',
      'owner',
      'controller',
      'author',
      'show_name',
      'selector_type',
    ]
  ) {
    expectNonEmptyString(appDoc[field], `APPDOC.${field}`)
  }
  if (appDoc.doc_type !== 'app') throw invalid('appdoc', 'APPDOC.doc_type must be app')
  if (!['service', 'dapp', 'web', 'agent'].includes(String(appDoc.app_type))) {
    throw invalid('appdoc', 'APPDOC.app_type is invalid')
  }
  if (appDoc.name !== undefined) expectNonEmptyString(appDoc.name, 'APPDOC.name')
  optionalString(appDoc.copyright, 'APPDOC.copyright')
  for (const field of ['tags', 'categories']) {
    const values = appDoc[field]
    if (
      values !== undefined &&
      (!Array.isArray(values) || values.length === 0 ||
        values.some((value) => typeof value !== 'string'))
    ) {
      throw invalid('appdoc', `APPDOC.${field} must be a non-empty string array when present`)
    }
  }
  if (appDoc.base_on !== undefined) {
    const baseOn = expectNonEmptyString(appDoc.base_on, 'APPDOC.base_on')
    try {
      ndn.ObjId.fromString(baseOn)
    } catch {
      throw invalid('appdoc', 'APPDOC.base_on must be an ObjectId')
    }
  }
  for (const field of ['directory', 'references']) {
    if (appDoc[field] === undefined) continue
    const entries = expectObject(appDoc[field], `APPDOC.${field}`)
    if (Object.keys(entries).length === 0) {
      throw invalid('appdoc', `APPDOC.${field} must not be empty when present`)
    }
    for (const [key, value] of Object.entries(entries)) {
      expectObject(value, `APPDOC.${field}.${key}`)
    }
  }
  expectSafeInteger(appDoc.create_time, 'APPDOC.create_time')
  expectSafeInteger(appDoc.last_update_time, 'APPDOC.last_update_time')
  if (expectSafeInteger(appDoc.exp, 'APPDOC.exp') === 0) {
    throw invalid('appdoc', 'APPDOC.exp must be greater than zero')
  }
  appIdFromDid(String(appDoc.did))
  parseDid(String(appDoc.owner), 'owner')
  parseDid(String(appDoc.controller), 'controller')
  parseDid(String(appDoc.author), 'author')
  if (appDoc.permissions !== undefined && !Array.isArray(appDoc.permissions)) {
    throw invalid('appdoc', 'APPDOC.permissions must be an array')
  }
  validatePermissions(appDoc.permissions ?? [], 'APPDOC.permissions')
  validateServiceConfigTips(appDoc.service_config_tips, 'APPDOC.service_config_tips')
  const pkgList = expectObject(appDoc.pkg_list, 'APPDOC.pkg_list')
  if (Object.keys(pkgList).length === 0) {
    throw invalid('appdoc', 'APPDOC.pkg_list must not be empty')
  }
  deriveNamespaceForPackage(appDoc)
}

function deriveNamespaceForPackage(appDoc: Record<string, unknown>): string {
  try {
    return deriveAppNamespace(String(appDoc.did))
  } catch (error) {
    if (error instanceof UsageError) throw invalid('namespace', error.message)
    throw error
  }
}

function validateContentIndexEntry(value: unknown, label: string): ContentIndexEntry {
  const entry = expectObject(value, label)
  rejectUnknown(entry, ['sub_pkg_name', 'path', 'format', 'size', 'digest'], label)
  const subpackage = expectNonEmptyString(entry.sub_pkg_name, `${label}.sub_pkg_name`)
  validateSubpackageNameForPackage(subpackage)
  const path = expectNonEmptyString(entry.path, `${label}.path`)
  if (path !== `${subpackage}.tar.gz`) {
    throw invalid('content-index', `invalid payload path: ${path}`)
  }
  if (entry.format !== 'tar.gz') throw invalid('content-index', `invalid payload format: ${path}`)
  const size = expectSafeInteger(entry.size, `${label}.size`)
  const digest = expectNonEmptyString(entry.digest, `${label}.digest`)
  parseSha256(digest, `${label}.digest`)
  return { sub_pkg_name: subpackage, path, format: 'tar.gz', size, digest }
}

function validateChunkContent(content: string, size: number, sha256: string, key: string): void {
  let chunk: InstanceType<typeof ndn.ChunkId>
  try {
    chunk = ndn.ChunkId.fromString(content)
  } catch {
    throw invalid('content', `PackageMeta content is not a ChunkId: ${key}`)
  }
  if (!['sha256', 'mix256'].includes(chunk.chunkType)) {
    throw invalid('content', `unsupported PackageMeta ChunkId type: ${key}`)
  }
  const hash = ndn.hexToBytes(sha256)
  const expected = chunk.chunkType === 'mix256'
    ? ndn.ChunkId.fromMix256Result(size, hash)
    : ndn.ChunkId.fromSha256Result(hash)
  if (chunk.toString() !== expected.toString()) {
    throw invalid('content', `PackageMeta ChunkId digest mismatch: ${key}`)
  }
}

function parsePackageId(value: string, key: string): { name: string; version: string } {
  const parts = value.split('#')
  if (parts.length !== 2 || !parts[0] || !parts[1] || parts[1].startsWith('$')) {
    throw invalid('namespace', `subpackage ${key} must use an exact-version pkg_id`)
  }
  return { name: parts[0], version: parts[1] }
}

function packageUniqueName(packageName: string, key: string): string {
  const parts = packageName.split('.')
  const environments = new Set([
    'all',
    'nightly-linux-amd64',
    'nightly-linux-aarch64',
    'nightly-windows-amd64',
    'nightly-windows-aarch64',
    'nightly-apple-amd64',
    'nightly-apple-aarch64',
  ])
  const unique = parts.length > 1 && environments.has(parts[0])
    ? parts.slice(1).join('.')
    : parts.join('.')
  if (!unique || unique.split('.').some((label) => !/^[a-z0-9][a-z0-9_-]*$/.test(label))) {
    throw invalid('namespace', `subpackage ${key} has an invalid package name`)
  }
  return unique
}

function decodeJwtClaims(jwt: string): Record<string, unknown> {
  const segments = jwt.split('.')
  if (segments.length !== 3 || !segments[1]) throw invalid('appdoc', 'APPDOC.jwt is malformed')
  try {
    const normalized = segments[1].replaceAll('-', '+').replaceAll('_', '/')
    const padded = normalized.padEnd(Math.ceil(normalized.length / 4) * 4, '=')
    const binary = atob(padded)
    const bytes = Uint8Array.from(binary, (character) => character.charCodeAt(0))
    return parseJsonObject(bytes, APPDOC_JWT_ENTRY)
  } catch (error) {
    if (error instanceof ToolError) throw error
    throw invalid('appdoc', 'APPDOC.jwt claims are not valid JSON')
  }
}

function parseJsonObject(bytes: Uint8Array, label: string): Record<string, unknown> {
  return expectObject(parseJsonValue(bytes, label), label)
}

function parseJsonValue(bytes: Uint8Array, label: string): unknown {
  try {
    return JSON.parse(new TextDecoder('utf-8', { fatal: true }).decode(bytes))
  } catch (error) {
    if (error instanceof ToolError) throw error
    throw invalid('metadata', `${label} is not valid JSON`)
  }
}

function parseDid(value: string, label: string): { method: string; id: string } {
  const match = /^did:([a-z0-9]+):(.+)$/.exec(value)
  if (!match) throw new UsageError('INVALID_DID', `${label} must be a DID`)
  return { method: match[1], id: match[2] }
}

function parseSha256(value: string, label: string): string {
  const match = SHA256_DIGEST.exec(value)
  if (!match) throw invalid('digest', `${label} must be sha256:<64 lowercase hex>`)
  return match[1].toLowerCase()
}

function canonicalSelectorForPackage(
  value: unknown,
  label: string,
): Record<string, string> | undefined {
  try {
    return canonicalSelector(value, label)
  } catch (error) {
    if (error instanceof UsageError) throw invalid('appdoc', error.message)
    throw error
  }
}

function assertSelectorForPackage(key: string, selector: Record<string, string> | undefined): void {
  try {
    assertSelectorCompatible(key, selector, `APPDOC.pkg_list.${key}.selector`)
  } catch (error) {
    if (error instanceof UsageError) throw invalid('appdoc', error.message)
    throw error
  }
}

function validateSubpackageNameForPackage(name: string): void {
  try {
    validateSubpackageName(name)
  } catch (error) {
    if (error instanceof UsageError) throw invalid('appdoc', error.message)
    throw error
  }
}

export function expectObject(value: unknown, label: string): Record<string, unknown> {
  if (!value || typeof value !== 'object' || Array.isArray(value)) {
    throw invalid('schema', `${label} must be an object`)
  }
  return value as Record<string, unknown>
}

export function expectNonEmptyString(value: unknown, label: string): string {
  if (typeof value !== 'string' || !value.trim()) {
    throw invalid('schema', `${label} must be a string`)
  }
  return value
}

function optionalNonEmptyString(value: unknown, label: string): string | undefined {
  return value === undefined ? undefined : expectNonEmptyString(value, label)
}

function expectSafeInteger(value: unknown, label: string): number {
  if (typeof value !== 'number' || !Number.isSafeInteger(value) || value < 0) {
    throw invalid('schema', `${label} must be a non-negative safe integer`)
  }
  return value
}

function expectUint(value: unknown, label: string, maximum: number): number {
  const result = expectSafeInteger(value, label)
  if (result > maximum) throw invalid('schema', `${label} is too large`)
  return result
}

function optionalBoolean(value: unknown, label: string): void {
  if (value !== undefined && typeof value !== 'boolean') {
    throw invalid('schema', `${label} must be boolean`)
  }
}

function optionalString(value: unknown, label: string): void {
  if (value !== undefined && typeof value !== 'string') {
    throw invalid('schema', `${label} must be a string`)
  }
}

function validateStringMap(value: unknown, label: string): Record<string, unknown> {
  const result = expectObject(value, label)
  if (Object.values(result).some((item) => typeof item !== 'string')) {
    throw invalid('schema', `${label} values must be strings`)
  }
  return result
}

function assertKnownFields(
  value: Record<string, unknown>,
  allowed: string[],
  label: string,
): void {
  const accepted = new Set(allowed)
  const unknown = Object.keys(value).find((key) => !accepted.has(key))
  if (unknown) throw invalid('schema', `${label}.${unknown} is not allowed`)
}

export function rejectUnknown(
  value: Record<string, unknown>,
  allowed: string[],
  label: string,
): void {
  const accepted = new Set(allowed)
  const unknown = Object.keys(value).filter((key) => !accepted.has(key))
  if (unknown.length) {
    throw new UsageError('SCHEMA_VALIDATION_FAILED', `${label}.${unknown[0]} is not allowed`)
  }
}

function invalid(stage: string, message: string, entry?: string): ToolError {
  return new ToolError('INVALID_PACKAGE', message, 6, false, {
    stage,
    ...(entry ? { entry: basename(entry) } : {}),
  })
}

export function stableJsonDigest(value: unknown): string {
  return sha256Bytes(new TextEncoder().encode(ndn.toCanonicalJsonString(value)))
}
