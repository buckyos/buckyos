import { formatProviderListSummary } from "./ai_provider.ts";

function assertIncludes(actual: string, expected: string): void {
  if (!actual.includes(expected)) {
    throw new Error(
      `expected ${JSON.stringify(actual)} to include ${
        JSON.stringify(expected)
      }`,
    );
  }
}

Deno.test("formatProviderListSummary includes actionable provider state", () => {
  const summary = formatProviderListSummary({
    providers: [
      {
        provider_instance_name: "glm-main-china",
        provider_type: "cloud_api",
        provider_profile_id: "glm",
        protocol_adapter_id: "glm-chat",
        base_url: "https://open.bigmodel.cn/api/paas/v4",
        enabled: true,
        auth: {
          mode: "api_key",
          credential_kind: "bearer",
          configured: true,
        },
        inventory: {
          state: "loaded",
          revision: "rev",
          model_count: 10,
          updated_at_ms: 1,
        },
        health: {
          state: "healthy",
          checked_at_ms: 1,
        },
      },
    ],
    settings_revision: 7,
    inventory_revision: "9:2",
  });

  assertIncludes(summary, "provider list: 1 provider(s)");
  assertIncludes(summary, "settings_revision=7");
  assertIncludes(summary, "inventory_revision=9:2");
  assertIncludes(summary, "glm-main-china");
  assertIncludes(summary, "profile=glm");
  assertIncludes(summary, "inventory=loaded/10 models");
  assertIncludes(summary, "health=healthy");
});
