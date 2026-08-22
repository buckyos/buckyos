import { basename, dirname, isAbsolute, join, relative, resolve, sep } from 'node:path'
import type { CommandContext } from '../core/context.ts'
import type { CommandModule } from '../core/command.ts'
import { EXIT_PERMISSION, EXIT_UNAVAILABLE, ToolError, UsageError } from '../core/errors.ts'
import { createDeterministicTarGz, digestFile, sha256Bytes } from './pikg_archive.ts'
import {
  APPDOC_ENTRY,
  appDocObjectId,
  assertSelectorCompatible,
  canonicalSelector,
  createPackageMeta,
  deriveAppNamespace,
  derivedSelector,
  DIST_MANIFEST_NAME,
  expectNonEmptyString,
  expectObject,
  inspectPikg,
  normalizeArch,
  PACKAGE_META_ENTRY,
  PACKAGE_META_SCHEMA,
  type PackageMetaFile,
  packSnapshot,
  parsePackageMeta,
  rejectUnknown,
  stableJsonDigest,
  validatePermissions,
  validateServiceConfigTips,
  validateSnapshot,
  validateSubpackageName,
} from './pikg_protocol.ts'

const TOOL_VERSION = '0.1.0-phase3'
const APP_DOCUMENT_LIFETIME_SECONDS = 5 * 365 * 24 * 60 * 60
const SAFE_PIKG_FILE = /^[A-Za-z0-9._-]+\.pikg$/
const APP_NAME = /^[a-z0-9](?:[a-z0-9-]*[a-z0-9])?$/
const VERSION = /^\d+\.\d+\.\d+(?:-[0-9A-Za-z.-]+)?(?:\+[0-9A-Za-z.-]+)?$/
const SHA256_ID = /^sha256:[0-9a-f]{64}$/
const OBJECT_ID = /^(?:appdoc|pkg):[0-9a-f]{64}$/
const SAFE_GENERATED_FILE = /^[A-Za-z0-9._-]+$/
const OBJECT_OUTPUT = { type: 'object' as const, additionalProperties: true }

export interface DockerImageInfo {
  id: string
  architecture: string
  canonicalName: string
}

export interface DockerClient {
  inspect(reference: string): Promise<DockerImageInfo>
  save(imageId: string, destinationTarGz: string): Promise<void>
}

export interface PikgModuleDependencies {
  docker?: DockerClient
  now?: () => number
}

interface AppMeta {
  schema_version: 1
  did: string
  name: string
  version: string
  owner: string
  author: string
  show_name: string
  categories: string[]
  permissions: unknown[]
  selector_type: string
  service_config_tips: Record<string, unknown>
}

interface PathSource {
  type: 'path'
  path: string
}

interface DockerSource {
  type: 'docker-image'
  image: string
}

interface SubpackageInput {
  selector?: Record<string, string>
  required: boolean
  source: PathSource | DockerSource
}

interface PikgMeta {
  schema_version: 1
  output_dir: string
  pikg_file: string
  sub_pkgs: Record<string, SubpackageInput>
}

interface GeneratedFileRecord {
  size: number
  digest: string
}

interface DistManifest {
  schema_version: 1
  tool_version: string
  meta_root_id: string
  source_fingerprint: string
  app_doc_object_id: string
  pikg_file: string
  generated_files: Record<string, GeneratedFileRecord>
  subpackages: Record<
    string,
    { source_kind: string; size: number; digest: string; pkg_objid: string }
  >
}

interface PreparedSubpackage {
  key: string
  input: SubpackageInput
  payloadPath: string
  digest: GeneratedFileRecord
  docker?: DockerImageInfo
}

function packageEnvironmentQualifier(
  key: string,
  selector: Record<string, string> | undefined,
): string {
  const effective = selector ?? derivedSelector(key)
  const os = effective?.os
  const arch = effective?.arch
  if (!os || !arch) return 'all'
  const environmentOs = os === 'macos' ? 'apple' : os
  const environmentArch = arch === 'x86_64' ? 'amd64' : arch
  const qualifier = `nightly-${environmentOs}-${environmentArch}`
  return new Set([
      'nightly-linux-amd64',
      'nightly-linux-aarch64',
      'nightly-windows-amd64',
      'nightly-windows-aarch64',
      'nightly-apple-amd64',
      'nightly-apple-aarch64',
    ]).has(qualifier)
    ? qualifier
    : 'all'
}

