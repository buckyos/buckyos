import { copyFile, mkdir, rm, writeFile } from 'node:fs/promises'
import os from 'node:os'
import path from 'node:path'

import {
  buildPikgProject,
  PIKG_SAMPLES_ROOT,
  configurePikgSample,
  copyPikgSample,
  dockerTarget,
  runCommand,
} from './pikg_sample_builder.mjs'

const DOCKER_BASE_IMAGE =
  process.env.BUCKYOS_TEST_DOCKER_BASE_IMAGE ?? 'busybox:1.36.1'
const OUTPUT_ROOT = path.resolve(
  process.env.BUCKYOS_PIKG_OUTPUT_DIR ??
    path.join(os.tmpdir(), 'buckyos-pikg-samples'),
)

const outputRelativeToSamples = path.relative(PIKG_SAMPLES_ROOT, OUTPUT_ROOT)
if (
  outputRelativeToSamples === '' ||
  (!outputRelativeToSamples.startsWith('..') &&
    !path.isAbsolute(outputRelativeToSamples))
) {
  throw new Error('BUCKYOS_PIKG_OUTPUT_DIR must be outside pikg_samples')
}

async function buildSample(name) {
  const { tempRoot, projectDir } = await copyPikgSample(name)
  try {
    const result = await buildPikgProject(projectDir)
    const outputPath = path.join(OUTPUT_ROOT, `${name}.pikg`)
    await copyFile(result.pack.pikg_path, outputPath)
    return {
      name,
      file: outputPath,
      bytes: result.pack.size,
      sha256: result.pack.pikg_digest,
      app_did: result.appDoc.did,
      app_doc_id: result.pack.app_doc_object_id,
      package_keys: result.info.subpackages.map((item) => item.key),
    }
  } finally {
    await rm(tempRoot, { recursive: true, force: true })
  }
}

async function main() {
  await mkdir(OUTPUT_ROOT, { recursive: true })
  const samples = []
  for (const name of ['static-web', 'script-host', 'agent']) {
    samples.push(await buildSample(name))
  }

  const target = dockerTarget()
  const imageName =
    `local/pikg-docker-sample:${Date.now().toString(36)}-${target.tagArch}`
  const { tempRoot, projectDir } = await copyPikgSample('docker')
  try {
    await runCommand('docker', [
      'build',
      '--build-arg',
      `BASE_IMAGE=${DOCKER_BASE_IMAGE}`,
      '-t',
      imageName,
      path.join(projectDir, 'image'),
    ])
    await configurePikgSample(projectDir, {
      appId: 'pikg-docker',
      version: '0.1.0',
      ownerDid: 'did:bns:root',
      dockerImage: imageName,
    })
    const result = await buildPikgProject(projectDir)
    const outputPath = path.join(OUTPUT_ROOT, 'docker.pikg')
    await copyFile(result.pack.pikg_path, outputPath)
    samples.push({
      name: 'docker',
      file: outputPath,
      bytes: result.pack.size,
      sha256: result.pack.pikg_digest,
      app_did: result.appDoc.did,
      app_doc_id: result.pack.app_doc_object_id,
      package_keys: result.info.subpackages.map((item) => item.key),
    })
  } finally {
    await runCommand('docker', ['image', 'rm', '-f', imageName]).catch(() => {})
    await rm(tempRoot, { recursive: true, force: true })
  }

  const manifest = {
    schema: 'buckyos.pikg.samples.v2',
    output_dir: OUTPUT_ROOT,
    samples,
  }
  await writeFile(
    path.join(OUTPUT_ROOT, 'manifest.json'),
    `${JSON.stringify(manifest, null, 2)}\n`,
  )
  process.stdout.write(`${JSON.stringify(manifest, null, 2)}\n`)
}

await main()
