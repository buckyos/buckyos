import assert from 'node:assert/strict'
import test from 'node:test'
import { groupSoftwareProjects, type SoftwarePackage } from '../../src/app/settings/data/open-source-projects.ts'

function software(name: string, fields: Partial<SoftwarePackage> = {}): SoftwarePackage {
  return {
    id: `${name}-${fields.version ?? '1.0.0'}-${fields.repository ?? ''}`,
    name,
    version: '1.0.0',
    ecosystem: 'cargo',
    license: 'MIT',
    description: '',
    homepage: '',
    repository: '',
    scopes: ['rust'],
    licenseFileCount: 1,
    ...fields,
  }
}

test('monorepo components share one project without changing their versions or licenses', () => {
  const packages = [
    software('@emotion/react', { ecosystem: 'npm', repository: 'git+https://github.com/emotion-js/emotion.git#main', license: 'MIT', version: '11.14.0' }),
    software('@emotion/styled', { ecosystem: 'npm', repository: 'https://GitHub.com/Emotion-JS/Emotion/tree/main/packages/styled?plain=1', license: 'Apache-2.0', version: '11.14.1' }),
  ]
  const before = JSON.stringify(packages)
  const projects = groupSoftwareProjects(packages)
  assert.equal(projects.length, 1)
  assert.equal(projects[0].id, 'repository:github.com/emotion-js/emotion')
  assert.equal(projects[0].name.toLowerCase(), 'emotion')
  assert.deepEqual(projects[0].packages, packages)
  assert.equal(projects[0].packages[0], packages[0])
  assert.equal(JSON.stringify(packages), before)
  assert.match(projects[0].searchText, /@emotion\/styled 11\.14\.1 apache-2\.0 npm/)
})

test('package identity connects versions across a moved repository and its companion packages', () => {
  const packages = [
    software('react', { ecosystem: 'npm', version: '18.3.1', repository: 'https://github.com/facebook/react' }),
    software('react', { ecosystem: 'npm', version: '19.3.1', repository: 'https://github.com/react/react' }),
    software('react-dom', { ecosystem: 'npm', version: '19.3.1', repository: 'https://github.com/react/react' }),
    software('react', { ecosystem: 'npm', version: '17.0.0' }),
  ]
  const projects = groupSoftwareProjects(packages)
  assert.equal(projects.length, 1)
  assert.equal(projects[0].name, 'react')
  assert.equal(projects[0].packages.length, 4)
  assert.deepEqual(groupSoftwareProjects([...packages].reverse()), projects)
})

test('the requested proc-macro series is grouped while preserving each component source', () => {
  const packages = [
    software('proc-macro-crate', { repository: 'https://github.com/bkchr/proc-macro-crate' }),
    software('proc-macro-error-attr3', { repository: 'https://github.com/gamma0987/proc-macro-error3' }),
    software('proc-macro-error3', { repository: 'https://github.com/gamma0987/proc-macro-error3' }),
    software('proc-macro2', { repository: 'https://github.com/dtolnay/proc-macro2' }),
    software('proc-macro2-diagnostics', { repository: 'https://github.com/SergioBenitez/proc-macro2-diagnostics' }),
    software('proc-macro2', { ecosystem: 'npm', id: 'npm-proc-macro2' }),
  ]
  const projects = groupSoftwareProjects(packages)
  assert.equal(projects.length, 2)
  const family = projects.find((project) => project.id === 'family:cargo:proc-macro')!
  assert.equal(family.name, 'proc-macro')
  assert.equal(family.packages.length, 5)
  assert.equal(new Set(family.packages.map((item) => item.repository)).size, 4)
})