export function createPikgModule(dependencies: PikgModuleDependencies = {}): CommandModule {
  const docker = dependencies.docker ?? new LocalDockerClient()
  const now = dependencies.now ?? (() => Math.floor(Date.now() / 1000))
  return {
    name: 'pikg',
    summary: 'Build and verify local PIKG release candidates',
    commands: [
      {
        verb: 'init',
        summary: 'Create local PIKG development metadata',
        description: 'Creates only dapp_meta/app.json and dapp_meta/pikg.json.',
        positionals: [
          { name: 'project_dir', description: 'Existing App project directory', required: false },
        ],
        options: [
          { name: 'name', description: 'App name', type: 'string' },
          { name: 'owner', description: 'Owner DID', type: 'string' },
          {
            name: 'kind',
            description: 'Initial App kind',
            type: 'string',
            enum: ['static-web', 'script', 'docker'],
          },
          {
            name: 'source',
            description: 'Build output path or local Docker image',
            type: 'string',
          },
          { name: 'version', description: 'Initial App version', type: 'string' },
          { name: 'app-did', property: 'app_did', description: 'Explicit App DID', type: 'string' },
        ],
        inputSchema: {
          type: 'object',
          properties: {
            project_dir: { type: 'string', minLength: 1 },
            name: { type: 'string', minLength: 1 },
            owner: { type: 'string', minLength: 1 },
            kind: { type: 'string', enum: ['static-web', 'script', 'docker'] },
            source: { type: 'string', minLength: 1 },
            version: { type: 'string', minLength: 1 },
            app_did: { type: 'string', minLength: 1 },
          },
          additionalProperties: false,
        },
        outputSchema: OBJECT_OUTPUT,
        resultSchemaVersion: 1,
        access: { mode: 'fixed', level: 'write' },
        asyncMode: 'sync',
        requiresSession: false,
        execution: 'local',
        networkAccess: false,
        examples: [
          'buckyos pikg init .',
          'buckyos --non-interactive pikg init . --owner did:bns:root --kind static-web --source ./web/dist',
        ],
        handler: async (ctx, input) => await initCommand(ctx, input, docker),
      },
      {
        verb: 'build',
        summary: 'Build a managed dapp_dist snapshot',
        positionals: [
          { name: 'meta_dir', description: 'dapp_meta directory', required: false },
        ],
        inputSchema: {
          type: 'object',
          properties: { meta_dir: { type: 'string', minLength: 1 } },
          additionalProperties: false,
        },
        outputSchema: OBJECT_OUTPUT,
        resultSchemaVersion: 1,
        access: { mode: 'fixed', level: 'write' },
        asyncMode: 'sync',
        requiresSession: false,
        execution: 'local',
        networkAccess: false,
        examples: ['buckyos pikg build ./dapp_meta'],
        handler: async (ctx, input) => await buildCommand(ctx, input, docker, now),
      },
      {
        verb: 'pack',
        summary: 'Pack and verify a complete PIKG',
        positionals: [
          { name: 'dist_dir', description: 'Managed dapp_dist directory', required: false },
        ],
        inputSchema: {
          type: 'object',
          properties: { dist_dir: { type: 'string', minLength: 1 } },
          additionalProperties: false,
        },
        outputSchema: OBJECT_OUTPUT,
        resultSchemaVersion: 1,
        access: { mode: 'fixed', level: 'write' },
        asyncMode: 'sync',
        requiresSession: false,
        execution: 'local',
        networkAccess: false,
        examples: ['buckyos pikg pack ./dapp_dist'],
        handler: async (ctx, input) => await packCommand(ctx, input),
      },
      {
        verb: 'info',
        summary: 'Strictly verify and inspect a local PIKG',
        positionals: [{ name: 'pikg_path', description: 'Local .pikg file' }],
        inputSchema: {
          type: 'object',
          properties: { pikg_path: { type: 'string', minLength: 1 } },
          required: ['pikg_path'],
          additionalProperties: false,
        },
        outputSchema: OBJECT_OUTPUT,
        resultSchemaVersion: 1,
        access: { mode: 'fixed', level: 'read' },
        asyncMode: 'sync',
        requiresSession: false,
        execution: 'local',
        networkAccess: false,
        examples: ['buckyos pikg info ./dapp_dist/demo-0.1.0.pikg'],
        handler: async (ctx, input) =>
          await inspectPikg(resolveFromCwd(ctx, expectInputString(input, 'pikg_path'))),
      },
      {
        verb: 'clean',
        summary: 'Safely delete a managed dapp_dist',
        positionals: [
          { name: 'meta_dir', description: 'dapp_meta directory', required: false },
        ],
        inputSchema: {
          type: 'object',
          properties: { meta_dir: { type: 'string', minLength: 1 } },
          additionalProperties: false,
        },
        outputSchema: OBJECT_OUTPUT,
        resultSchemaVersion: 1,
        access: { mode: 'fixed', level: 'destructive' },
        asyncMode: 'sync',
        requiresSession: false,
        execution: 'local',
        networkAccess: false,
        examples: ['buckyos --non-interactive --yes pikg clean ./dapp_meta'],
        handler: async (ctx, input) => await cleanCommand(ctx, input),
      },
    ],
  }
}

async function initCommand(
  ctx: CommandContext,
  input: Record<string, unknown>,
  docker: DockerClient,
): Promise<Record<string, unknown>> {
  const projectInput = optionalInputString(input, 'project_dir') ?? '.'
  const projectDir = await realDirectory(resolveFromCwd(ctx, projectInput), 'project directory')
  let name = optionalInputString(input, 'name') ?? normalizeAppName(basename(projectDir))
  const canPrompt = !ctx.config.nonInteractive && !ctx.interactive && ctx.io.inputIsTerminal
  if (!name) name = await requiredPrompt(ctx, canPrompt, 'App name: ', 'name')
  let owner = optionalInputString(input, 'owner')
  if (!owner) owner = await requiredPrompt(ctx, canPrompt, 'Owner DID: ', 'owner')
  let kind = optionalInputString(input, 'kind')
  if (!kind) {
    kind = await requiredPrompt(
      ctx,
      canPrompt,
      'App kind (static-web/script/docker): ',
      'kind',
    )
  }
  if (!['static-web', 'script', 'docker'].includes(kind)) {
    throw new UsageError('INVALID_APP_KIND', `invalid App kind: ${kind}`)
  }
  let source = optionalInputString(input, 'source')
  if (!source) source = await requiredPrompt(ctx, canPrompt, 'Source: ', 'source')
  const version = optionalInputString(input, 'version') ?? '0.1.0'
  validateAppName(name)
  validateVersion(version)
  const derivedDid = deriveAppDid(name, owner)
  const appDid = optionalInputString(input, 'app_did') ?? derivedDid
  deriveAppNamespace(appDid, name, owner)

  let subpackageKey: string
  let sourceValue: Record<string, string>
  let selector: Record<string, string> | undefined
  let sourceSummary: Record<string, unknown>
  if (kind === 'docker') {
    const image = await docker.inspect(source)
    const arch = normalizeArch(image.architecture)
    subpackageKey = arch === 'x86_64'
      ? 'amd64_docker_image'
      : arch === 'aarch64'
      ? 'aarch64_docker_image'
      : (() => {
        throw new UsageError('UNSUPPORTED_ARCHITECTURE', `unsupported Docker architecture: ${arch}`)
      })()
    selector = { os: 'linux', arch }
    sourceValue = { type: 'docker-image', image: source }
    sourceSummary = { type: 'docker-image', image: image.canonicalName, image_id: image.id }
  } else {
    subpackageKey = kind === 'static-web' ? 'web' : 'script'
    const absoluteSource = resolve(projectDir, source)
    const persistedSource = isAbsolute(source)
      ? resolve(source)
      : toPortablePath(relative(join(projectDir, 'dapp_meta'), absoluteSource))
    sourceValue = { type: 'path', path: persistedSource }
    sourceSummary = { type: 'path', path: displaySource(projectDir, absoluteSource) }
    try {
      await Deno.lstat(absoluteSource)
    } catch (error) {
      if (error instanceof Deno.errors.NotFound) {
        await ctx.io.stderr(`warning: source does not exist yet: ${sourceSummary.path}\n`)
      } else throw error
    }
  }

  const appMeta: AppMeta = {
    schema_version: 1,
    did: appDid,
    name,
    version,
    owner,
    author: owner,
    show_name: name,
    categories: [kind === 'static-web' ? 'web' : 'dapp'],
    permissions: [],
    selector_type: kind === 'static-web' ? 'static' : 'single',
    service_config_tips: {},
  }
  const pikgMeta = {
    schema_version: 1,
    output_dir: '../dapp_dist',
    pikg_file: `${name}-${version}.pikg`,
    sub_pkgs: {
      [subpackageKey]: {
        ...(selector ? { selector } : {}),
        required: true,
        source: sourceValue,
      },
    },
  }
  parseAppMeta({ ...appMeta })
  parsePikgMeta(pikgMeta)
  const metaDir = join(projectDir, 'dapp_meta')
  await initializeMetaDirectory(projectDir, metaDir, appMeta, pikgMeta)
  return {
    project_dir: projectDir,
    meta_dir: metaDir,
    generated_files: [join(metaDir, 'app.json'), join(metaDir, 'pikg.json')],
    app: {
      did: appDid,
      name,
      version,
      owner,
      author: owner,
      show_name: name,
      categories: appMeta.categories,
      permissions: [],
      selector_type: appMeta.selector_type,
      service_config_tips: {},
    },
    subpackage: {
      key: subpackageKey,
      kind,
      source: sourceSummary,
      required: true,
      ...(selector ? { selector } : {}),
    },
    output_dir: join(projectDir, 'dapp_dist'),
    pikg_file: pikgMeta.pikg_file,
    next_command: `buckyos pikg build ${metaDir}`,
  }
}

