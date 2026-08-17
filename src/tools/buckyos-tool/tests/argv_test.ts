import { parseCommandArgs, parseInvocation, parseShellLine } from '../core/argv.ts'
import { createRegistry } from '../core/app.ts'
import { assertEquals, assertRejects } from './test_helpers.ts'

Deno.test('two-stage parser keeps global options before the command', () => {
  const parsed = parseInvocation([
    '--profile',
    'production',
    '--output=json',
    'system',
    'status',
  ])
  assertEquals(parsed.global.profile, 'production')
  assertEquals(parsed.global.output, 'json')
  assertEquals(parsed.module, 'system')
  assertEquals(parsed.verb, 'status')
})

Deno.test('argv and input JSON produce the same typed command input', () => {
  const command = createRegistry().get('config', 'set')
  const fromArgv = parseCommandArgs(command, [
    'zone',
    '--value',
    'corp.example.com',
    '--profile-name',
    'production',
  ], undefined)
  const fromJson = parseCommandArgs(command, [], {
    key: 'zone',
    value: 'corp.example.com',
    profile_name: 'production',
  })
  assertEquals(fromArgv.input, fromJson.input)
})

Deno.test('input JSON and argv conflict instead of overriding', async () => {
  const command = createRegistry().get('config', 'set')
  await assertRejects(
    () =>
      parseCommandArgs(command, ['zone', '--value', 'b.example'], {
        key: 'zone',
        value: 'a.example',
      }),
    'ARGUMENT_CONFLICT',
  )
})

Deno.test('REPL parser supports quotes, escapes, and empty arguments', () => {
  assertEquals(
    parseShellLine(
      `config set identity --value "did:web:ops example" 'empty value' escaped\\ value ""`,
    ),
    [
      'config',
      'set',
      'identity',
      '--value',
      'did:web:ops example',
      'empty value',
      'escaped value',
      '',
    ],
  )
})

Deno.test('REPL rejects session-scoped options on a command line', async () => {
  const command = createRegistry().get('system', 'status')
  await assertRejects(
    () => parseCommandArgs(command, ['--zone', 'other.example'], undefined, true),
    'SESSION_OPTION_FROZEN',
  )
})