test('similar names share a series while retaining the independent project count', () => {
  const packages = [
    software('react', { ecosystem: 'npm', repository: 'https://github.com/facebook/react' }),
    software('react-hook-form', { ecosystem: 'npm', repository: 'https://github.com/react-hook-form/react-hook-form', license: 'Apache-2.0' }),
    software('serde', { repository: 'https://github.com/serde-rs/serde' }),
    software('serde_yaml', { repository: 'https://github.com/dtolnay/serde-yaml' }),
    software('serde_json', { repository: 'https://github.com/serde-rs/json' }),
    software('@lezer/json', { ecosystem: 'npm', repository: 'https://github.com/lezer-parser/json' }),
  ]
  const projects = groupSoftwareProjects(packages)
  assert.equal(projects.length, 3)
  const react = projects.find((project) => project.name === 'react')!
  assert.equal(react.isFamily, true)
  assert.equal(react.projectCount, 2)
  assert.equal(react.packages[1].license, 'Apache-2.0')
  assert.equal(projects.find((project) => project.name === 'serde')!.projectCount, 3)
  assert.equal(projects.find((project) => project.name === 'json')!.isFamily, false)
})

test('GitLab subgroup paths are retained and other forge source paths normalize to repository roots', () => {
  const packages = [
    software('one', { repository: 'https://gitlab.com/org/subgroup/tool.git' }),
    software('two', { repository: 'https://gitlab.com/org/subgroup/tool/-/tree/main/crate' }),
    software('three', { repository: 'https://gitlab.com/org/other/tool' }),
    software('four', { repository: 'https://codeberg.org/owner/project/src/branch/main/crate' }),
    software('five', { repository: 'https://codeberg.org/owner/project.git/' }),
    software('six', { repository: 'https://bitbucket.org/owner/library/src/main/package' }),
    software('seven', { repository: 'https://bitbucket.org/owner/library' }),
  ]
  assert.equal(groupSoftwareProjects(packages).length, 4)
})

test('known forge homepages fill missing repositories, while generic homepages and registries do not', () => {
  const packages = [
    software('one', { homepage: 'https://github.com/owner/project/tree/main' }),
    software('two', { repository: 'https://github.com/owner/project' }),
    software('three', { homepage: 'https://crates.io/crates' }),
    software('four', { homepage: 'https://crates.io/crates' }),
    software('five', { homepage: 'https://www.npmjs.com' }),
    software('six', { homepage: 'https://www.npmjs.com' }),
    software('seven', { homepage: 'https://example.org' }),
    software('eight', { homepage: 'https://example.org' }),
    software('nine', { homepage: 'https://github.com/owner/project', repository: 'https://elsewhere.example/one/two' }),
  ]
  assert.equal(groupSoftwareProjects(packages).length, 8)
})

test('unknown repository hosts retain their full paths and source-less packages stay separated by ecosystem', () => {
  const packages = [
    software('one', { repository: 'https://git.example/org/repository/subproject' }),
    software('two', { repository: 'https://git.example/org/repository.git' }),
    software('three', { repository: 'https://git.example/org/repository/#readme' }),
    software('same-name', { ecosystem: 'cargo', version: '1.0.0' }),
    software('same-name', { ecosystem: 'cargo', version: '2.0.0' }),
    software('same-name', { ecosystem: 'npm', id: 'npm-same-name' }),
  ]
  assert.equal(groupSoftwareProjects(packages).length, 4)
  assert.deepEqual(groupSoftwareProjects([]), [])
})


test('scopes and repository anchors prevent names from bridging independent families', () => {
  const packages = [
    software('react', { ecosystem: 'npm', repository: 'https://github.com/facebook/react' }),
    software('react-hook-form', { ecosystem: 'npm', repository: 'https://github.com/react-hook-form/react-hook-form' }),
    software('@emotion/react', { ecosystem: 'npm', repository: 'https://github.com/emotion-js/emotion' }),
    software('@emotion/styled', { ecosystem: 'npm', repository: 'https://github.com/emotion-js/emotion' }),
    software('serde', { repository: 'https://github.com/serde-rs/serde' }),
    software('serde_yaml', { repository: 'https://github.com/dtolnay/serde-yaml' }),
    software('toml', { repository: 'https://github.com/toml-rs/toml' }),
    software('serde_spanned', { repository: 'https://github.com/toml-rs/toml' }),
    software('toml_edit', { repository: 'https://github.com/toml-rs/toml' }),
  ]
  const projects = groupSoftwareProjects(packages)
  assert.equal(projects.length, 4)
  assert.deepEqual(projects.find((project) => project.name === 'react')!.packages.map((item) => item.name), ['react', 'react-hook-form'])
  assert.deepEqual(projects.find((project) => project.name === 'emotion')!.packages.map((item) => item.name), ['@emotion/react', '@emotion/styled'])
  assert.deepEqual(projects.find((project) => project.name === 'serde')!.packages.map((item) => item.name), ['serde', 'serde_yaml'])
  assert.equal(projects.find((project) => project.name === 'toml')!.packages.length, 3)
})