async function buildCommand(
  ctx: CommandContext,
  input: Record<string, unknown>,
  docker: DockerClient,
  now: () => number,
): Promise<Record<string, unknown>> {
  const metaInput = optionalInputString(input, 'meta_dir') ?? './dapp_meta'
  const metaDir = await realDirectory(resolveFromCwd(ctx, metaInput), 'dapp_meta')
  const appMeta = parseAppMeta(await readJson(join(metaDir, 'app.json'), 'app.json'))
  const pikgMeta = parsePikgMeta(await readJson(join(metaDir, 'pikg.json'), 'pikg.json'), appMeta)
  const configuredDist = resolve(metaDir, pikgMeta.output_dir)
  await rejectLeafSymlink(configuredDist, 'UNSAFE_OUTPUT_DIR')
  const distDir = await canonicalTarget(configuredDist)
  const projectDir = dirname(metaDir)
  const sourcePaths: string[] = []
  for (const subpackage of Object.values(pikgMeta.sub_pkgs)) {
    if (subpackage.source.type === 'path') {
      sourcePaths.push(await Deno.realPath(resolve(metaDir, subpackage.source.path)))
    }
  }
  assertBuildPaths(metaDir, distDir, sourcePaths)
  await validateReplaceableDist(distDir, metaRootId(metaDir))
  await Deno.mkdir(dirname(distDir), { recursive: true })
  const temporary = await Deno.makeTempDir({
    dir: dirname(distDir),
    prefix: '.buckyos-pikg-build-',
  })
  try {
    const prepared: PreparedSubpackage[] = []
    for (
      const [key, subpackage] of Object.entries(pikgMeta.sub_pkgs).sort(([a], [b]) =>
        a.localeCompare(b)
      )
    ) {
      const payloadPath = join(temporary, `${key}.tar.gz`)
      let dockerInfo: DockerImageInfo | undefined
      if (subpackage.source.type === 'path') {
        const sourcePath = await Deno.realPath(resolve(metaDir, subpackage.source.path))
        const info = await Deno.stat(sourcePath)
        if (info.isDirectory) {
          await createDeterministicTarGz(sourcePath, payloadPath)
        } else if (info.isFile && sourcePath.toLowerCase().endsWith('.tar.gz')) {
          const before = await digestFile(sourcePath)
          const beforeStat = statIdentity(await Deno.stat(sourcePath))
          await Deno.copyFile(sourcePath, payloadPath)
          const after = await digestFile(sourcePath)
          const afterStat = statIdentity(await Deno.stat(sourcePath))
          const copied = await digestFile(payloadPath)
          if (
            beforeStat !== afterStat || before.size !== after.size ||
            before.sha256 !== after.sha256 || copied.size !== before.size ||
            copied.sha256 !== before.sha256
          ) {
            throw new ToolError('SOURCE_CHANGED', `source changed while copied: ${key}`)
          }
        } else {
          throw new UsageError(
            'INVALID_SOURCE',
            `${key} source must be a directory or .tar.gz file`,
          )
        }
      } else {
        dockerInfo = await docker.inspect(subpackage.source.image)
        if (!SHA256_ID.test(dockerInfo.id)) {
          throw new ToolError(
            'DOCKER_IDENTITY_INVALID',
            `Docker image ${key} has no immutable image ID`,
          )
        }
        await docker.save(dockerInfo.id, payloadPath)
        const after = await docker.inspect(dockerInfo.id)
        if (after.id !== dockerInfo.id) {
          throw new ToolError(
            'SOURCE_CHANGED',
            `Docker image identity changed while exporting: ${key}`,
          )
        }
      }
      const digest = await digestFile(payloadPath)
      prepared.push({
        key,
        input: subpackage,
        payloadPath,
        digest: { size: digest.size, digest: `sha256:${digest.sha256}` },
        ...(dockerInfo ? { docker: dockerInfo } : {}),
      })
    }

    const timestamp = now()
    if (!Number.isSafeInteger(timestamp) || timestamp < 0) throw new Error('invalid clock value')
    const namespace = deriveAppNamespace(appMeta.did, appMeta.name, appMeta.owner)
    const packageObjects: Record<string, Record<string, unknown>> = {}
    const contentIndex: PackageMetaFile['content_index'] = {}
    const pkgList: Record<string, Record<string, unknown>> = {}
    const dependencies: Record<string, string> = {}
    const generatedSubpackages: DistManifest['subpackages'] = {}
    const packageNames = new Set<string>()
    for (const item of prepared) {
      const packageName = `${
        packageEnvironmentQualifier(item.key, item.input.selector)
      }.${namespace}-${packageSuffix(item.key)}`
      if (packageNames.has(packageName)) {
        throw new UsageError('PACKAGE_NAME_COLLISION', `subpackage names collide at ${packageName}`)
      }
      packageNames.add(packageName)
      const payloadHash = item.digest.digest.slice('sha256:'.length)
      const packageMeta = createPackageMeta(
        packageName,
        appMeta.version,
        appMeta.author,
        appMeta.owner,
        { size: item.digest.size, sha256: payloadHash, crc32: 0 },
        timestamp,
      )
      packageObjects[packageMeta.objectId] = packageMeta.value
      dependencies[packageName] = appMeta.version
      if (contentIndex[item.digest.digest]) {
        throw new UsageError(
          'DUPLICATE_PAYLOAD_DIGEST',
          `subpackages cannot share the same payload digest: ${item.key}`,
        )
      }
      contentIndex[item.digest.digest] = {
        sub_pkg_name: item.key,
        path: `${item.key}.tar.gz`,
        format: 'tar.gz',
        size: item.digest.size,
        digest: item.digest.digest,
      }
      const selector = item.input.selector
      pkgList[item.key] = {
        pkg_id: `${packageName}#${appMeta.version}`,
        pkg_objid: packageMeta.objectId,
        ...(item.docker
          ? {
            docker_image_name: item.docker.canonicalName,
            docker_image_digest: item.docker.id,
          }
          : {}),
        ...(selector ? { selector } : {}),
        required: item.input.required,
      }
      generatedSubpackages[item.key] = {
        source_kind: item.input.source.type,
        size: item.digest.size,
        digest: item.digest.digest,
        pkg_objid: packageMeta.objectId,
      }
    }
    const appDoc: Record<string, unknown> = {
      did: appMeta.did,
      name: appMeta.name,
      author: appMeta.author,
      owner: appMeta.owner,
      create_time: timestamp,
      last_update_time: timestamp,
      exp: timestamp + APP_DOCUMENT_LIFETIME_SECONDS,
      categories: appMeta.categories,
      version: appMeta.version,
      deps: dependencies,
      doc_type: 'app',
      pkg_list: pkgList,
      show_name: appMeta.show_name,
      ...(appMeta.permissions.length ? { permissions: appMeta.permissions } : {}),
      selector_type: appMeta.selector_type,
      service_config_tips: appMeta.service_config_tips,
    }
    const appObjectId = appDocObjectId(appDoc)
    const packageMeta: PackageMetaFile = {
      '@schema': PACKAGE_META_SCHEMA,
      app_doc_id: appObjectId,
      package_objects: packageObjects,
      content_index: contentIndex,
    }
    await writeJson(join(temporary, APPDOC_ENTRY), appDoc)
    await writeJson(join(temporary, PACKAGE_META_ENTRY), packageMeta)
    const validated = await validateSnapshot(temporary, appDoc, packageMeta)
    const generatedFiles: DistManifest['generated_files'] = {}
    for (
      const name of [
        APPDOC_ENTRY,
        PACKAGE_META_ENTRY,
        ...prepared.map((item) => `${item.key}.tar.gz`),
      ]
    ) {
      const digest = await digestFile(join(temporary, name))
      generatedFiles[name] = { size: digest.size, digest: `sha256:${digest.sha256}` }
    }
    const manifest: DistManifest = {
      schema_version: 1,
      tool_version: TOOL_VERSION,
      meta_root_id: metaRootId(metaDir),
      source_fingerprint: `sha256:${
        stableJsonDigest({
          app: appMeta,
          pikg: pikgMeta,
          payloads: generatedSubpackages,
        })
      }`,
      app_doc_object_id: appObjectId,
      pikg_file: pikgMeta.pikg_file,
      generated_files: generatedFiles,
      subpackages: generatedSubpackages,
    }
    await writeJson(join(temporary, DIST_MANIFEST_NAME), manifest)
    await replaceDirectory(temporary, distDir)
    return {
      meta_dir: metaDir,
      dist_dir: distDir,
      app_did: appMeta.did,
      app_doc_object_id: appObjectId,
      subpackage_count: validated.subpackages.length,
      subpackages: validated.subpackages.map((item) => ({
        key: item.key,
        source_kind: pikgMeta.sub_pkgs[item.key].source.type,
        size: item.payload_size,
        digest: item.payload_digest,
        pkg_objid: item.pkg_objid,
      })),
      ready_for_pack: true,
      next_command: `buckyos pikg pack ${distDir}`,
      project_dir: projectDir,
    }
  } catch (error) {
    await removeTreeIfExists(temporary)
    throw error
  }
}

