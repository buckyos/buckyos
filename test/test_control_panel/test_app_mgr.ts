/**
 * test_app_mgr.ts — DV 测试用例，覆盖 control_panel 的 app_service_mgr.rs
 *
 * 测试 RPC 方法：
 *   - apps.list  → handle_apps_list
 *   - apps.details → handle_app_detials
 *
 * 通过 deno 直接运行：
 *   deno install
 *   deno run --allow-net --allow-read --allow-env test_app_mgr.ts
 */

import { initTestRuntime } from "../test_helpers/buckyos_client.ts";
import {
  fetchAppList,
  fetchAppDetails,
  type AppsListResponse,
  type AppDetailsResponse,
  type AppSummary,
} from "../../src/frame/desktop/src/api/app_mgr.ts";

// ---------------------------------------------------------------------------
// Test runner
// ---------------------------------------------------------------------------

type TestResult = {
  name: string;
  ok: boolean;
  durationMs: number;
  error?: string;
};

async function runCase(
  name: string,
  fn: () => Promise<void>,
): Promise<TestResult> {
  const start = Date.now();
  try {
    await fn();
    const ms = Date.now() - start;
    console.log(`  ✓ ${name} (${ms}ms)`);
    return { name, ok: true, durationMs: ms };
  } catch (err) {
    const ms = Date.now() - start;
    const msg = err instanceof Error ? err.message : String(err);
    console.error(`  ✗ ${name} (${ms}ms): ${msg}`);
    return { name, ok: false, durationMs: ms, error: msg };
  }
}

function assert(condition: boolean, message: string): void {
  if (!condition) throw new Error(`Assertion failed: ${message}`);
}

// ---------------------------------------------------------------------------
// Main
// ---------------------------------------------------------------------------

const SYSTEM_BUILTIN_APP_IDS = ["messagehub", "homestation", "content-store"];

// DV 环境预装应用 (来自 src/rootfs/etc/scheduler/boot.template.toml)
const DV_PREINSTALLED_APPS: {
  app_id: string;
  show_name: string;
  author: string;
  version: string;
}[] = [
  {
    app_id: "buckyos_filebrowser",
    show_name: "BuckyOS File Browser",
    author: "did:web:buckyos.ai",
    version: "0.5.1",
  },
  {
    app_id: "buckyos_systest",
    show_name: "BuckyOS System Test",
    author: "did:web:buckyos.ai",
    version: "0.5.1",
  },
];

// DV 环境默认 Agent (来自 src/kernel/scheduler/src/system_config_builder.rs)
const DV_DEFAULT_AGENTS: {
  app_id: string;
  show_name: string;
  author: string;
}[] = [
  {
    app_id: "jarvis",
    show_name: "Jarvis",
    author: "did:bns:buckyos",
  },
];