test('npm scopes combine independent repositories without moving DefinitelyTyped into a generic types group', () => {
  const packages = [
    software('@codemirror/view', { ecosystem: 'npm', repository: 'https://github.com/codemirror/view' }),
    software('@codemirror/lang-rust', { ecosystem: 'npm', repository: 'https://github.com/codemirror/lang-rust' }),
    software('@types/react', { ecosystem: 'npm', repository: 'https://github.com/DefinitelyTyped/DefinitelyTyped' }),
    software('@types/node', { ecosystem: 'npm', repository: 'https://github.com/DefinitelyTyped/DefinitelyTyped' }),
  ]
  const projects = groupSoftwareProjects(packages)
  assert.equal(projects.length, 2)
  assert.equal(projects.find((project) => project.name === 'codemirror')!.projectCount, 2)
  assert.equal(projects.find((project) => project.name === 'DefinitelyTyped')!.isFamily, false)
})

test('generic prefixes and unrelated ecosystems do not create catch-all families', () => {
  const packages = ['lib-alpha', 'lib-beta', 'node-first', 'node-second', 'rust-first', 'rust-second'].map((name) => software(name))
  packages.push(software('serde', { ecosystem: 'npm' }), software('serde', { ecosystem: 'cargo' }))
  assert.equal(groupSoftwareProjects(packages).length, packages.length)
})

test('native counterparts and numeric variants merge only with a concrete matching family', () => {
  const packages = [
    software('freetype', { ecosystem: 'conda' }),
    software('libfreetype', { ecosystem: 'conda' }),
    software('libfreetype6', { ecosystem: 'conda' }),
    software('libreoffice', { ecosystem: 'conda' }),
    software('libcurl', { ecosystem: 'conda' }),
    software('sdl2', { ecosystem: 'conda' }),
    software('sdl3', { ecosystem: 'conda' }),
    software('python', { ecosystem: 'conda' }),
    software('python', { ecosystem: 'runtime', id: 'runtime-python' }),
  ]
  const projects = groupSoftwareProjects(packages)
  assert.equal(projects.find((project) => project.name === 'freetype')!.packages.length, 3)
  assert.equal(projects.find((project) => project.name === 'libreoffice')!.isFamily, false)
  assert.equal(projects.find((project) => project.name === 'sdl')!.packages.length, 2)
  assert.equal(projects.find((project) => project.name === 'python')!.packages.length, 2)
})

test('series grouping preserves every original object and remains deterministic when input order changes', () => {
  const packages = [
    software('serde', { repository: 'https://github.com/serde-rs/serde' }),
    software('serde_yaml', { repository: 'https://github.com/dtolnay/serde-yaml', license: 'Apache-2.0' }),
    software('serde_yaml', { version: '0.8.0', repository: 'https://github.com/dtolnay/serde-yaml', license: null }),
    software('react', { ecosystem: 'npm', repository: 'https://github.com/facebook/react' }),
    software('react-hook-form', { ecosystem: 'npm', repository: 'https://github.com/react-hook-form/react-hook-form' }),
  ]
  const before = JSON.stringify(packages)
  const groups = groupSoftwareProjects(packages)
  const retained = groups.flatMap((group) => group.packages)
  assert.equal(retained.length, packages.length)
  assert.equal(new Set(retained).size, packages.length)
  for (const item of packages) assert(retained.includes(item))
  assert.equal(JSON.stringify(packages), before)
  assert.deepEqual(groupSoftwareProjects([...packages].reverse()), groups)
})