async function packCommand(
  ctx: CommandContext,
  input: Record<string, unknown>,
): Promise<Record<string, unknown>> {
  const distInput = optionalInputString(input, 'dist_dir') ?? './dapp_dist'
  const distDir = await safeExistingDirectory(resolveFromCwd(ctx, distInput), 'dapp_dist')
  const manifest = await validateManagedDist(distDir, undefined, true)
  const appDoc = await readJson(join(distDir, APPDOC_ENTRY), APPDOC_ENTRY)
  const packageMeta = parsePackageMeta(
    await readJson(join(distDir, PACKAGE_META_ENTRY), PACKAGE_META_ENTRY),
  )
  const validated = await validateSnapshot(distDir, appDoc, packageMeta)
  if (validated.appDocObjectId !== manifest.app_doc_object_id) {
    throw new ToolError(
      'INVALID_PACKAGE',
      'snapshot AppDoc Object ID differs from ownership manifest',
    )
  }
  const temporary = join(distDir, `.${manifest.pikg_file}.tmp-${crypto.randomUUID()}`)
  const finalPath = join(distDir, manifest.pikg_file)
  try {
    await packSnapshot(distDir, temporary, appDoc, packageMeta)
    const inspection = await inspectPikg(temporary)
    if (inspection.app.app_doc_object_id !== manifest.app_doc_object_id) {
      throw new ToolError(
        'INVALID_PACKAGE',
        'PIKG self-check returned a different AppDoc Object ID',
      )
    }
    await replaceFile(temporary, finalPath)
    const digest = await digestFile(finalPath)
    return {
      dist_dir: distDir,
      pikg_path: finalPath,
      size: digest.size,
      pikg_digest: `sha256:${digest.sha256}`,
      app_doc_object_id: inspection.app.app_doc_object_id,
      validation: 'passed',
    }
  } catch (error) {
    await removeFileIfExists(temporary)
    throw error
  }
}

