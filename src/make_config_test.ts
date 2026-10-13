import { copyIdentityOutputs, makeConfigByGroupName, prepareSeedIdentity } from "./make_config.ts";
import { getParamsFromGroupName } from "./devenv_config.ts";

Deno.test("release and nightly remain unactivated without a seed identity handoff", async () => {
  const root = await Deno.makeTempDir();
  try {
    for (const group of ["release", "nightly"]) {
      const target = `${root}/${group}`;
      const env = `${root}/env`;
      await makeConfigByGroupName(group, target, undefined, env);
      const machine = JSON.parse(await Deno.readTextFile(`${target}/etc/machine.json`));
      const domain = group === "release" ? "buckyos.ai" : "buckyos.io";
      if (machine.bns_host !== `bns.${domain}` ||
        machine.web3_bridge.bns !== `web3.${domain}` || !machine.force_https) {
        throw new Error("formal environment machine config changed");
      }
      for (const forbidden of [env, `${target}/etc/node_identity.json`, `${target}/etc/start_config.json`]) {
        try {
          await Deno.stat(forbidden);
        } catch (error) {
          if (error instanceof Deno.errors.NotFound) {
            continue;
          }
          throw error;
        }
        throw new Error(`unactivated mode created identity state: ${forbidden}`);
      }
    }
  } finally {
    await Deno.remove(root, { recursive: true });
  }
});

Deno.test("vmtest remains unactivated with an isolated environment root", async () => {
  const root = await Deno.makeTempDir();
  try {
    await makeConfigByGroupName("vmtest", root, `${root}/ca`, `${root}/env`);
    const params = JSON.parse(await Deno.readTextFile(`${root}/etc/node_gateway_params.json`));
    if (params.params.device_did !== "did:bns:unactivated.local") {
      throw new Error("vmtest preseeded a device identity");
    }
  } finally {
    await Deno.remove(root, { recursive: true });
  }
});

Deno.test("finalized identity rejects tampering, changed inputs and parallel initialization", async () => {
  const root = await Deno.makeTempDir();
  try {
    const envRoot = `${root}/env`;
    const userDir = `${envRoot}/alice.bns.did`;
    const nodeDir = `${userDir}/ood1`;
    const bundlePath = `${nodeDir}/sn_seed_identity.json`;
    await prepareSeedIdentity("alice.ood1", envRoot);
    const originalBundle = await Deno.readTextFile(bundlePath);
    const ownerPath = `${userDir}/user_config.json`;
    const originalOwner = await Deno.readTextFile(ownerPath);
    const target = `${root}/target`;
    await Deno.mkdir(`${target}/etc`, { recursive: true });
    const machinePath = `${target}/etc/machine.json`;
    await Deno.writeTextFile(machinePath, "preserve installed machine config");
    const expectFailure = async (action: () => unknown, message: string) => {
      try {
        await action();
      } catch (error) {
        if (error instanceof Error && error.message.includes(message)) {
          return;
        }
        throw error;
      }
      throw new Error(`expected failure: ${message}`);
    };

    const bundle = JSON.parse(originalBundle);
    bundle.device_doc_jwt = "tampered";
    await Deno.writeTextFile(bundlePath, JSON.stringify(bundle));
    await expectFailure(
      () => makeConfigByGroupName("alice.ood1", target, `${root}/ca`, envRoot),
      "differs from finalized identity",
    );
    if (await Deno.readTextFile(machinePath) !== "preserve installed machine config" ||
        await Deno.readTextFile(ownerPath) !== originalOwner ||
        await Deno.readTextFile(bundlePath) !== JSON.stringify(bundle)) {
      throw new Error("OOD generation changed state before rejecting tampered identity");
    }
    await Deno.writeTextFile(bundlePath, originalBundle);
    await expectFailure(
      () => copyIdentityOutputs(userDir, nodeDir, target, {
        ...getParamsFromGroupName("alice.ood1"), rtcp_port: 9999,
      }),
      "differs from finalized identity",
    );

    const zonePath = `${userDir}/zone_config.json`;
    const originalZone = await Deno.readTextFile(zonePath);
    const zone = JSON.parse(originalZone);
    zone.iat += 1;
    await Deno.writeTextFile(zonePath, JSON.stringify(zone));
    await expectFailure(() => prepareSeedIdentity("alice.ood1", envRoot), "is stale");
    if (await Deno.readTextFile(zonePath) !== JSON.stringify(zone)) {
      throw new Error("stale identity was silently regenerated");
    }
    await Deno.writeTextFile(zonePath, originalZone);

    const deviceKeyPath = `${nodeDir}/security/ood1.alice.bns.did/authentication.private.pem`;
    const deviceKey = await Deno.readTextFile(deviceKeyPath);
    await Deno.writeTextFile(deviceKeyPath, await Deno.readTextFile(`${userDir}/user_private_key.pem`));
    await expectFailure(
      () => prepareSeedIdentity("alice.ood1", envRoot),
      "differs from device document key",
    );
    await Deno.writeTextFile(deviceKeyPath, deviceKey);

    const lockPath = `${userDir}/.sn_seed_identity.lock`;
    await Deno.writeTextFile(lockPath, "another generation owns this lock");
    await expectFailure(() => prepareSeedIdentity("alice.ood1", envRoot), "EEXIST");
    if (await Deno.readTextFile(lockPath) !== "another generation owns this lock") {
      throw new Error("another generation's lock was stolen");
    }
    await Deno.remove(lockPath);
    await prepareSeedIdentity("alice.ood1", envRoot);
    if (await Deno.readTextFile(bundlePath) !== originalBundle) {
      throw new Error("valid repeated preparation changed the finalized bundle");
    }
  } finally {
    await Deno.remove(root, { recursive: true });
  }
});