async function main() {
  console.log("=== test_app_mgr: app_service_mgr.rs DV tests ===\n");

  // 1. Initialize as AppClient
  const { userId: accountUserId, ownerUserId, zoneHost } = await initTestRuntime();
  console.log(`zone: ${zoneHost}, user: ${accountUserId}, owner: ${ownerUserId}\n`);

  const results: TestResult[] = [];

  // -----------------------------------------------------------------------
  // Test: apps.list — 返回应用列表
  // -----------------------------------------------------------------------
  console.log("[apps.list]");

  let appsList: AppsListResponse | null = null;

  results.push(
    await runCase("apps.list returns valid response", async () => {
      const { data, error } = await fetchAppList({ userId: ownerUserId });
      assert(!error, `fetchAppList should not error: ${error}`);
      assert(data !== null, "data should not be null");
      appsList = data!;
      assert(typeof appsList.user_id === "string", "user_id should be string");
      assert(typeof appsList.total === "number", "total should be number");
      assert(Array.isArray(appsList.apps), "apps should be array");
      assert(
        appsList.total === appsList.apps.length,
        `total (${appsList.total}) should match apps.length (${appsList.apps.length})`,
      );
    }),
  );

  results.push(
    await runCase("apps.list includes system built-in apps", async () => {
      assert(appsList !== null, "appsList should be populated");
      const systemApps = appsList!.apps.filter((app) => app.is_system);
      assert(
        systemApps.length >= SYSTEM_BUILTIN_APP_IDS.length,
        `should have at least ${SYSTEM_BUILTIN_APP_IDS.length} system apps, got ${systemApps.length}`,
      );
      for (const expectedId of SYSTEM_BUILTIN_APP_IDS) {
        const found = systemApps.find((app) => app.app_id === expectedId);
        assert(!!found, `system app '${expectedId}' should be in list`);
      }
    }),
  );

  results.push(
    await runCase("apps.list system apps have correct metadata", async () => {
      assert(appsList !== null, "appsList should be populated");
      const systemApps = appsList!.apps.filter((app) => app.is_system);
      for (const app of systemApps) {
        assert(
          typeof app.app_id === "string" && app.app_id.length > 0,
          `system app should have non-empty app_id`,
        );
        assert(
          typeof app.version === "string" && app.version.length > 0,
          `system app '${app.app_id}' should have version`,
        );
        assert(
          typeof app.icon_res_url === "string",
          `system app '${app.app_id}' should have icon_res_url`,
        );
        assert(
          app.is_system === true,
          `system app '${app.app_id}' should have is_system=true`,
        );
        assert(
          app.is_agent === false,
          `system app '${app.app_id}' should have is_agent=false`,
        );
        assert(
          app.enable === true,
          `system app '${app.app_id}' should be enabled`,
        );
        assert(
          typeof app.spec_path === "string" &&
            app.spec_path.startsWith("system/apps/"),
          `system app '${app.app_id}' spec_path should start with 'system/apps/'`,
        );
      }
    }),
  );

  results.push(
    await runCase("apps.list all entries have required fields", async () => {
      assert(appsList !== null, "appsList should be populated");
      for (const app of appsList!.apps) {
        assert(typeof app.app_id === "string", "app should have string app_id");
        assert(
          typeof app.app_type === "string",
          `app '${app.app_id}' should have string app_type`,
        );
        assert(
          typeof app.state === "string",
          `app '${app.app_id}' should have string state`,
        );
        assert(
          typeof app.enable === "boolean",
          `app '${app.app_id}' enable should be boolean`,
        );
        assert(
          typeof app.is_agent === "boolean",
          `app '${app.app_id}' is_agent should be boolean`,
        );
        assert(
          typeof app.is_system === "boolean",
          `app '${app.app_id}' is_system should be boolean`,
        );
        assert(
          typeof app.app_index === "number",
          `app '${app.app_id}' app_index should be number`,
        );
        assert(
          typeof app.user_id === "string",
          `app '${app.app_id}' user_id should be string`,
        );
      }
    }),
  );

  results.push(
    await runCase("apps.list is sorted by app_index", async () => {
      assert(appsList !== null, "appsList should be populated");
      for (let i = 1; i < appsList!.apps.length; i++) {
        const prev = appsList!.apps[i - 1].app_index;
        const curr = appsList!.apps[i].app_index;
        assert(
          prev <= curr,
          `apps should be sorted by app_index: index[${i - 1}]=${prev} > index[${i}]=${curr}`,
        );
      }
    }),
  );

  results.push(
    await runCase(
      "apps.list includes DV preinstalled apps",
      async () => {
        assert(appsList !== null, "appsList should be populated");
        for (const expected of DV_PREINSTALLED_APPS) {
          const found = appsList!.apps.find(
            (app) => app.app_id === expected.app_id,
          );
          assert(
            !!found,
            `DV preinstalled app '${expected.app_id}' should be in list`,
          );
          assert(
            found!.is_system === false,
            `'${expected.app_id}' should not be a system app`,
          );
          assert(
            found!.show_name === expected.show_name,
            `'${expected.app_id}' show_name should be '${expected.show_name}', got '${found!.show_name}'`,
          );
          assert(
            found!.author === expected.author,
            `'${expected.app_id}' author should be '${expected.author}', got '${found!.author}'`,
          );
          assert(
            found!.version === expected.version,
            `'${expected.app_id}' version should be '${expected.version}', got '${found!.version}'`,
          );
        }
      },
    ),
  );

  // -----------------------------------------------------------------------
  // Test: apps.list — DV 默认 Agent
  // -----------------------------------------------------------------------
  results.push(
    await runCase(
      "apps.list includes DV default agent (jarvis)",
      async () => {
        assert(appsList !== null, "appsList should be populated");
        for (const expected of DV_DEFAULT_AGENTS) {
          const found = appsList!.apps.find(
            (app) => app.app_id === expected.app_id,
          );
          assert(
            !!found,
            `DV default agent '${expected.app_id}' should be in list`,
          );
          assert(
            found!.is_agent === true,
            `'${expected.app_id}' should have is_agent=true`,
          );
          assert(
            found!.is_system === false,
            `'${expected.app_id}' should not be a system app`,
          );
          assert(
            found!.show_name === expected.show_name,
            `'${expected.app_id}' show_name should be '${expected.show_name}', got '${found!.show_name}'`,
          );
          assert(
            found!.author === expected.author,
            `'${expected.app_id}' author should be '${expected.author}', got '${found!.author}'`,
          );
          assert(
            found!.enable === true,
            `'${expected.app_id}' should be enabled`,
          );
        }
      },
    ),
  );

  // -----------------------------------------------------------------------
  // Test: apps.details — DV 预装应用详情
  // -----------------------------------------------------------------------
  for (const expected of DV_PREINSTALLED_APPS) {
    results.push(
      await runCase(
        `apps.details returns DV preinstalled app (${expected.app_id})`,
        async () => {
          const { data, error } = await fetchAppDetails(expected.app_id, { userId: ownerUserId });
          assert(!error, `fetchAppDetails should not error: ${error}`);
          assert(data !== null, "data should not be null");
          assert(
            data!.app_id === expected.app_id,
            `app_id should be '${expected.app_id}'`,
          );
          assert(
            data!.is_system === false,
            `'${expected.app_id}' should not be a system app`,
          );
          assert(
            data!.is_agent === false,
            `'${expected.app_id}' should not be an agent`,
          );
          const spec = data!.spec as Record<string, unknown>;
          assert(
            typeof spec.app_doc === "object" && spec.app_doc !== null,
            "spec should contain app_doc",
          );
          const appDoc = spec.app_doc as Record<string, unknown>;
          assert(
            appDoc.name === expected.app_id,
            `app_doc.name should be '${expected.app_id}'`,
          );
          assert(
            appDoc.author === expected.author,
            `app_doc.author should be '${expected.author}'`,
          );
        },
      ),
    );
  }

  // -----------------------------------------------------------------------
  // Test: apps.details — DV 默认 Agent 详情
  // -----------------------------------------------------------------------
  for (const expected of DV_DEFAULT_AGENTS) {
    results.push(
      await runCase(
        `apps.details returns DV default agent (${expected.app_id})`,
        async () => {
          const { data, error } = await fetchAppDetails(expected.app_id, { userId: ownerUserId });
          assert(!error, `fetchAppDetails should not error: ${error}`);
          assert(data !== null, "data should not be null");
          assert(
            data!.app_id === expected.app_id,
            `app_id should be '${expected.app_id}'`,
          );
          assert(
            data!.is_agent === true,
            `'${expected.app_id}' should be an agent`,
          );
          assert(
            data!.is_system === false,
            `'${expected.app_id}' should not be a system app`,
          );
          const spec = data!.spec as Record<string, unknown>;
          assert(
            typeof spec.app_doc === "object" && spec.app_doc !== null,
            "spec should contain app_doc",
          );
          const appDoc = spec.app_doc as Record<string, unknown>;
          assert(
            appDoc.name === expected.app_id,
            `app_doc.name should be '${expected.app_id}'`,
          );
          assert(
            appDoc.author === expected.author,
            `app_doc.author should be '${expected.author}'`,
          );
        },
      ),
    );
  }

  results.push(
    await runCase("apps.list with explicit user_id works", async () => {
      const { data, error } = await fetchAppList({ userId: ownerUserId });
      assert(!error, `fetchAppList with userId should not error: ${error}`);
      assert(data !== null, "data should not be null");
      assert(
        data!.user_id === ownerUserId,
        `user_id should match requested: got '${data!.user_id}', expected '${ownerUserId}'`,
      );
      assert(Array.isArray(data!.apps), "apps should be array");
    }),
  );

  // -----------------------------------------------------------------------
  // Test: apps.details — 获取应用详情
  // -----------------------------------------------------------------------
  console.log("\n[apps.details]");

  results.push(
    await runCase(
      "apps.details returns system app (messagehub)",
      async () => {
        const { data, error } = await fetchAppDetails("messagehub", { userId: ownerUserId });
        assert(!error, `fetchAppDetails should not error: ${error}`);
        assert(data !== null, "data should not be null");
        const detail = data!;
        assert(
          detail.app_id === "messagehub",
          `app_id should be 'messagehub', got '${detail.app_id}'`,
        );
        assert(detail.is_system === true, "messagehub should be a system app");
        assert(detail.is_agent === false, "messagehub should not be an agent");
        assert(
          typeof detail.spec === "object" && detail.spec !== null,
          "spec should be a non-null object",
        );
        assert(
          typeof detail.summary === "object" && detail.summary !== null,
          "summary should be a non-null object",
        );
        assert(
          detail.summary.app_id === "messagehub",
          "summary.app_id should match",
        );
        assert(
          typeof detail.spec_path === "string" &&
            detail.spec_path.includes("messagehub"),
          `spec_path should reference messagehub`,
        );
      },
    ),
  );

  results.push(
    await runCase(
      "apps.details returns system app (homestation)",
      async () => {
        const { data, error } = await fetchAppDetails("homestation", { userId: ownerUserId });
        assert(!error, `fetchAppDetails should not error: ${error}`);
        assert(data !== null, "data should not be null");
        assert(data!.app_id === "homestation", `app_id should be 'homestation'`);
        assert(data!.is_system === true, "should be system app");
      },
    ),
  );

  results.push(
    await runCase(
      "apps.details returns system app (content-store)",
      async () => {
        const { data, error } = await fetchAppDetails("content-store", { userId: ownerUserId });
        assert(!error, `fetchAppDetails should not error: ${error}`);
        assert(data !== null, "data should not be null");
        assert(
          data!.app_id === "content-store",
          `app_id should be 'content-store'`,
        );
        assert(data!.is_system === true, "should be system app");
      },
    ),
  );

  results.push(
    await runCase("apps.details spec contains app_doc fields", async () => {
      const { data, error } = await fetchAppDetails("messagehub", { userId: ownerUserId });
      assert(!error, `fetchAppDetails should not error: ${error}`);
      assert(data !== null, "data should not be null");
      const spec = data!.spec as Record<string, unknown>;
      assert(
        typeof spec.app_doc === "object" && spec.app_doc !== null,
        "spec should contain app_doc",
      );
      const appDoc = spec.app_doc as Record<string, unknown>;
      assert(typeof appDoc.name === "string", "app_doc should have name");
      assert(
        typeof appDoc.version === "string",
        "app_doc should have version",
      );
    }),
  );

  results.push(
    await runCase(
      "apps.details for non-existent app returns error",
      async () => {
        const { data, error } = await fetchAppDetails(
          "non_existent_app_xyz_12345",
          { userId: ownerUserId },
        );
        assert(
          error !== null || data === null,
          "request should fail for non-existent app",
        );
      },
    ),
  );

  results.push(
    await runCase("apps.details with explicit user_id works", async () => {
      const { data, error } = await fetchAppDetails("messagehub", { userId: ownerUserId });
      assert(!error, `fetchAppDetails with userId should not error: ${error}`);
      assert(data !== null, "data should not be null");
      assert(
        data!.user_id === ownerUserId,
        `user_id should match: got '${data!.user_id}', expected '${ownerUserId}'`,
      );
    }),
  );

  // -----------------------------------------------------------------------
  // Test: apps.list and apps.details consistency
  // -----------------------------------------------------------------------
  console.log("\n[consistency]");

  results.push(
    await runCase(
      "apps.details matches apps.list summary for each system app",
      async () => {
        assert(appsList !== null, "appsList should be populated");
        for (const expectedId of SYSTEM_BUILTIN_APP_IDS) {
          const listItem = appsList!.apps.find(
            (app) => app.app_id === expectedId,
          );
          assert(!!listItem, `'${expectedId}' should be in apps.list`);

          const { data, error } = await fetchAppDetails(expectedId, { userId: ownerUserId });
          assert(!error, `fetchAppDetails('${expectedId}') should not error`);
          assert(data !== null, `details for '${expectedId}' should not be null`);
          assert(
            data!.summary.app_id === listItem!.app_id,
            `summary.app_id mismatch for '${expectedId}'`,
          );
          assert(
            data!.summary.is_system === listItem!.is_system,
            `summary.is_system mismatch for '${expectedId}'`,
          );
          assert(
            data!.summary.version === listItem!.version,
            `summary.version mismatch for '${expectedId}'`,
          );
        }
      },
    ),
  );

  results.push(
    await runCase(
      "apps.details matches apps.list for DV preinstalled apps",
      async () => {
        assert(appsList !== null, "appsList should be populated");
        for (const expected of DV_PREINSTALLED_APPS) {
          const listItem = appsList!.apps.find(
            (app) => app.app_id === expected.app_id,
          );
          assert(!!listItem, `'${expected.app_id}' should be in apps.list`);

          const { data, error } = await fetchAppDetails(expected.app_id, { userId: ownerUserId });
          assert(!error, `fetchAppDetails('${expected.app_id}') should not error`);
          assert(data !== null, `details for '${expected.app_id}' should not be null`);
          assert(
            data!.summary.app_id === listItem!.app_id,
            `summary.app_id mismatch for '${expected.app_id}'`,
          );
          assert(
            data!.summary.version === listItem!.version,
            `summary.version mismatch for '${expected.app_id}'`,
          );
          assert(
            data!.summary.show_name === listItem!.show_name,
            `summary.show_name mismatch for '${expected.app_id}'`,
          );
        }
      },
    ),
  );

  results.push(
    await runCase(
      "apps.details matches apps.list for DV default agents",
      async () => {
        assert(appsList !== null, "appsList should be populated");
        for (const expected of DV_DEFAULT_AGENTS) {
          const listItem = appsList!.apps.find(
            (app) => app.app_id === expected.app_id,
          );
          assert(!!listItem, `'${expected.app_id}' should be in apps.list`);

          const { data, error } = await fetchAppDetails(expected.app_id, { userId: ownerUserId });
          assert(!error, `fetchAppDetails('${expected.app_id}') should not error`);
          assert(data !== null, `details for '${expected.app_id}' should not be null`);
          assert(
            data!.summary.app_id === listItem!.app_id,
            `summary.app_id mismatch for '${expected.app_id}'`,
          );
          assert(
            data!.summary.is_agent === listItem!.is_agent,
            `summary.is_agent mismatch for '${expected.app_id}'`,
          );
          assert(
            data!.summary.show_name === listItem!.show_name,
            `summary.show_name mismatch for '${expected.app_id}'`,
          );
        }
      },
    ),
  );

  // -----------------------------------------------------------------------
  // Summary
  // -----------------------------------------------------------------------
  console.log("\n=== Summary ===");
  const passed = results.filter((r) => r.ok).length;
  const failed = results.filter((r) => !r.ok).length;
  const totalMs = results.reduce((sum, r) => sum + r.durationMs, 0);
  console.log(
    `${passed} passed, ${failed} failed, ${results.length} total (${totalMs}ms)`,
  );

  if (failed > 0) {
    console.log("\nFailed tests:");
    for (const r of results.filter((r) => !r.ok)) {
      console.log(`  - ${r.name}: ${r.error}`);
    }
    Deno.exit(1);
  }

  console.log("\nAll tests passed!");
}

main().catch((err) => {
  console.error("Fatal error:", err);
  Deno.exit(2);
});