async function cleanCommand(
  ctx: CommandContext,
  input: Record<string, unknown>,
): Promise<Record<string, unknown>> {
  const metaInput = optionalInputString(input, 'meta_dir') ?? './dapp_meta'
  const metaDir = await realDirectory(resolveFromCwd(ctx, metaInput), 'dapp_meta')
  const appMeta = parseAppMeta(await readJson(join(metaDir, 'app.json'), 'app.json'))
  const pikgMeta = parsePikgMeta(
    await readJson(join(metaDir, 'pikg.json'), 'pikg.json'),
    appMeta,
  )
  const configuredDist = resolve(metaDir, pikgMeta.output_dir)
  await rejectLeafSymlink(configuredDist, 'UNSAFE_CLEAN_TARGET')
  const distDir = await canonicalTarget(configuredDist)
  try {
    await Deno.lstat(distDir)
  } catch (error) {
    if (error instanceof Deno.errors.NotFound) {
      return { meta_dir: metaDir, dist_dir: distDir, removed: false }
    }
    throw error
  }
  const sourcePaths: string[] = []
  for (const subpackage of Object.values(pikgMeta.sub_pkgs)) {
    if (subpackage.source.type === 'path') {
      try {
        sourcePaths.push(await Deno.realPath(resolve(metaDir, subpackage.source.path)))
      } catch (error) {
        if (!(error instanceof Deno.errors.NotFound)) throw error
      }
    }
  }
  assertSafeCleanTarget(ctx, metaDir, distDir, sourcePaths)
  await validateManagedDist(distDir, metaRootId(metaDir), true, 'UNSAFE_CLEAN_TARGET')
  if (!ctx.confirmed) {
    if (ctx.config.nonInteractive || ctx.interactive || !ctx.io.inputIsTerminal) {
      throw new ToolError(
        'CONFIRMATION_REQUIRED',
        'pikg clean requires --yes in non-interactive mode',
        EXIT_PERMISSION,
      )
    }
    const answer = await ctx.io.prompt(`Delete managed PIKG output ${distDir}? [y/N] `)
    if (!answer || !['y', 'yes'].includes(answer.trim().toLowerCase())) {
      throw new ToolError('CONFIRMATION_DECLINED', 'PIKG clean was declined', EXIT_PERMISSION)
    }
  }
  await Deno.remove(distDir, { recursive: true })
  return { meta_dir: metaDir, dist_dir: distDir, removed: true }
}

function parseAppMeta(value: Record<string, unknown>): AppMeta {
  rejectUnknown(
    value,
    [
      'schema_version',
      'did',
      'name',
      'version',
      'owner',
      'author',
      'show_name',
      'categories',
      'permissions',
      'selector_type',
      'service_config_tips',
    ],
    'app.json',
  )
  if (value.schema_version !== 1) {
    throw new UsageError('UNSUPPORTED_SCHEMA_VERSION', 'app.json.schema_version must be 1')
  }
  const name = developmentString(value.name, 'app.json.name')
  validateAppName(name)
  const version = developmentString(value.version, 'app.json.version')
  validateVersion(version)
  const did = developmentString(value.did, 'app.json.did')
  const owner = developmentString(value.owner, 'app.json.owner')
  deriveAppNamespace(did, name, owner)
  const categories = value.categories
  if (
    !Array.isArray(categories) || !categories.length ||
    categories.some((item) => typeof item !== 'string')
  ) {
    throw new UsageError(
      'SCHEMA_VALIDATION_FAILED',
      'app.json.categories must be a non-empty string array',
    )
  }
  const permissions = developmentValidation(
    () => validatePermissions(value.permissions ?? [], 'app.json.permissions'),
  )
  const serviceConfig = developmentValidation(() =>
    validateServiceConfigTips(
      value.service_config_tips ?? {},
      'app.json.service_config_tips',
    )
  )
  return {
    schema_version: 1,
    did,
    name,
    version,
    owner,
    author: developmentString(value.author, 'app.json.author'),
    show_name: developmentString(value.show_name, 'app.json.show_name'),
    categories: categories as string[],
    permissions,
    selector_type: developmentString(value.selector_type, 'app.json.selector_type'),
    service_config_tips: serviceConfig,
  }
}

function parsePikgMeta(value: Record<string, unknown>, app?: AppMeta): PikgMeta {
  rejectUnknown(value, ['schema_version', 'output_dir', 'pikg_file', 'sub_pkgs'], 'pikg.json')
  if (value.schema_version !== 1) {
    throw new UsageError('UNSUPPORTED_SCHEMA_VERSION', 'pikg.json.schema_version must be 1')
  }
  const outputDir = value.output_dir === undefined
    ? '../dapp_dist'
    : developmentString(value.output_dir, 'pikg.json.output_dir')
  const defaultPikgFile = app ? `${app.name}-${app.version}.pikg` : undefined
  const pikgFile = value.pikg_file === undefined
    ? defaultPikgFile ?? (() => {
      throw new UsageError('SCHEMA_VALIDATION_FAILED', 'pikg.json.pikg_file is required')
    })()
    : developmentString(value.pikg_file, 'pikg.json.pikg_file')
  if (
    !SAFE_PIKG_FILE.test(pikgFile) || pikgFile.includes('..') || basename(pikgFile) !== pikgFile
  ) {
    throw new UsageError('INVALID_PIKG_FILE', 'pikg_file must be a safe .pikg file name')
  }
  const rawSubpackages = developmentObject(value.sub_pkgs, 'pikg.json.sub_pkgs')
  if (!Object.keys(rawSubpackages).length) {
    throw new UsageError('SCHEMA_VALIDATION_FAILED', 'pikg.json.sub_pkgs must not be empty')
  }
  const subpackages: Record<string, SubpackageInput> = {}
  for (const [key, raw] of Object.entries(rawSubpackages)) {
    validateSubpackageName(key)
    const subpackage = developmentObject(raw, `pikg.json.sub_pkgs.${key}`)
    rejectUnknown(subpackage, ['selector', 'required', 'source'], `pikg.json.sub_pkgs.${key}`)
    const selector = developmentValidation(() =>
      canonicalSelector(subpackage.selector, `pikg.json.sub_pkgs.${key}.selector`)
    )
    assertSelectorCompatible(key, selector, `pikg.json.sub_pkgs.${key}.selector`)
    if (subpackage.required !== undefined && typeof subpackage.required !== 'boolean') {
      throw new UsageError('SCHEMA_VALIDATION_FAILED', `${key}.required must be boolean`)
    }
    const source = developmentObject(subpackage.source, `pikg.json.sub_pkgs.${key}.source`)
    if (source.type === 'path') {
      rejectUnknown(source, ['type', 'path'], `pikg.json.sub_pkgs.${key}.source`)
      subpackages[key] = {
        ...(selector ? { selector } : {}),
        required: subpackage.required === undefined ? true : subpackage.required,
        source: { type: 'path', path: developmentString(source.path, `${key}.source.path`) },
      }
    } else if (source.type === 'docker-image') {
      rejectUnknown(source, ['type', 'image'], `pikg.json.sub_pkgs.${key}.source`)
      subpackages[key] = {
        ...(selector ? { selector } : {}),
        required: subpackage.required === undefined ? true : subpackage.required,
        source: {
          type: 'docker-image',
          image: developmentString(source.image, `${key}.source.image`),
        },
      }
    } else {
      throw new UsageError('INVALID_SOURCE', `${key}.source.type must be path or docker-image`)
    }
  }
  return {
    schema_version: 1,
    output_dir: outputDir,
    pikg_file: pikgFile,
    sub_pkgs: subpackages,
  }
}

