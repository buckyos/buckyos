import { type ChildProcess, spawn } from "node:child_process";
import { mkdtemp, rm, writeFile } from "node:fs/promises";
import { createServer } from "node:net";
import { tmpdir } from "node:os";
import { join } from "node:path";
import { ndm_proxy, ndn } from "buckyos";

export type CloudCatalogKind =
  | "model_driver"
  | "provider_rules"
  | "known_provider";

export type CloudCatalogFile = {
  catalog_kind: CloudCatalogKind;
  catalog_id: string;
  revision_seq: number;
  contents: Record<string, unknown>;
};

export type CloudCatalogTombstone = {
  catalog_kind: CloudCatalogKind;
  catalog_id: string;
  revision_seq: number;
};

type CloudUpdateFixtureOptions = {
  gatewayUrl: string;
  sessionToken: string;
  runId: string;
  gatewayBinary: string;
  namedStoreConfigPath: string;
  gatewayControlUrl: string;
  systemRoot: string;
  startupTimeoutMs?: number;
};

type StoredObject = {
  id: InstanceType<typeof ndn.ObjId>;
  size: number;
};

function safeSuffix(value: string): string {
  const suffix = value.replace(/[^a-zA-Z0-9_-]/g, "-");
  if (!suffix) throw new Error("cloud update fixture run_id is empty");
  return suffix;
}

async function allocatePort(): Promise<number> {
  return await new Promise((resolvePromise, reject) => {
    const server = createServer();
    server.once("error", reject);
    server.listen(0, "127.0.0.1", () => {
      const address = server.address();
      if (!address || typeof address === "string") {
        server.close();
        reject(new Error("failed to allocate loopback port"));
        return;
      }
      const port = address.port;
      server.close((error) => error ? reject(error) : resolvePromise(port));
    });
  });
}

async function runGatewayCommand(
  binary: string,
  args: string[],
  systemRoot: string,
): Promise<string> {
  return await new Promise((resolvePromise, reject) => {
    const child = spawn(binary, args, {
      env: { ...process.env, BUCKYOS_ROOT: systemRoot },
      stdio: ["ignore", "pipe", "pipe"],
    });
    let stdout = "";
    let stderr = "";
    child.stdout?.on("data", (chunk) => stdout += String(chunk));
    child.stderr?.on("data", (chunk) => stderr += String(chunk));
    child.once("error", reject);
    child.once("close", (code) => {
      if (code === 0) resolvePromise(stdout);
      else {reject(
          new Error(
            `cyfs-gateway ${args[0]} failed: ${
              stderr.trim() || stdout.trim() || code
            }`,
          ),
        );}
    });
  });
}

async function waitReady(
  url: string,
  child: ChildProcess,
  timeoutMs: number,
): Promise<void> {
  const deadline = Date.now() + timeoutMs;
  while (Date.now() < deadline) {
    if (child.exitCode !== null) {
      throw new Error(
        `temporary cloud NDN gateway exited with code ${child.exitCode}`,
      );
    }
    try {
      await fetch(url);
      return;
    } catch {
      await new Promise((resolvePromise) => setTimeout(resolvePromise, 100));
    }
  }
  throw new Error(
    `temporary cloud NDN gateway did not become ready within ${timeoutMs}ms`,
  );
}

async function stopChild(child: ChildProcess): Promise<void> {
  if (child.exitCode !== null) return;
  await new Promise<void>((resolvePromise) => {
    child.once("close", () => resolvePromise());
    child.kill("SIGTERM");
  });
}

function buildGatewayConfig(input: {
  controlPort: number;
  dataPort: number;
  routePrefix: string;
  namedStoreConfigPath: string;
  routes: Map<string, string>;
}): Record<string, unknown> {
  const routeRules = [...input.routes.entries()].map(([path, objId]) =>
    `if eq \${REQ.path} "${input.routePrefix}/${path}" then\n  global RESP_cyobj_id="${objId}";\nend`
  ).join("\n");
  return {
    stacks: {
      __control_server__: {
        bind: `127.0.0.1:${input.controlPort}`,
        protocol: "tcp",
        hook_point: {
          main: {
            priority: 1,
            blocks: {
              default: {
                priority: 1,
                block: 'return "server __control_server__";',
              },
            },
          },
        },
      },
      aicc_cloud_ndn_http: {
        bind: `127.0.0.1:${input.dataPort}`,
        protocol: "tcp",
        hook_point: {
          main: {
            priority: 1,
            blocks: {
              default: {
                priority: 1,
                block: 'return "server aicc_cloud_ndn";',
              },
            },
          },
        },
      },
    },
    servers: {
      __control_server__: { type: "control_server" },
      aicc_cloud_ndn: {
        type: "cyfs-dir",
        named_store_config_path: input.namedStoreConfigPath,
        url_prefix: input.routePrefix,
        hook_point: {
          resolve: {
            blocks: { main: { block: routeRules } },
          },
        },
      },
    },
  };
}

