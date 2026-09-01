type NdnFixtureServiceOptions = {
  gatewayUrl: string;
  runId: string;
  gatewayBinary: string;
  namedStoreConfigPath: string;
  gatewayControlUrl: string;
  systemRoot: string;
  startupTimeoutMs?: number;
};

export type NdnFixtureService = {
  publicBaseUrl: string;
  stop: () => Promise<void>;
};

type GatewayConfigInput = {
  controlPort: number;
  dataPort: number;
  routePrefix: string;
  namedStoreConfigPath: string;
};

export function buildNdnGatewayConfig(
  input: GatewayConfigInput,
): Record<string, unknown> {
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
      aicc_ndn_http: {
        bind: `127.0.0.1:${input.dataPort}`,
        protocol: "tcp",
        hook_point: {
          main: {
            priority: 1,
            blocks: {
              default: {
                priority: 1,
                block: 'return "server aicc_ndn";',
              },
            },
          },
        },
      },
    },
    servers: {
      __control_server__: { type: "control_server" },
      aicc_ndn: {
        type: "cyfs-dir",
        named_store_config_path: input.namedStoreConfigPath,
        url_prefix: input.routePrefix,
      },
    },
  };
}

export function gatewayRouterArgs(input: {
  action: "add_router" | "remove_router";
  routePrefix: string;
  dataPort: number;
  gatewayControlUrl: string;
}): string[] {
  return [
    input.action,
    "--id",
    "server:node_gateway",
    "--uri",
    input.routePrefix,
    "--target",
    `http://127.0.0.1:${input.dataPort}`,
    "--server",
    input.gatewayControlUrl,
  ];
}

async function allocateLoopbackPorts(): Promise<[number, number]> {
  const first = Deno.listen({ hostname: "127.0.0.1", port: 0 });
  const second = Deno.listen({ hostname: "127.0.0.1", port: 0 });
  try {
    return [
      (first.addr as Deno.NetAddr).port,
      (second.addr as Deno.NetAddr).port,
    ];
  } finally {
    first.close();
    second.close();
  }
}

async function runGatewayCommand(
  gatewayBinary: string,
  args: string[],
  systemRoot: string,
): Promise<string> {
  const output = await new Deno.Command(gatewayBinary, {
    args,
    env: { BUCKYOS_ROOT: systemRoot },
    stdout: "piped",
    stderr: "piped",
  }).output();
  if (!output.success) {
    const stderr = new TextDecoder().decode(output.stderr).trim();
    const stdout = new TextDecoder().decode(output.stdout).trim();
    throw new Error(
      `cyfs-gateway ${args[0]} failed: ${stderr || stdout || output.code}`,
    );
  }
  return new TextDecoder().decode(output.stdout);
}

async function waitUntilReady(
  url: string,
  status: Promise<Deno.CommandStatus>,
  timeoutMs: number,
): Promise<void> {
  const deadline = Date.now() + timeoutMs;
  while (Date.now() < deadline) {
    const state = await Promise.race([
      status.then((value) => ({ exited: true as const, value })),
      new Promise<{ exited: false }>((resolve) =>
        setTimeout(() => resolve({ exited: false }), 50)
      ),
    ]);
    if (state.exited) {
      throw new Error(
        `temporary NDN gateway exited with code ${state.value.code}`,
      );
    }
    try {
      await fetch(url);
      return;
    } catch {
      await new Promise((resolve) => setTimeout(resolve, 100));
    }
  }
  throw new Error(
    `temporary NDN gateway did not become ready within ${timeoutMs}ms`,
  );
}

export async function startNdnFixtureService(
  input: NdnFixtureServiceOptions,
): Promise<NdnFixtureService> {
  const routeSuffix = input.runId.replace(/[^a-zA-Z0-9_-]/g, "-");
  const routePrefix = `/aicc-test-ndn-${routeSuffix}`;
  const publicBaseUrl = `${input.gatewayUrl.replace(/\/+$/, "")}${routePrefix}`;
  const [controlPort, dataPort] = await allocateLoopbackPorts();
  const tempRoot = await Deno.makeTempDir({ prefix: "aicc-ndn-" });
  const configPath = `${tempRoot}/gateway.json`;
  await Deno.writeTextFile(
    configPath,
    `${
      JSON.stringify(
        buildNdnGatewayConfig({
          controlPort,
          dataPort,
          routePrefix,
          namedStoreConfigPath: input.namedStoreConfigPath,
        }),
        null,
        2,
      )
    }\n`,
  );
  const child = new Deno.Command(input.gatewayBinary, {
    args: ["--config_file", configPath],
    env: { BUCKYOS_ROOT: tempRoot },
    stdout: "piped",
    stderr: "piped",
  }).spawn();
  const status = child.status;
  const stdout = new Response(child.stdout).text();
  const stderr = new Response(child.stderr).text();
  let routerInstalled = false;

  const stopChild = async (): Promise<void> => {
    try {
      child.kill("SIGTERM");
    } catch (error) {
      if (!(error instanceof TypeError)) throw error;
    }
    await status;
    await Promise.all([stdout, stderr]);
  };

  try {
    await waitUntilReady(
      `http://127.0.0.1:${dataPort}${routePrefix}/mix256:not-a-real-object`,
      status,
      input.startupTimeoutMs ?? 15_000,
    );
    await runGatewayCommand(
      input.gatewayBinary,
      gatewayRouterArgs({
        action: "add_router",
        routePrefix,
        dataPort,
        gatewayControlUrl: input.gatewayControlUrl,
      }),
      input.systemRoot,
    );
    routerInstalled = true;
  } catch (error) {
    await stopChild();
    await Deno.remove(tempRoot, { recursive: true });
    throw error;
  }

  let stopped = false;
  const service: NdnFixtureService = {
    publicBaseUrl,
    stop: async () => {
      if (stopped) return;
      stopped = true;
      Deno.removeSignalListener("SIGINT", stopOnSignal);
      Deno.removeSignalListener("SIGTERM", stopOnSignal);
      let routerError: unknown;
      if (routerInstalled) {
        try {
          await runGatewayCommand(
            input.gatewayBinary,
            gatewayRouterArgs({
              action: "remove_router",
              routePrefix,
              dataPort,
              gatewayControlUrl: input.gatewayControlUrl,
            }),
            input.systemRoot,
          );
        } catch (error) {
          routerError = error;
        }
      }
      await stopChild();
      if (!routerError) {
        try {
          const gatewayConfig = await runGatewayCommand(
            input.gatewayBinary,
            ["show", "--server", input.gatewayControlUrl],
            input.systemRoot,
          );
          if (gatewayConfig.includes(routePrefix)) {
            throw new Error(
              `temporary NDN route remains in gateway config: ${routePrefix}`,
            );
          }
        } catch (error) {
          routerError = error;
        }
      }
      await Deno.remove(tempRoot, { recursive: true });
      if (routerError) throw routerError;
    },
  };
  const stopOnSignal = (): void => {
    void service.stop().finally(() => Deno.exit(130));
  };
  Deno.addSignalListener("SIGINT", stopOnSignal);
  Deno.addSignalListener("SIGTERM", stopOnSignal);
  return service;
}