async function initializeMetaDirectory(
  projectDir: string,
  metaDir: string,
  appMeta: AppMeta,
  pikgMeta: Record<string, unknown>,
): Promise<void> {
  let existedEmpty = false
  try {
    const stat = await Deno.lstat(metaDir)
    if (!stat.isDirectory || stat.isSymlink) {
      throw new UsageError(
        'ALREADY_EXISTS',
        'dapp_meta already exists and is not an empty directory',
      )
    }
    for await (const _entry of Deno.readDir(metaDir)) {
      throw new UsageError('ALREADY_EXISTS', 'dapp_meta already contains files')
    }
    existedEmpty = true
  } catch (error) {
    if (!(error instanceof Deno.errors.NotFound)) throw error
  }
  const temporary = await Deno.makeTempDir({ dir: projectDir, prefix: '.buckyos-pikg-init-' })
  try {
    await writeJson(join(temporary, 'app.json'), appMeta)
    await writeJson(join(temporary, 'pikg.json'), pikgMeta)
    parseAppMeta(await readJson(join(temporary, 'app.json'), 'app.json'))
    parsePikgMeta(await readJson(join(temporary, 'pikg.json'), 'pikg.json'), appMeta)
    if (existedEmpty) await Deno.remove(metaDir)
    try {
      await Deno.rename(temporary, metaDir)
    } catch (error) {
      if (existedEmpty) await Deno.mkdir(metaDir)
      throw error
    }
  } catch (error) {
    await removeTreeIfExists(temporary)
    throw error
  }
}

async function validateReplaceableDist(distDir: string, expectedMetaRootId: string): Promise<void> {
  try {
    await Deno.lstat(distDir)
  } catch (error) {
    if (error instanceof Deno.errors.NotFound) return
    throw error
  }
  await validateManagedDist(distDir, expectedMetaRootId, true)
}

async function validateManagedDist(
  distDir: string,
  expectedMetaRootId?: string,
  verifyFiles = true,
  unsafeCode = 'UNSAFE_DIST_TARGET',
): Promise<DistManifest> {
  try {
    const info = await Deno.lstat(distDir)
    if (!info.isDirectory || info.isSymlink) throw new Error('target is not a real directory')
    const manifest = parseDistManifest(
      await readJson(join(distDir, DIST_MANIFEST_NAME), DIST_MANIFEST_NAME),
    )
    if (expectedMetaRootId && manifest.meta_root_id !== expectedMetaRootId) {
      throw new Error('ownership manifest belongs to another dapp_meta')
    }
    const allowed = new Set([
      DIST_MANIFEST_NAME,
      manifest.pikg_file,
      ...Object.keys(manifest.generated_files),
    ])
    for await (const entry of Deno.readDir(distDir)) {
      if (!entry.isFile || entry.isSymlink || !allowed.has(entry.name)) {
        throw new Error(`unmanaged entry exists: ${entry.name}`)
      }
    }
    for (const name of Object.keys(manifest.generated_files)) {
      const info = await Deno.lstat(join(distDir, name))
      if (!info.isFile || info.isSymlink) throw new Error(`generated file is unsafe: ${name}`)
      if (verifyFiles) {
        const expected = manifest.generated_files[name]
        const actual = await digestFile(join(distDir, name))
        if (actual.size !== expected.size || `sha256:${actual.sha256}` !== expected.digest) {
          throw new Error(`generated file was modified: ${name}`)
        }
      }
    }
    return manifest
  } catch (error) {
    if (error instanceof ToolError && error.code === unsafeCode) throw error
    throw new ToolError(
      unsafeCode,
      `managed dist validation failed: ${error instanceof Error ? error.message : String(error)}`,
      6,
    )
  }
}

function parseDistManifest(value: Record<string, unknown>): DistManifest {
  rejectUnknown(
    value,
    [
      'schema_version',
      'tool_version',
      'meta_root_id',
      'source_fingerprint',
      'app_doc_object_id',
      'pikg_file',
      'generated_files',
      'subpackages',
    ],
    DIST_MANIFEST_NAME,
  )
  if (value.schema_version !== 1) throw new Error('unsupported ownership manifest version')
  const pikgFile = String(value.pikg_file ?? '')
  if (
    !SAFE_PIKG_FILE.test(pikgFile) || pikgFile.includes('..') || basename(pikgFile) !== pikgFile
  ) {
    throw new Error('ownership manifest has an unsafe pikg_file')
  }
  const generatedRaw = expectObject(value.generated_files, `${DIST_MANIFEST_NAME}.generated_files`)
  const generated: Record<string, GeneratedFileRecord> = {}
  for (const [name, raw] of Object.entries(generatedRaw)) {
    if (
      !SAFE_GENERATED_FILE.test(name) || name === '.' || name === '..' || name.includes('\\') ||
      basename(name) !== name || name === DIST_MANIFEST_NAME || name === pikgFile
    ) {
      throw new Error(`ownership manifest has an unsafe generated file: ${name}`)
    }
    const record = expectObject(raw, `generated_files.${name}`)
    rejectUnknown(record, ['size', 'digest'], `generated_files.${name}`)
    if (typeof record.size !== 'number' || !Number.isSafeInteger(record.size) || record.size < 0) {
      throw new Error(`generated_files.${name}.size is invalid`)
    }
    if (typeof record.digest !== 'string' || !SHA256_ID.test(record.digest)) {
      throw new Error(`generated_files.${name}.digest is invalid`)
    }
    generated[name] = { size: record.size, digest: record.digest }
  }
  if (!generated[APPDOC_ENTRY] || !generated[PACKAGE_META_ENTRY]) {
    throw new Error('ownership manifest is missing required metadata files')
  }
  return {
    schema_version: 1,
    tool_version: String(value.tool_version ?? ''),
    meta_root_id: validateDigestString(value.meta_root_id, 'meta_root_id'),
    source_fingerprint: validateDigestString(value.source_fingerprint, 'source_fingerprint'),
    app_doc_object_id: validateObjectId(value.app_doc_object_id, 'app_doc_object_id'),
    pikg_file: pikgFile,
    generated_files: generated,
    subpackages: expectObject(value.subpackages, 'subpackages') as DistManifest['subpackages'],
  }
}

