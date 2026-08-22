import { join, parse as parsePath } from 'node:path'
import { BuckyOSToolApplication, type ToolStdio } from '../core/app.ts'
import { createDeterministicTarGz, digestFile } from '../modules/pikg_archive.ts'
import type { DockerClient, DockerImageInfo } from '../modules/pikg.ts'
import { assert, assertEquals, assertRejects } from './test_helpers.ts'

class CaptureStdio implements ToolStdio {
  stdoutText = ''
  stderrText = ''

  stdout(value: string): Promise<void> {
    this.stdoutText += value
    return Promise.resolve()
  }

  stderr(value: string): Promise<void> {
    this.stderrText += value
    return Promise.resolve()
  }

  readStdin(): Promise<string> {
    return Promise.resolve('')
  }

  takeEnvelope() {
    const value = JSON.parse(this.stdoutText)
    this.stdoutText = ''
    return value
  }
}

class FakeDocker implements DockerClient {
  inspectReferences: string[] = []
  saveReferences: string[] = []
  readonly info: DockerImageInfo = {
    id: `sha256:${'ab'.repeat(32)}`,
    architecture: 'amd64',
    canonicalName: 'local/demo:0.1.0-amd64',
  }

  inspect(reference: string): Promise<DockerImageInfo> {
    this.inspectReferences.push(reference)
    return Promise.resolve(this.info)
  }

  async save(imageId: string, destinationTarGz: string): Promise<void> {
    this.saveReferences.push(imageId)
    const body = new TextEncoder().encode('local docker image fixture')
    const compressed = await new Response(
      new Blob([body]).stream().pipeThrough(new CompressionStream('gzip')),
    ).bytes()
    await Deno.writeFile(destinationTarGz, compressed, { createNew: true })
  }
}