export class CloudUpdateFixtureService {
  readonly publicBaseUrl: string;
  localBaseUrl: string;
  private child: ChildProcess;
  private readonly tempRoot: string;
  private readonly configPath: string;
  private readonly routePrefix: string;
  private controlPort: number;
  private dataPort: number;
  private readonly options: CloudUpdateFixtureOptions;
  private readonly ndm: ReturnType<typeof ndm_proxy.createNdmProxyClient>;
  private readonly storedIds = new Set<string>();
  private stopped = false;

  private constructor(input: {
    options: CloudUpdateFixtureOptions;
    child: ChildProcess;
    tempRoot: string;
    configPath: string;
    routePrefix: string;
    controlPort: number;
    dataPort: number;
  }) {
    this.options = input.options;
    this.child = input.child;
    this.tempRoot = input.tempRoot;
    this.configPath = input.configPath;
    this.routePrefix = input.routePrefix;
    this.controlPort = input.controlPort;
    this.dataPort = input.dataPort;
    this.publicBaseUrl = `${
      input.options.gatewayUrl.replace(/\/+$/, "")
    }${input.routePrefix}`;
    this.localBaseUrl =
      `http://127.0.0.1:${input.dataPort}${input.routePrefix}`;
    this.ndm = ndm_proxy.createNdmProxyClient({
      endpoint: input.options.gatewayUrl,
      sessionToken: input.options.sessionToken,
      fetcher: (request: RequestInfo | URL, init?: RequestInit) => {
        const target = typeof request === "string"
          ? request.replaceAll("%3A", ":").replaceAll("%3a", ":")
          : request instanceof URL
          ? new URL(
            request.toString().replaceAll("%3A", ":").replaceAll("%3a", ":"),
          )
          : request;
        return fetch(target, init);
      },
    });
  }

  static async start(
    options: CloudUpdateFixtureOptions,
  ): Promise<CloudUpdateFixtureService> {
    const routePrefix = `/aicc-cloud-update-${safeSuffix(options.runId)}`;
    const [controlPort, dataPort] = await Promise.all([
      allocatePort(),
      allocatePort(),
    ]);
    const tempRoot = await mkdtemp(join(tmpdir(), "aicc-cloud-ndn-"));
    const configPath = join(tempRoot, "gateway.json");
    await writeFile(
      configPath,
      `${
        JSON.stringify(
          buildGatewayConfig({
            controlPort,
            dataPort,
            routePrefix,
            namedStoreConfigPath: options.namedStoreConfigPath,
            routes: new Map(),
          }),
          null,
          2,
        )
      }\n`,
    );
    const child = spawn(options.gatewayBinary, ["--config_file", configPath], {
      env: { ...process.env, BUCKYOS_ROOT: tempRoot },
      stdio: "ignore",
    });
    try {
      await waitReady(
        `http://127.0.0.1:${dataPort}${routePrefix}/mix256:not-a-real-object`,
        child,
        options.startupTimeoutMs ?? 15_000,
      );
      await runGatewayCommand(options.gatewayBinary, [
        "add_router",
        "--id",
        "server:node_gateway",
        "--uri",
        routePrefix,
        "--target",
        `http://127.0.0.1:${dataPort}`,
        "--server",
        options.gatewayControlUrl,
      ], options.systemRoot);
      return new CloudUpdateFixtureService({
        options,
        child,
        tempRoot,
        configPath,
        routePrefix,
        controlPort,
        dataPort,
      });
    } catch (error) {
      await stopChild(child).catch(() => undefined);
      await rm(tempRoot, { recursive: true, force: true });
      throw error;
    }
  }

  private async storeJson(
    value: Record<string, unknown>,
  ): Promise<StoredObject> {
    const [id, encoded] = ndn.buildNamedObjectByJson("jobj", value);
    await this.ndm.putObject({ obj_id: id.toString(), obj_data: encoded });
    this.storedIds.add(id.toString());
    return { id, size: new TextEncoder().encode(encoded).byteLength };
  }