function assertBuildPaths(metaDir: string, distDir: string, sourcePaths: string[]): void {
  if (pathsOverlap(metaDir, distDir)) {
    throw new UsageError('UNSAFE_OUTPUT_DIR', 'output_dir overlaps dapp_meta')
  }
  for (const source of sourcePaths) {
    if (pathsOverlap(source, distDir)) {
      throw new UsageError('UNSAFE_OUTPUT_DIR', 'output_dir overlaps a subpackage source')
    }
  }
}

function assertSafeCleanTarget(
  ctx: CommandContext,
  metaDir: string,
  distDir: string,
  sourcePaths: string[],
): void {
  const projectDir = dirname(metaDir)
  const exactProtected = [resolve(ctx.cwd), projectDir]
  const home = Deno.env.get('HOME') ?? Deno.env.get('USERPROFILE')
  if (home) exactProtected.push(resolve(home))
  if (
    dirname(distDir) === distDir ||
    exactProtected.some((path) => resolve(path) === resolve(distDir)) ||
    pathsOverlap(metaDir, distDir) || sourcePaths.some((path) => pathsOverlap(path, distDir))
  ) {
    throw new ToolError('UNSAFE_CLEAN_TARGET', 'refusing to clean an unsafe output directory')
  }
}

async function replaceDirectory(temporary: string, destination: string): Promise<void> {
  let exists = false
  try {
    await Deno.lstat(destination)
    exists = true
  } catch (error) {
    if (!(error instanceof Deno.errors.NotFound)) throw error
  }
  if (!exists) {
    await Deno.rename(temporary, destination)
    return
  }
  const backup = `${destination}.previous-${crypto.randomUUID()}`
  await Deno.rename(destination, backup)
  try {
    await Deno.rename(temporary, destination)
  } catch (error) {
    await Deno.rename(backup, destination)
    throw error
  }
  await Deno.remove(backup, { recursive: true })
}

async function replaceFile(temporary: string, destination: string): Promise<void> {
  let exists = false
  try {
    await Deno.lstat(destination)
    exists = true
  } catch (error) {
    if (!(error instanceof Deno.errors.NotFound)) throw error
  }
  if (!exists) {
    await Deno.rename(temporary, destination)
    return
  }
  const backup = `${destination}.previous-${crypto.randomUUID()}`
  await Deno.rename(destination, backup)
  try {
    await Deno.rename(temporary, destination)
  } catch (error) {
    await Deno.rename(backup, destination)
    throw error
  }
  await removeFileIfExists(backup)
}

class LocalDockerClient implements DockerClient {
  async inspect(reference: string): Promise<DockerImageInfo> {
    let result: Deno.CommandOutput
    try {
      result = await new Deno.Command('docker', {
        args: ['image', 'inspect', reference],
        stdout: 'piped',
        stderr: 'piped',
      }).output()
    } catch (error) {
      throw new ToolError(
        'DOCKER_UNAVAILABLE',
        `Docker inspect is unavailable: ${error instanceof Error ? error.message : String(error)}`,
        EXIT_UNAVAILABLE,
        true,
      )
    }
    if (!result.success) {
      throw new ToolError(
        'DOCKER_IMAGE_NOT_FOUND',
        `local Docker image is unavailable: ${safeDockerError(result.stderr)}`,
        EXIT_UNAVAILABLE,
      )
    }
    let values: unknown
    try {
      values = JSON.parse(new TextDecoder().decode(result.stdout))
    } catch {
      throw new ToolError('DOCKER_INSPECT_INVALID', 'Docker inspect returned invalid JSON')
    }
    if (!Array.isArray(values) || values.length !== 1) {
      throw new ToolError('DOCKER_INSPECT_INVALID', 'Docker inspect returned an unexpected result')
    }
    const value = developmentObject(values[0], 'Docker inspect')
    const id = developmentString(value.Id, 'Docker image ID').toLowerCase()
    if (!SHA256_ID.test(id)) {
      throw new ToolError('DOCKER_IDENTITY_INVALID', 'Docker image ID is not immutable sha256')
    }
    const architecture = developmentString(value.Architecture, 'Docker architecture')
    const tags = Array.isArray(value.RepoTags)
      ? value.RepoTags.filter((tag): tag is string =>
        typeof tag === 'string' && tag !== '<none>:<none>'
      )
      : []
    return {
      id,
      architecture,
      canonicalName: tags.includes(reference) ? reference : tags[0] ?? reference,
    }
  }

  async save(imageId: string, destinationTarGz: string): Promise<void> {
    let child: Deno.ChildProcess
    try {
      child = new Deno.Command('docker', {
        args: ['image', 'save', imageId],
        stdout: 'piped',
        stderr: 'piped',
      }).spawn()
    } catch (error) {
      throw new ToolError(
        'DOCKER_UNAVAILABLE',
        `Docker save is unavailable: ${error instanceof Error ? error.message : String(error)}`,
        EXIT_UNAVAILABLE,
        true,
      )
    }
    const output = await Deno.open(destinationTarGz, { createNew: true, write: true, mode: 0o600 })
    const stderrPromise = new Response(child.stderr).arrayBuffer()
    try {
      const [status, stderr] = await Promise.all([
        child.status,
        stderrPromise,
        child.stdout.pipeThrough(new CompressionStream('gzip')).pipeTo(output.writable),
      ])
      if (!status.success) {
        throw new ToolError(
          'DOCKER_EXPORT_FAILED',
          `Docker image save failed: ${safeDockerError(new Uint8Array(stderr))}`,
          EXIT_UNAVAILABLE,
        )
      }
    } catch (error) {
      await removeFileIfExists(destinationTarGz)
      throw error
    } finally {
      try {
        output.close()
      } catch {
        // The pipe closes the file resource.
      }
    }
  }
}

function resolveFromCwd(ctx: CommandContext, value: string): string {
  return isAbsolute(value) ? resolve(value) : resolve(ctx.cwd, value)
}

async function realDirectory(path: string, label: string): Promise<string> {
  const real = await Deno.realPath(path)
  const stat = await Deno.stat(real)
  if (!stat.isDirectory) throw new UsageError('INVALID_PATH', `${label} must be a directory`)
  return real
}