Deno.test('pikg path workflow is offline, self-verifying, and safely cleanable', async () => {
  const root = await Deno.makeTempDir()
  try {
    const project = join(root, 'demo')
    await Deno.mkdir(join(project, 'web', 'dist'), { recursive: true })
    await Deno.writeTextFile(join(project, 'web', 'dist', 'index.html'), '<h1>Hello</h1>\n')
    const io = new CaptureStdio()
    let authenticationCreated = false
    const app = new BuckyOSToolApplication({
      cwd: project,
      homeDir: join(root, 'home'),
      environment: { HOME: join(root, 'home') },
      stdio: io,
      pikg: { now: () => 1_800_000_000 },
      createAuthentication: () => {
        authenticationCreated = true
        throw new Error('PIKG must not create authentication')
      },
    })

    assertEquals(
      await app.run([
        '--non-interactive',
        'pikg',
        'init',
        '.',
        '--owner',
        'did:bns:root',
        '--kind',
        'static-web',
        '--source',
        './web/dist',
      ]),
      0,
    )
    let envelope = io.takeEnvelope()
    assertEquals(envelope.data.app.did, 'did:bns:demo.root')
    assertEquals(envelope.data.subpackage.key, 'web')
    assertEquals(authenticationCreated, false)

    const appMetaPath = join(project, 'dapp_meta', 'app.json')
    const appMeta = JSON.parse(await Deno.readTextFile(appMetaPath))
    appMeta.pkg_list = {}
    await Deno.writeTextFile(appMetaPath, `${JSON.stringify(appMeta, null, 2)}\n`)
    assertEquals(await app.run(['pikg', 'build']), 2)
    envelope = io.takeEnvelope()
    assertEquals(envelope.error.code, 'SCHEMA_VALIDATION_FAILED')
    delete appMeta.pkg_list
    await Deno.writeTextFile(appMetaPath, `${JSON.stringify(appMeta, null, 2)}\n`)

    const pikgMetaPath = join(project, 'dapp_meta', 'pikg.json')
    const pikgMeta = JSON.parse(await Deno.readTextFile(pikgMetaPath))
    pikgMeta.sub_pkgs.web.selector = 'linux'
    await Deno.writeTextFile(pikgMetaPath, `${JSON.stringify(pikgMeta, null, 2)}\n`)
    assertEquals(await app.run(['pikg', 'build']), 2)
    envelope = io.takeEnvelope()
    assertEquals(envelope.error.code, 'SCHEMA_VALIDATION_FAILED')
    delete pikgMeta.sub_pkgs.web.selector
    await Deno.writeTextFile(pikgMetaPath, `${JSON.stringify(pikgMeta, null, 2)}\n`)

    const buildExit = await app.run(['pikg', 'build'])
    assertEquals(buildExit, 0, io.stdoutText)
    envelope = io.takeEnvelope()
    assertEquals(envelope.data.ready_for_pack, true)
    assertEquals(envelope.data.subpackage_count, 1)
    assertEquals(authenticationCreated, false)

    const packExit = await app.run(['pikg', 'pack'])
    assertEquals(packExit, 0, io.stdoutText)
    envelope = io.takeEnvelope()
    assertEquals(envelope.data.validation, 'passed')
    const pikgPath = envelope.data.pikg_path
    const originalPikg = await Deno.readFile(pikgPath)

    assertEquals(await app.run(['pikg', 'info', pikgPath]), 0)
    envelope = io.takeEnvelope()
    assertEquals(envelope.data.valid, true)
    assertEquals(envelope.data.offline_content_validation, 'passed')
    assertEquals(envelope.data.signature_validation, 'not-present')
    assertEquals(envelope.data.publication_validation, 'not-checked')

    const tamperedPikg = originalPikg.slice()
    tamperedPikg[Math.floor(tamperedPikg.length / 2)] ^= 1
    const tamperedPath = join(project, 'tampered.pikg')
    await Deno.writeFile(tamperedPath, tamperedPikg)
    assertEquals(await app.run(['pikg', 'info', tamperedPath]), 6)
    envelope = io.takeEnvelope()
    assertEquals(envelope.error.code, 'INVALID_PACKAGE')

    const payloadPath = join(project, 'dapp_dist', 'web.tar.gz')
    const originalPayload = await Deno.readFile(payloadPath)
    await Deno.writeFile(payloadPath, new Uint8Array([...originalPayload, 0]))
    assertEquals(await app.run(['pikg', 'pack']), 6)
    io.takeEnvelope()
    assertEquals(await Deno.readFile(pikgPath), originalPikg)
    await Deno.writeFile(payloadPath, originalPayload)

    assertEquals(await app.run(['--non-interactive', 'pikg', 'clean']), 4)
    envelope = io.takeEnvelope()
    assertEquals(envelope.error.code, 'CONFIRMATION_REQUIRED')
    assertEquals(await app.run(['--non-interactive', '--yes', 'pikg', 'clean']), 0)
    envelope = io.takeEnvelope()
    assertEquals(envelope.data.removed, true)
    await assertRejects(() => Deno.stat(join(project, 'dapp_dist')))
    assertEquals(authenticationCreated, false)
  } finally {
    await Deno.remove(root, { recursive: true })
  }
})

Deno.test('pikg Docker workflow pins the image ID and never uses a session', async () => {
  const root = await Deno.makeTempDir()
  try {
    const project = join(root, 'docker-demo')
    await Deno.mkdir(project)
    const docker = new FakeDocker()
    const io = new CaptureStdio()
    const app = new BuckyOSToolApplication({
      cwd: project,
      homeDir: join(root, 'home'),
      environment: { HOME: join(root, 'home') },
      stdio: io,
      pikg: { docker, now: () => 1_800_000_000 },
      createAuthentication: () => {
        throw new Error('PIKG must not create authentication')
      },
    })
    assertEquals(
      await app.run([
        '--non-interactive',
        'pikg',
        'init',
        '.',
        '--owner',
        'did:bns:root',
        '--kind',
        'docker',
        '--source',
        'local/demo:0.1.0-amd64',
      ]),
      0,
    )
    io.takeEnvelope()
    assertEquals(docker.saveReferences, [])
    const buildExit = await app.run(['pikg', 'build'])
    assertEquals(buildExit, 0, io.stdoutText)
    io.takeEnvelope()
    assertEquals(docker.saveReferences, [docker.info.id])
    assert(docker.inspectReferences.length >= 3)
    const packExit = await app.run(['pikg', 'pack'])
    assertEquals(packExit, 0, io.stdoutText)
    const packed = io.takeEnvelope()
    assertEquals(await app.run(['pikg', 'info', packed.data.pikg_path]), 0)
    const inspected = io.takeEnvelope()
    assertEquals(inspected.data.subpackages[0].docker_image_digest, docker.info.id)
    assertEquals(inspected.data.subpackages[0].selector.arch, 'x86_64')
  } finally {
    await Deno.remove(root, { recursive: true })
  }
})