  private async replaceRoutes(routes: Map<string, string>): Promise<void> {
    await runGatewayCommand(this.options.gatewayBinary, [
      "remove_router",
      "--id",
      "server:node_gateway",
      "--uri",
      this.routePrefix,
      "--target",
      `http://127.0.0.1:${this.dataPort}`,
      "--server",
      this.options.gatewayControlUrl,
    ], this.options.systemRoot);
    await stopChild(this.child);
    [this.controlPort, this.dataPort] = await Promise.all([
      allocatePort(),
      allocatePort(),
    ]);
    this.localBaseUrl = `http://127.0.0.1:${this.dataPort}${this.routePrefix}`;
    await writeFile(
      this.configPath,
      `${
        JSON.stringify(
          buildGatewayConfig({
            controlPort: this.controlPort,
            dataPort: this.dataPort,
            routePrefix: this.routePrefix,
            namedStoreConfigPath: this.options.namedStoreConfigPath,
            routes,
          }),
          null,
          2,
        )
      }\n`,
    );
    this.child = spawn(this.options.gatewayBinary, [
      "--config_file",
      this.configPath,
    ], {
      env: { ...process.env, BUCKYOS_ROOT: this.tempRoot },
      stdio: "ignore",
    });
    await waitReady(
      `http://127.0.0.1:${this.dataPort}${this.routePrefix}/mix256:not-a-real-object`,
      this.child,
      this.options.startupTimeoutMs ?? 15_000,
    );
    await runGatewayCommand(this.options.gatewayBinary, [
      "add_router",
      "--id",
      "server:node_gateway",
      "--uri",
      this.routePrefix,
      "--target",
      `http://127.0.0.1:${this.dataPort}`,
      "--server",
      this.options.gatewayControlUrl,
    ], this.options.systemRoot);
  }

  async publish(input: {
    revisionSeq: number;
    files: CloudCatalogFile[];
    tombstones?: CloudCatalogTombstone[];
  }): Promise<{ sourceUrl: string; rootObjId: string }> {
    if (!Number.isSafeInteger(input.revisionSeq) || input.revisionSeq < 2) {
      throw new Error("cloud fixture revision_seq must be a safe integer >= 2");
    }
    const manifestFiles: Array<Record<string, unknown>> = [];
    const routes = new Map<string, string>();
    for (const file of input.files) {
      const stored = await this.storeJson(file.contents);
      const directory = file.catalog_kind === "model_driver"
        ? "model-drivers"
        : file.catalog_kind === "provider_rules"
        ? "provider-rules"
        : "known-providers";
      const path =
        `aicc/provider-catalog/v2/${directory}/${file.catalog_id}-${file.revision_seq}.json`;
      routes.set(path, stored.id.toString());
      manifestFiles.push({
        catalog_kind: file.catalog_kind,
        catalog_id: file.catalog_id,
        path,
        schema_version: 1,
        revision_seq: file.revision_seq,
        obj_id: stored.id.toString(),
      });
    }
    const manifestPath =
      `aicc/provider-catalog/v2/manifest-${input.revisionSeq}.json`;
    const manifest = await this.storeJson({
      format: "buckyos.aicc.provider-catalog-manifest",
      protocol_version: 2,
      revision_seq: input.revisionSeq,
      match: "*",
      required_features: [],
      files: manifestFiles,
      tombstones: input.tombstones ?? [],
    });
    routes.set(manifestPath, manifest.id.toString());
    const index = await this.storeJson({
      format: "buckyos.aicc.provider-catalog-index",
      index_version: 2,
      index_revision_seq: input.revisionSeq,
      tracks: [{
        revision_seq: input.revisionSeq,
        manifest_path: manifestPath,
        manifest_obj_id: manifest.id.toString(),
        match: "*",
        required_features: [],
      }],
    });
    routes.set("aicc/provider-catalog/index.json", index.id.toString());
    await this.replaceRoutes(routes);
    const sourceUrl = this.localBaseUrl;
    const response = await fetch(
      `${sourceUrl}/aicc/provider-catalog/index.json`,
      {
        headers: { authorization: `Bearer ${this.options.sessionToken}` },
      },
    );
    if (!response.ok) {
      throw new Error(
        `published cloud NDN index is unreachable: HTTP ${response.status}`,
      );
    }
    const observed = await response.json() as { index_revision_seq?: unknown };
    if (observed.index_revision_seq !== input.revisionSeq) {
      throw new Error("published cloud NDN index returned unexpected contents");
    }
    return { sourceUrl, rootObjId: index.id.toString() };
  }

  async stop(): Promise<void> {
    if (this.stopped) return;
    this.stopped = true;
    let routeError: unknown;
    try {
      await runGatewayCommand(this.options.gatewayBinary, [
        "remove_router",
        "--id",
        "server:node_gateway",
        "--uri",
        this.routePrefix,
        "--target",
        `http://127.0.0.1:${this.dataPort}`,
        "--server",
        this.options.gatewayControlUrl,
      ], this.options.systemRoot);
    } catch (error) {
      routeError = error;
    }
    await stopChild(this.child).catch((error) => routeError ??= error);
    for (const id of [...this.storedIds].reverse()) {
      await this.ndm.removeObject({ obj_id: id }).catch(() => undefined);
    }
    await rm(this.tempRoot, { recursive: true, force: true });
    if (routeError) throw routeError;
  }
}