async function safeExistingDirectory(path: string, label: string): Promise<string> {
  const info = await Deno.lstat(path)
  if (!info.isDirectory || info.isSymlink) {
    throw new UsageError('INVALID_PATH', `${label} must be a real directory`)
  }
  return await Deno.realPath(path)
}

async function canonicalTarget(path: string): Promise<string> {
  const absolute = resolve(path)
  try {
    return await Deno.realPath(absolute)
  } catch (error) {
    if (!(error instanceof Deno.errors.NotFound)) throw error
    const parent = dirname(absolute)
    if (parent === absolute) return absolute
    return join(await canonicalTarget(parent), basename(absolute))
  }
}

async function rejectLeafSymlink(path: string, code: string): Promise<void> {
  try {
    if ((await Deno.lstat(path)).isSymlink) {
      throw new ToolError(code, 'configured output_dir must not be a symlink')
    }
  } catch (error) {
    if (error instanceof Deno.errors.NotFound) return
    throw error
  }
}

function pathsOverlap(left: string, right: string): boolean {
  const leftToRight = relative(resolve(left), resolve(right))
  const rightToLeft = relative(resolve(right), resolve(left))
  return leftToRight === '' || !leftToRight.startsWith(`..${sep}`) && leftToRight !== '..' ||
    !rightToLeft.startsWith(`..${sep}`) && rightToLeft !== '..'
}

function metaRootId(metaDir: string): string {
  return `sha256:${sha256Bytes(new TextEncoder().encode(resolve(metaDir)))}`
}

function packageSuffix(key: string): string {
  const suffix = key.toLowerCase().replaceAll(/[^a-z0-9_-]+/g, '-').replaceAll(/^-+|-+$/g, '')
  if (!suffix) {
    throw new UsageError('INVALID_SUBPACKAGE_NAME', `cannot derive package name from ${key}`)
  }
  return suffix
}

function normalizeAppName(value: string): string {
  return value.toLowerCase().replaceAll(/[^a-z0-9]+/g, '-').replaceAll(/^-+|-+$/g, '')
}

function validateAppName(value: string): void {
  if (!APP_NAME.test(value)) throw new UsageError('INVALID_APP_NAME', `invalid App name: ${value}`)
}

function validateVersion(value: string): void {
  if (!VERSION.test(value)) throw new UsageError('INVALID_VERSION', `invalid App version: ${value}`)
}

function deriveAppDid(name: string, owner: string): string {
  const match = /^did:bns:([a-z0-9](?:[a-z0-9-]*[a-z0-9])?)$/.exec(owner)
  if (!match) {
    throw new UsageError(
      'INVALID_OWNER_DID',
      'Owner DID must be a standard single-label did:bns DID for an ordinary App',
    )
  }
  return `did:bns:${name}.${match[1]}`
}

async function requiredPrompt(
  ctx: CommandContext,
  canPrompt: boolean,
  message: string,
  field: string,
): Promise<string> {
  if (!canPrompt) {
    throw new UsageError('MISSING_REQUIRED_INPUT', `${field} is required`)
  }
  const answer = await ctx.io.prompt(message)
  if (!answer?.trim()) throw new UsageError('MISSING_REQUIRED_INPUT', `${field} is required`)
  return answer.trim()
}

function optionalInputString(input: Record<string, unknown>, key: string): string | undefined {
  return input[key] === undefined ? undefined : expectInputString(input, key)
}

function expectInputString(input: Record<string, unknown>, key: string): string {
  const value = input[key]
  if (typeof value !== 'string' || !value) {
    throw new UsageError('MISSING_REQUIRED_INPUT', `${key} is required`)
  }
  return value
}

function developmentString(value: unknown, label: string): string {
  try {
    return expectNonEmptyString(value, label)
  } catch (error) {
    if (error instanceof ToolError) throw new UsageError('SCHEMA_VALIDATION_FAILED', error.message)
    throw error
  }
}

function developmentObject(value: unknown, label: string): Record<string, unknown> {
  try {
    return expectObject(value, label)
  } catch (error) {
    if (error instanceof ToolError) throw new UsageError('SCHEMA_VALIDATION_FAILED', error.message)
    throw error
  }
}

function developmentValidation<T>(validate: () => T): T {
  try {
    return validate()
  } catch (error) {
    if (error instanceof ToolError) throw new UsageError('SCHEMA_VALIDATION_FAILED', error.message)
    throw error
  }
}

async function readJson(path: string, label: string): Promise<Record<string, unknown>> {
  try {
    const parsed = JSON.parse(await Deno.readTextFile(path))
    return developmentObject(parsed, label)
  } catch (error) {
    if (error instanceof SyntaxError) {
      throw new UsageError('INVALID_JSON', `${label} is not valid JSON`)
    }
    throw error
  }
}

async function writeJson(path: string, value: unknown): Promise<void> {
  await Deno.writeTextFile(path, `${JSON.stringify(value, null, 2)}\n`, {
    createNew: true,
    mode: 0o600,
  })
}

function validateDigestString(value: unknown, label: string): string {
  if (typeof value !== 'string' || !SHA256_ID.test(value)) throw new Error(`${label} is invalid`)
  return value
}

function validateObjectId(value: unknown, label: string): string {
  if (typeof value !== 'string' || !OBJECT_ID.test(value)) throw new Error(`${label} is invalid`)
  return value
}

function statIdentity(value: Deno.FileInfo): string {
  return JSON.stringify({
    size: value.size,
    mtime: value.mtime?.getTime() ?? null,
    dev: value.dev,
    ino: value.ino,
    mode: value.mode,
  })
}

function displaySource(projectDir: string, source: string): string {
  const display = relative(projectDir, source)
  return display && !display.startsWith(`..${sep}`) && display !== '..'
    ? toPortablePath(display)
    : '[external path]'
}

function toPortablePath(path: string): string {
  return path.split(sep).join('/') || '.'
}

function safeDockerError(bytes: Uint8Array): string {
  const text = new TextDecoder().decode(bytes).trim().replaceAll(/[\r\n]+/g, ' ')
  return text.slice(0, 500) || 'unknown Docker error'
}

async function removeTreeIfExists(path: string): Promise<void> {
  try {
    await Deno.remove(path, { recursive: true })
  } catch (error) {
    if (!(error instanceof Deno.errors.NotFound)) throw error
  }
}

async function removeFileIfExists(path: string): Promise<void> {
  try {
    await Deno.remove(path)
  } catch (error) {
    if (!(error instanceof Deno.errors.NotFound)) throw error
  }
}
