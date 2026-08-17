import type { CommandModule } from '../core/command.ts'
import type { CommandRegistry } from '../core/registry.ts'
import {
  configValueError,
  effectiveConfigView,
  type ProfileConfig,
  type ToolConfig,
  validateProfileName,
} from '../core/config.ts'
import { UsageError } from '../core/errors.ts'

const EMPTY_INPUT = {
  type: 'object' as const,
  properties: {},
  additionalProperties: false,
}
const OBJECT_OUTPUT = { type: 'object' as const, additionalProperties: true }

export function createCoreModules(registry: CommandRegistry): CommandModule[] {
  return [createCommandModule(registry), createConfigModule(), createCompletionModule(registry)]
}

function createCommandModule(registry: CommandRegistry): CommandModule {
  return {
    name: 'command',
    summary: 'Discover the machine-readable command registry',
    commands: [
      {
        verb: 'list',
        summary: 'List registered modules and commands',
        inputSchema: EMPTY_INPUT,
        outputSchema: OBJECT_OUTPUT,
        resultSchemaVersion: 1,
        access: { mode: 'fixed', level: 'read' },
        asyncMode: 'sync',
        requiresSession: false,
        examples: ['buckyos command list'],
        handler: () =>
          Promise.resolve({
            modules: registry.modules().map((module) => ({
              name: module.name,
              summary: module.summary,
              commands: module.commands.map((command) => ({
                verb: command.verb,
                summary: command.summary,
              })),
            })),
          }),
      },
      {
        verb: 'describe',
        summary: 'Describe one command and its complete schemas',
        positionals: [
          { name: 'target_module', description: 'Module name' },
          { name: 'target_verb', description: 'Verb name' },
        ],
        inputSchema: {
          type: 'object',
          properties: {
            target_module: { type: 'string', minLength: 1 },
            target_verb: { type: 'string', minLength: 1 },
          },
          required: ['target_module', 'target_verb'],
          additionalProperties: false,
        },
        outputSchema: OBJECT_OUTPUT,
        resultSchemaVersion: 1,
        access: { mode: 'fixed', level: 'read' },
        asyncMode: 'sync',
        requiresSession: false,
        examples: ['buckyos command describe system status'],
        handler: (_ctx, input) =>
          Promise.resolve(
            registry.describe(String(input.target_module), String(input.target_verb)),
          ),
      },
    ],
  }
}

function createConfigModule(): CommandModule {
  return {
    name: 'config',
    summary: 'Manage local non-secret tool configuration',
    commands: [
      {
        verb: 'list',
        summary: 'List the global configuration and available profiles',
        inputSchema: EMPTY_INPUT,
        outputSchema: OBJECT_OUTPUT,
        resultSchemaVersion: 1,
        access: { mode: 'fixed', level: 'read' },
        asyncMode: 'sync',
        requiresSession: false,
        examples: ['buckyos config list'],
        handler: async (ctx) => ({
          config: await ctx.configStore.readConfig(),
          profiles: await ctx.configStore.listProfiles(),
        }),
      },
      {
        verb: 'get',
        summary: 'Read a global or profile configuration value',
        positionals: [{ name: 'key', description: 'Configuration key', required: false }],
        options: [
          {
            name: 'profile-name',
            property: 'profile_name',
            description: 'Read a profile',
            type: 'string',
          },
        ],
        inputSchema: {
          type: 'object',
          properties: {
            key: { type: 'string', minLength: 1 },
            profile_name: { type: 'string', minLength: 1 },
          },
          additionalProperties: false,
        },
        outputSchema: OBJECT_OUTPUT,
        resultSchemaVersion: 1,
        access: { mode: 'fixed', level: 'read' },
        asyncMode: 'sync',
        requiresSession: false,
        examples: [
          'buckyos config get output',
          'buckyos config get zone --profile-name production',
        ],
        handler: async (ctx, input) => {
          const profileName = optionalString(input.profile_name)
          const value = profileName
            ? await ctx.configStore.readProfile(profileName)
            : await ctx.configStore.readConfig()
          if (!value) throw new UsageError('PROFILE_NOT_FOUND', `profile not found: ${profileName}`)
          const key = optionalString(input.key)
          if (!key) return { value }
          if (!Object.hasOwn(value, key)) {
            throw new UsageError('CONFIG_KEY_NOT_FOUND', `configuration key not found: ${key}`)
          }
          return { key, value: (value as unknown as Record<string, unknown>)[key] }
        },
      },
      {
        verb: 'set',
        summary: 'Atomically set a global or profile configuration value',
        positionals: [{ name: 'key', description: 'Configuration key' }],
        options: [
          { name: 'value', description: 'Configuration value', type: 'string', required: true },
          {
            name: 'profile-name',
            property: 'profile_name',
            description: 'Write a profile',
            type: 'string',
          },
        ],
        inputSchema: {
          type: 'object',
          properties: {
            key: { type: 'string', minLength: 1 },
            value: { type: 'string' },
            profile_name: { type: 'string', minLength: 1 },
          },
          required: ['key', 'value'],
          additionalProperties: false,
        },
        outputSchema: OBJECT_OUTPUT,
        resultSchemaVersion: 1,
        access: { mode: 'fixed', level: 'write' },
        asyncMode: 'sync',
        requiresSession: false,
        examples: [
          'buckyos config set output --value json',
          'buckyos config set zone --value corp.example.com --profile-name production',
        ],
        handler: async (ctx, input) => {
          const key = String(input.key)
          const rawValue = String(input.value)
          const profileName = optionalString(input.profile_name)
          if (profileName) {
            validateProfileName(profileName)
            const profile = await ctx.configStore.readProfile(profileName) ?? { schema_version: 1 }
            setProfileValue(profile, key, rawValue)
            await ctx.configStore.writeProfile(profileName, profile)
            return {
              profile: profileName,
              key,
              value: (profile as unknown as Record<string, unknown>)[key],
            }
          }
          const config = await ctx.configStore.readConfig()
          setGlobalValue(config, key, rawValue)
          await ctx.configStore.writeConfig(config)
          return { profile: null, key, value: (config as unknown as Record<string, unknown>)[key] }
        },
      },
      {
        verb: 'use',
        summary: 'Select the default profile',
        positionals: [{ name: 'profile_name', description: 'Profile name' }],
        inputSchema: {
          type: 'object',
          properties: { profile_name: { type: 'string', minLength: 1 } },
          required: ['profile_name'],
          additionalProperties: false,
        },
        outputSchema: OBJECT_OUTPUT,
        resultSchemaVersion: 1,
        access: { mode: 'fixed', level: 'write' },
        asyncMode: 'sync',
        requiresSession: false,
        examples: ['buckyos config use production'],
        handler: async (ctx, input) => {
          const profileName = String(input.profile_name)
          if (!await ctx.configStore.readProfile(profileName)) {
            throw new UsageError('PROFILE_NOT_FOUND', `profile not found: ${profileName}`)
          }
          const config = await ctx.configStore.readConfig()
          config.default_profile = profileName
          await ctx.configStore.writeConfig(config)
          return { default_profile: profileName }
        },
      },
      {
        verb: 'check',
        summary: 'Validate configuration and show a redacted effective view',
        options: [
          {
            name: 'effective',
            description: 'Include the effective merged configuration',
            type: 'boolean',
          },
        ],
        inputSchema: {
          type: 'object',
          properties: { effective: { type: 'boolean' } },
          additionalProperties: false,
        },
        outputSchema: OBJECT_OUTPUT,
        resultSchemaVersion: 1,
        access: { mode: 'fixed', level: 'read' },
        asyncMode: 'sync',
        requiresSession: false,
        examples: ['buckyos --profile production config check --effective'],
        handler: async (ctx, input) => ({
          valid: true,
          profile_count: (await ctx.configStore.listProfiles()).length,
          ...(input.effective ? { effective: effectiveConfigView(ctx.config) } : {}),
        }),
      },
    ],
  }
}

