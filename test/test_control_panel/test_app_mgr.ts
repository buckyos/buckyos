/**
 * DV coverage for the beta 2.2 apps.list/apps.details identity contract.
 * Run with: deno run --allow-net --allow-read --allow-env test_app_mgr.ts
 */

import { initTestRuntime } from "../test_helpers/buckyos_client.ts";
import {
  type AppsListResponse,
  type AppSummary,
  fetchAppDetails,
  fetchAppList,
} from "../../src/frame/desktop/src/api/app_mgr.ts";

type TestResult = {
  name: string;
  ok: boolean;
  durationMs: number;
  error?: string;
};

type ExpectedApp = {
  appId: string;
  appDid: string;
  showName: string;
  author: string;
  version: string;
};

const PREINSTALLED_APPS: ExpectedApp[] = [
  {
    appId: "buckyos-filebrowser.buckyos.bns.did",
    appDid: "did:bns:buckyos-filebrowser.buckyos",
    showName: "BuckyOS File Browser",
    author: "did:web:buckyos.ai",
    version: "0.5.1",
  },
  {
    appId: "buckyos-systest.buckyos.bns.did",
    appDid: "did:bns:buckyos-systest.buckyos",
    showName: "BuckyOS System Test",
    author: "did:web:buckyos.ai",
    version: "0.5.1",
  },
];

const SYSTEM_SERVICE_IDS = ["messagehub", "homestation", "content-store"];

function assert(condition: boolean, message: string): asserts condition {
  if (!condition) throw new Error(`Assertion failed: ${message}`);
}

async function runCase(
  name: string,
  fn: () => Promise<void>,
): Promise<TestResult> {
  const start = Date.now();
  try {
    await fn();
    const durationMs = Date.now() - start;
    console.log(`  ✓ ${name} (${durationMs}ms)`);
    return { name, ok: true, durationMs };
  } catch (error) {
    const durationMs = Date.now() - start;
    const message = error instanceof Error ? error.message : String(error);
    console.error(`  ✗ ${name} (${durationMs}ms): ${message}`);
    return { name, ok: false, durationMs, error: message };
  }
}

function findApp(apps: AppsListResponse, appId: string): AppSummary {
  const app = apps.apps.find((candidate) => candidate.app_id === appId);
  assert(app !== undefined, `${appId} should be present in apps.list`);
  return app;
}

async function main() {
  console.log("=== test_app_mgr: beta 2.2 App identity DV tests ===\n");
  const { ownerUserId, zoneHost } = await initTestRuntime();
  console.log(`zone: ${zoneHost}, owner: ${ownerUserId}\n`);

  const results: TestResult[] = [];
  const listResult = await fetchAppList({ userId: ownerUserId });
  results.push(
    await runCase("apps.list returns the requested owner scope", async () => {
      assert(
        !listResult.error,
        `apps.list should not error: ${listResult.error}`,
      );
      assert(listResult.data !== null, "apps.list data should not be null");
      assert(
        listResult.data.user_id === ownerUserId,
        "apps.list user_id should match owner",
      );
      assert(
        listResult.data.total === listResult.data.apps.length,
        "total should match apps length",
      );
    }),
  );

  const apps = listResult.data;
  if (apps) {
    results.push(
      await runCase(
        "ordinary apps expose canonical AppId/AppInstanceId/AppDID",
        async () => {
          for (const expected of PREINSTALLED_APPS) {
            const app = findApp(apps, expected.appId);
            assert(
              app.app_did === expected.appDid,
              `${expected.appId} AppDID should match`,
            );
            assert(
              app.app_instance_id === `${expected.appId}@${ownerUserId}`,
              `${expected.appId} AppInstanceId should be owner scoped`,
            );
            assert(
              app.owner_user_id === ownerUserId,
              `${expected.appId} owner should match`,
            );
            assert(
              app.show_name === expected.showName,
              `${expected.appId} show_name should match`,
            );
            assert(
              app.author === expected.author,
              `${expected.appId} author should match`,
            );
            assert(
              app.version === expected.version,
              `${expected.appId} version should match`,
            );
          }
        },
      ),
    );

    results.push(
      await runCase(
        "SystemServiceId values are not ordinary App installations",
        async () => {
          for (const serviceId of SYSTEM_SERVICE_IDS) {
            assert(
              !apps.apps.some((app) => app.app_id === serviceId),
              `${serviceId} must stay in the system service registry`,
            );
          }
        },
      ),
    );

    results.push(
      await runCase(
        "Agent identity is not substituted for its runtime App identity",
        async () => {
          assert(
            !apps.apps.some((app) =>
              app.app_id === "jarvis" || app.app_id.startsWith("jarvis.")
            ),
            "AgentDID/AgentId must not appear as an AppId",
          );
        },
      ),
    );

    for (const expected of PREINSTALLED_APPS) {
      results.push(
        await runCase(
          `apps.details uses strict AppDoc v1 (${expected.appId})`,
          async () => {
            const instanceId = `${expected.appId}@${ownerUserId}`;
            const { data, error } = await fetchAppDetails(instanceId);
            assert(!error, `apps.details should not error: ${error}`);
            assert(data !== null, "apps.details data should not be null");
            assert(
              data.app_id === expected.appId,
              "details AppId should match",
            );
            assert(
              data.app_instance_id === instanceId,
              "details AppInstanceId should match",
            );
            assert(
              data.owner_user_id === ownerUserId,
              "details owner should match",
            );
            assert(
              data.summary.app_instance_id === instanceId,
              "summary identity should match",
            );

            const appDoc = data.spec.app_doc as Record<string, unknown>;
            assert(
              appDoc.schema_version === 1,
              "AppDoc schema_version should be 1",
            );
            assert(appDoc.doc_type === "app", "AppDoc doc_type should be app");
            assert(
              appDoc.did === expected.appDid,
              "AppDoc DID should match AppDID",
            );
            assert(
              appDoc.author === expected.author,
              "AppDoc author should match",
            );
            assert(
              typeof appDoc.name === "string" && appDoc.name.length > 0,
              "AppDoc should expose BaseContentObject.name as non-identity metadata",
            );
            assert(
              Array.isArray(appDoc.categories),
              "AppDoc should expose BaseContentObject.categories",
            );
            assert(
              !Object.hasOwn(appDoc, "deps"),
              "AppDoc must not contain flattened PackageMeta",
            );
          },
        ),
      );
    }
  }

  results.push(
    await runCase(
      "apps.details rejects a non-existent AppInstanceId",
      async () => {
        const { data, error } = await fetchAppDetails(
          `missing.example@${ownerUserId}`,
        );
        assert(
          error !== null || data === null,
          "missing AppInstanceId should fail",
        );
      },
    ),
  );

  const failed = results.filter((result) => !result.ok);
  console.log(
    `\n=== ${results.length - failed.length}/${results.length} passed ===`,
  );
  if (failed.length > 0) Deno.exit(1);
}

if (import.meta.main) await main();