Deno.test('pikg clean rejects protected, symlinked, and unmanaged targets', async () => {
  const root = await Deno.makeTempDir()
  try {
    const project = join(root, 'clean-demo')
    const source = join(project, 'web', 'dist')
    await Deno.mkdir(source, { recursive: true })
    await Deno.writeTextFile(join(source, 'index.html'), 'safe')
    const io = new CaptureStdio()
    const app = new BuckyOSToolApplication({
      cwd: project,
      homeDir: join(root, 'home'),
      environment: { HOME: join(root, 'home') },
      stdio: io,
      pikg: { now: () => 1_800_000_000 },
      createAuthentication: () => {
        throw new Error('PIKG must not create authentication')
      },
    })
    assertEquals(
      await app.run([
        '--non-interactive',
        'pikg',
        'init',
        '.',
        '--owner',
        'did:bns:root',
        '--kind',
        'static-web',
        '--source',
        './web/dist',
      ]),
      0,
    )
    io.takeEnvelope()
    assertEquals(await app.run(['pikg', 'build']), 0)
    io.takeEnvelope()

    const metaPath = join(project, 'dapp_meta', 'pikg.json')
    const original = JSON.parse(await Deno.readTextFile(metaPath))
    const rejectTarget = async (outputDir: string) => {
      await Deno.writeTextFile(
        metaPath,
        `${JSON.stringify({ ...original, output_dir: outputDir }, null, 2)}\n`,
      )
      assertEquals(await app.run(['--non-interactive', '--yes', 'pikg', 'clean']), 6)
      assertEquals(io.takeEnvelope().error.code, 'UNSAFE_CLEAN_TARGET')
    }

    await rejectTarget('..')
    await rejectTarget('../web/dist')
    await rejectTarget(parsePath(project).root)

    const unmanaged = join(project, 'user-output')
    await Deno.mkdir(unmanaged)
    await Deno.writeTextFile(join(unmanaged, 'keep.txt'), 'keep')
    await rejectTarget('../user-output')
    assertEquals(await Deno.readTextFile(join(unmanaged, 'keep.txt')), 'keep')

    if (Deno.build.os !== 'windows') {
      const linked = join(project, 'linked-output')
      await Deno.symlink(join(project, 'dapp_dist'), linked)
      await rejectTarget('../linked-output')
      assert((await Deno.lstat(linked)).isSymlink)
    }
  } finally {
    await Deno.remove(root, { recursive: true })
  }
})

Deno.test('deterministic tar.gz rejects source symlinks that escape the input root', async () => {
  const root = await Deno.makeTempDir()
  try {
    const source = join(root, 'source')
    await Deno.mkdir(join(source, 'nested'), { recursive: true })
    await Deno.writeTextFile(join(source, 'nested', 'b.txt'), 'b')
    await Deno.writeTextFile(join(source, 'a.txt'), 'a')
    const first = join(root, 'first.tar.gz')
    const second = join(root, 'second.tar.gz')
    await createDeterministicTarGz(source, first)
    await createDeterministicTarGz(source, second)
    assertEquals((await digestFile(first)).sha256, (await digestFile(second)).sha256)

    if (Deno.build.os !== 'windows') {
      const outside = join(root, 'outside.txt')
      await Deno.writeTextFile(outside, 'outside')
      await Deno.symlink(outside, join(source, 'escape'))
      await assertRejects(
        () => createDeterministicTarGz(source, join(root, 'unsafe.tar.gz')),
        'INVALID_SOURCE',
      )
    }
  } finally {
    await Deno.remove(root, { recursive: true })
  }
})

Deno.test('pikg command schemas are local and expose all five verbs', () => {
  const app = new BuckyOSToolApplication({ environment: {} })
  const module = app.registry.modules().find((candidate) => candidate.name === 'pikg')
  assert(module)
  assertEquals(module.commands.map((command) => command.verb), [
    'init',
    'build',
    'pack',
    'info',
    'clean',
  ])
  for (const command of module.commands) {
    assertEquals(command.requiresSession, false)
    assertEquals(command.execution, 'local')
    assertEquals(command.networkAccess, false)
  }
})