function createCompletionModule(registry: CommandRegistry): CommandModule {
  return {
    name: 'completion',
    summary: 'Generate shell completion scripts from the command registry',
    commands: [
      {
        verb: 'generate',
        summary: 'Generate completion for bash, zsh, or fish',
        options: [
          {
            name: 'shell',
            description: 'Target shell',
            type: 'string',
            required: true,
            enum: ['bash', 'zsh', 'fish'],
          },
        ],
        inputSchema: {
          type: 'object',
          properties: { shell: { type: 'string', enum: ['bash', 'zsh', 'fish'] } },
          required: ['shell'],
          additionalProperties: false,
        },
        outputSchema: {
          type: 'object',
          properties: { shell: { type: 'string' }, script: { type: 'string' } },
          required: ['shell', 'script'],
          additionalProperties: false,
        },
        resultSchemaVersion: 1,
        access: { mode: 'fixed', level: 'read' },
        asyncMode: 'sync',
        requiresSession: false,
        examples: ['buckyos --output text completion generate --shell bash'],
        handler: (_ctx, input) => {
          const shell = String(input.shell)
          return Promise.resolve({ shell, script: completionScript(registry, shell) })
        },
      },
    ],
  }
}

function setGlobalValue(config: ToolConfig, key: string, value: string): void {
  if (key === 'default_profile') {
    validateProfileName(value)
    config.default_profile = value
  } else if (key === 'output') {
    config.output = outputValue(value)
  } else {
    throw configValueError(`unsupported global configuration key: ${key}`)
  }
}

function setProfileValue(profile: ProfileConfig, key: string, value: string): void {
  if (key === 'zone' || key === 'endpoint' || key === 'identity') profile[key] = value
  else if (key === 'default_protocol') {
    if (value !== 'http://' && value !== 'https://') {
      throw configValueError('default_protocol must be http:// or https://')
    }
    profile.default_protocol = value
  } else if (key === 'output') profile.output = outputValue(value)
  else throw configValueError(`unsupported profile configuration key: ${key}`)
}

function outputValue(value: string): 'json' | 'jsonl' | 'table' | 'text' | 'raw' {
  if (!['json', 'jsonl', 'table', 'text', 'raw'].includes(value)) {
    throw configValueError(`invalid output format: ${value}`)
  }
  return value as 'json' | 'jsonl' | 'table' | 'text' | 'raw'
}

function optionalString(value: unknown): string | undefined {
  return typeof value === 'string' && value ? value : undefined
}

function completionScript(registry: CommandRegistry, shell: string): string {
  const modules = registry.modules().map((module) => module.name).join(' ')
  const commands = registry.commands().map((command) => `${command.module}:${command.verb}`).join(
    ' ',
  )
  if (shell === 'fish') {
    return `complete -c buckyos -f\ncomplete -c buckyos -n '__fish_use_subcommand' -a '${modules}'\n# ${commands}\n`
  }
  if (shell === 'zsh') {
    return `#compdef buckyos\n_arguments '1:module:(${modules})' '*::argument:->args'\n# ${commands}\n`
  }
  return `_buckyos_complete() {\n  if [ "${'${COMP_CWORD}'}" -eq 1 ]; then\n    COMPREPLY=( $(compgen -W '${modules}' -- "${'${COMP_WORDS[COMP_CWORD]}'}") )\n  fi\n}\ncomplete -F _buckyos_complete buckyos\n# ${commands}\n`
}
