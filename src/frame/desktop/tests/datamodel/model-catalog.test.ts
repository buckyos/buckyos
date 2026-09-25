import { defaultModelFilters, filterModelCatalog, modelCard, type ModelCatalog } from '../../src/app/ai-center/datamodel/model-catalog.ts'

function assert(value: unknown, message: string): asserts value {
  if (!value) throw new Error(message)
}

const catalog: ModelCatalog = {
  revision: 1,
  vendors: [{
    id: 'future-vendor', revision: 1,
    models: [
      { id: 'cloud', metadata: { api_types: ['image.txt2img'] }, providers: [{ id: 'remote', local: false, exact_models: ['aliased@remote'] }] },
      { id: 'local', metadata: { local_deployable: true }, providers: [{ id: 'engine', local: true, exact_models: ['aliased@engine', 'aliased:high@engine'] }] },
      { id: 'unconnected', metadata: { local_deployable: true }, providers: [] },
    ],
    specs: [{ id: 'empty-spec', path: 'llm.empty-spec', direct_only: true, members: [] }],
  }],
}

Deno.test('status derives from provider presence separately from local deployment metadata', () => {
  const models = catalog.vendors[0].models.map((model) => modelCard(model, 'future-vendor'))
  assert(models[0].available && !models[0].local && !models[0].deployable, 'remote presence must not imply local support')
  assert(models[1].available && models[1].local && models[1].deployable, 'local inventory lights both indicators')
  assert(!models[2].available && !models[2].local && models[2].deployable, 'deployable is independent of availability')
})

Deno.test('filters intersect, search is case insensitive and empty specs survive without status filters', () => {
  const query = (filters: Partial<typeof defaultModelFilters>) => filterModelCatalog(catalog, { ...defaultModelFilters, ...filters })
  assert(query({ query: '  FUTURE-VENDOR  ' })[0].models.length === 3, 'unknown vendors stay searchable')
  assert(query({ query: 'image' })[0].models[0].id === 'cloud', 'capability search')
  assert(query({ available: true })[0].models.length === 2, 'available inventory')
  assert(query({ deployable: true })[0].models.length === 2, 'deployment metadata')
  assert(query({ available: true, deployable: true, local: true })[0].models[0].id === 'local', 'combined filters')
  assert(query({ query: 'empty-spec' })[0].specs.length === 1, 'empty specification search')
  assert(query({ available: true })[0].specs.length === 0, 'status filters hide empty specifications')
  assert(query({ query: 'missing' }).length === 0, 'unmatched search')
})
