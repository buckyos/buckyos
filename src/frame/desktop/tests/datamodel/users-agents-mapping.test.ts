import type { UserInfo } from '../../src/api/user_mgr.ts'
import {
  appListTargetUserIds,
  toVisibleLocalUserEntities,
} from '../../src/app/users-agents/datamodel/transforms.ts'

function assertEquals<T>(actual: T, expected: T, message: string) {
  if (JSON.stringify(actual) !== JSON.stringify(expected)) {
    throw new Error(`${message}: expected ${JSON.stringify(expected)}, got ${JSON.stringify(actual)}`)
  }
}

Deno.test('visible local users exclude the root tier, self, and duplicate ids', () => {
  const users: UserInfo[] = [
    {
      user_id: 'devtest',
      show_name: 'Liu Zhicong',
      user_type: 'admin',
      state: 'active',
      is_local: true,
    },
    {
      user_id: 'localdemo',
      show_name: 'Local Demo User',
      user_type: 'user',
      state: 'active',
      is_local: true,
    },
    {
      user_id: 'lucy',
      show_name: 'Lucy',
      user_type: 'user',
      state: 'active',
      is_local: true,
    },
    {
      user_id: 'devtest',
      show_name: 'devtest',
      user_type: 'root',
      state: 'active',
      is_local: true,
    },
    {
      user_id: 'localdemo',
      show_name: 'Duplicate Local Demo',
      user_type: 'user',
      state: 'active',
      is_local: true,
    },
  ]

  const entities = toVisibleLocalUserEntities(
    users,
    new Map(),
    'lucy',
    '2026-08-11T00:00:00Z',
  )

  assertEquals(entities.map((entity) => entity.id), ['devtest', 'localdemo'], 'visible ids')
  assertEquals(entities.map((entity) => entity.displayName), ['Liu Zhicong', 'Local Demo User'], 'visible names')
})

Deno.test('ordinary users only query their own app list', () => {
  const users = [
    { user_id: 'devtest' },
    { user_id: 'alice' },
    { user_id: 'bob' },
  ]

  assertEquals(
    appListTargetUserIds('alice', 'user', users),
    ['alice'],
    'ordinary app-list scope',
  )
  assertEquals(
    appListTargetUserIds('alice', 'admin', users),
    ['alice', 'devtest', 'bob'],
    'admin app-list scope',
  )
})
