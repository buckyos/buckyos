import type { UserInfo } from '../../src/api/user_mgr.ts'
import {
  changeTypeTargets,
  defaultContactProjectionRules,
  parseUserGroupRef,
  userTypeCanCreateAgents,
} from '../../src/api/user_type.ts'
import {
  appListTargetUserIds,
  toLocalUserEntity,
  toUserGroupsInfo,
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

Deno.test('account types map one to one; guest is no longer a type', () => {
  const now = '2026-10-21T00:00:00Z'
  const entity = (user_type: string, extra: Partial<UserInfo> = {}) =>
    toLocalUserEntity({ user_id: 'u', show_name: 'U', user_type, state: 'active', ...extra }, [], now)

  assertEquals(entity('admin').role, 'admin', 'admin')
  assertEquals(entity('limited').role, 'limited', 'limited')
  assertEquals(entity('guest').role, 'unknown', 'guest is unknown')
  assertEquals(entity('limited').passwordChangeAllowed, false, 'limited password default')
  assertEquals(entity('limited').contactEditAllowed, false, 'limited contact default')
  const opened = entity('limited', {
    allow_contact_edit: true,
    can_edit_contacts: true,
    groups: ['customer', 'customer', 'beta'],
  })
  assertEquals(opened.contactEditAllowed, true, 'override opens contacts')
  assertEquals(opened.contactEditOverride, true, 'override kept')
  assertEquals(opened.passwordChangeOverride, null, 'no password override')
  assertEquals(opened.groups, ['beta', 'customer'], 'groups sorted, unique')

  assertEquals(changeTypeTargets('user'), ['admin', 'limited'], 'user targets')
  assertEquals(changeTypeTargets('root'), [], 'root cannot change')
  assertEquals(userTypeCanCreateAgents('limited'), false, 'limited cannot create agents')
  assertEquals(userTypeCanCreateAgents('user'), true, 'user can create agents')
})

Deno.test('group references and the default projection rules', () => {
  assertEquals(parseUserGroupRef('group:customer'), { kind: 'group', groupId: 'customer' }, 'system group')
  assertEquals(parseUserGroupRef('type:limited'), { kind: 'type', userType: 'limited' }, 'type group')
  assertEquals(parseUserGroupRef('type:root'), null, 'root is no group')
  assertEquals(parseUserGroupRef('group:Bad'), null, 'bad id')
  assertEquals(defaultContactProjectionRules().length, 4, 'admins and users see each other')
  const info = toUserGroupsInfo({
    groups: [
      { group_id: 'service', name: 'Service', updated_by: 'alice', created_at: 1, updated_at: 1, members: ['bob'] },
      { group_id: 'customer', name: '', updated_by: 'alice', created_at: 1, updated_at: 1, members: ['dave', 'carol'] },
    ],
    projections: { rules: [{ source: 'group:customer', target: 'group:service' }], is_default: false },
  })
  assertEquals(info.groups.map((group) => group.name), ['customer', 'Service'], 'names fall back to ids')
  assertEquals(info.groups[0].members, ['carol', 'dave'], 'members sorted')
  assertEquals(info.projectionsAreDefault, false, 'custom rules')
})
