import { copyIdentityOutputs, makeConfigByGroupName, prepareSeedIdentity } from "./make_config.ts";
import { getParamsFromGroupName } from "./devenv_config.ts";
import { hashAdminPassword } from "buckyos/provision";

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
      for (const forbidden of [env, `${target}/etc/node_identity.json`, `${target}/security/start_config.json`, `${target}/etc/start_config.json`]) {
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
    if (params.params.device_did !== "did:bns:unactivated.local" ||
        params.params.rtcp_port !== "2980") {
      throw new Error("vmtest preseeded a device identity");
    }
  } finally {
    await Deno.remove(root, { recursive: true });
  }
});

Deno.test("generated runtime startup config and CA keys stay in security", async () => {
  const root = await Deno.makeTempDir();
  try {
    const target = `${root}/target`;
    const caDir = `${root}/ca`;
    await makeConfigByGroupName("alice.ood1", target, caDir, `${root}/env`);
    const params = getParamsFromGroupName("alice.ood1");
    const sourceKey = await Deno.readTextFile(`${caDir}/${params.ca_name}_ca_key.pem`);
    const copiedKey = await Deno.readTextFile(`${target}/security/ca_key.pem`);
    if (copiedKey !== sourceKey || !copiedKey.includes("PRIVATE KEY")) {
      throw new Error("runtime CA key does not match the generated CA");
    }
    await Deno.stat(`${target}/etc/ca.cert`);
    for (const filename of ["start_config.json", "ca_key.pem"]) {
      if (Deno.build.os !== "windows") {
        const file = await Deno.stat(`${target}/security/${filename}`);
        if (file.mode === null || (file.mode & 0o777) !== 0o600) {
          throw new Error(`${filename} must be private`);
        }
      }
      try {
        await Deno.stat(`${target}/etc/${filename}`);
        throw new Error(`${filename} must not be published through etc`);
      } catch (error) {
        if (!(error instanceof Deno.errors.NotFound)) throw error;
      }
    }
  } finally {
    await Deno.remove(root, { recursive: true });
  }
});

Deno.test("dev bootstrap secrets are merged into a private template", async () => {
  const root = await Deno.makeTempDir({ dir: Deno.build.os === "linux" ? "/tmp" : undefined });
  try {
    const target = `${root}/target`;
    const publicTemplatePath = `${target}/etc/scheduler/boot.template.toml`;
    const privateTemplatePath = `${target}/security/scheduler/boot.template.toml`;
    const publicTemplate = '"public/bootstrap" = """\npublic-base-value\n"""\n';
    await Deno.mkdir(`${target}/etc/scheduler`, { recursive: true });
    await Deno.writeTextFile(publicTemplatePath, publicTemplate);
    await makeConfigByGroupName("devtest_ood1", target, `${root}/ca`, `${root}/env`);
    if (await Deno.readTextFile(publicTemplatePath) !== publicTemplate) {
      throw new Error("dev bootstrap changed the public base template");
    }
    const privateTemplate = await Deno.readTextFile(privateTemplatePath);
    for (const required of ["public-base-value", '"security/bob/key"', '"security/alice/key"', "PRIVATE KEY"]) {
      if (!privateTemplate.includes(required)) {
        throw new Error(`private bootstrap is missing ${required}`);
      }
    }
    const nodeSettingsEntry = privateTemplate.match(/"nodes\/ood1\/settings"\s*=\s*"""\n([\s\S]*?)\n"""/);
    if (Deno.build.os === "linux") {
      if (!nodeSettingsEntry) throw new Error("dev bootstrap did not offer the host runtime");
      const settings = JSON.parse(nodeSettingsEntry[1]);
      const policy = settings.executor.owner_agents;
      const workspace = `${target}/data/home/devtest/host-runtime`;
      const expectedUid = Deno.uid() === 0 ? 65534 : Deno.uid();
      const expectedGid = Deno.uid() === 0 ? 65534 : Deno.gid();
      if (expectedUid === null || expectedGid === null ||
        !settings.executor.enabled || !policy.enabled || policy.computer !== "none" ||
        JSON.stringify(policy.workdir_roots) !== JSON.stringify([workspace]) ||
        policy.run_as.uid !== expectedUid || policy.run_as.gid !== expectedGid) {
        throw new Error("dev host policy must grant only its workspace with a non-root execution identity");
      }
      const directory = await Deno.lstat(workspace);
      if (!directory.isDirectory || directory.uid !== expectedUid || directory.gid !== expectedGid ||
        directory.mode === null || (directory.mode & 0o007) !== 0) {
        throw new Error("dev host workspace has incorrect ownership or permissions");
      }
      await Deno.chmod(root, 0o755);
      const probe = await new Deno.Command("/bin/sh", {
        args: ["-c", 'printf %s host-runtime-write > "$1/probe"', "host-runtime-probe", workspace],
        uid: expectedUid,
        gid: expectedGid,
        stdout: "piped",
        stderr: "piped",
      }).output();
      if (!probe.success) {
        throw new Error(`dev host identity cannot write its workspace: ${new TextDecoder().decode(probe.stderr)}`);
      }
      const probePath = `${workspace}/probe`;
      if (await Deno.readTextFile(probePath) !== "host-runtime-write" ||
        (await Deno.stat(probePath)).uid !== expectedUid) {
        throw new Error("dev host workspace write did not use the configured identity");
      }
    } else if (nodeSettingsEntry) {
      throw new Error("Linux system executor defaults leaked into another platform");
    }
    if (Deno.build.os !== "windows") {
      const file = await Deno.stat(privateTemplatePath);
      if (file.mode === null || (file.mode & 0o777) !== 0o600) {
        throw new Error("private bootstrap template must be private");
      }
    }
  } finally {
    await Deno.remove(root, { recursive: true });
  }
});

Deno.test("dev bootstrap preserves an explicitly disabled node executor", async () => {
  const root = await Deno.makeTempDir();
  try {
    const target = `${root}/target`;
    const settings = { executor: { enabled: false, owner_agents: { enabled: false, workdir_roots: [] } } };
    const entry = `"nodes/ood1/settings" = """\n${JSON.stringify(settings)}\n"""\n`;
    await Deno.mkdir(`${target}/etc/scheduler`, { recursive: true });
    await Deno.writeTextFile(`${target}/etc/scheduler/boot.template.toml`, entry);
    await makeConfigByGroupName("dev", target, `${root}/ca`, `${root}/env`);
    const template = await Deno.readTextFile(`${target}/security/scheduler/boot.template.toml`);
    const matches = [...template.matchAll(/"nodes\/ood1\/settings"\s*=\s*"""\n([\s\S]*?)\n"""/g)];
    if (matches.length !== 1 || JSON.stringify(JSON.parse(matches[0][1])) !== JSON.stringify(settings)) {
      throw new Error("dev bootstrap overwrote an explicit node executor policy");
    }
    try {
      await Deno.stat(`${target}/data/home/devtest/host-runtime`);
      throw new Error("disabled node executor created a host runtime workspace");
    } catch (error) {
      if (!(error instanceof Deno.errors.NotFound)) throw error;
    }
  } finally {
    await Deno.remove(root, { recursive: true });
  }
});

Deno.test("seed materialization validates owner passwords and preserves the device RTCP port", async () => {
  const root = await Deno.makeTempDir();
  try {
    for (const group of ["alice.ood1", "bob.ood1", "charlie.ood1"]) {
      const params = getParamsFromGroupName(group);
      const envRoot = `${root}/env`;
      const userDir = `${envRoot}/${params.zone_id}`;
      const nodeDir = `${userDir}/${params.node_name}`;
      const target = `${root}/${group}`;
      await prepareSeedIdentity(group, envRoot);
      const startPath = `${nodeDir}/start_config.json`;
      const start = JSON.parse(await Deno.readTextFile(startPath));
      const expectedHash = hashAdminPassword(params.username, "bucky2025");
      start.admin_password_hash = hashAdminPassword("devtest", "bucky2025");
      await Deno.writeTextFile(startPath, JSON.stringify(start));
      let rejected = false;
      try {
        copyIdentityOutputs(userDir, nodeDir, target, params);
      } catch (error) {
        if (!(error instanceof Error) || !error.message.includes("stale admin_password_hash") ||
            !error.message.includes("new --env_root")) {
          throw error;
        }
        rejected = true;
      }
      if (!rejected) throw new Error(`accepted stale owner password for ${group}`);
      try {
        await Deno.stat(target);
        throw new Error("stale seed created target state");
      } catch (error) {
        if (!(error instanceof Deno.errors.NotFound)) throw error;
      }
      start.admin_password_hash = expectedHash;
      await Deno.writeTextFile(startPath, JSON.stringify(start));
      copyIdentityOutputs(userDir, nodeDir, target, params);
      const saved = JSON.parse(await Deno.readTextFile(`${target}/security/start_config.json`));
      try {
        await Deno.stat(`${target}/etc/start_config.json`);
        throw new Error("private startup config was written to etc");
      } catch (error) {
        if (!(error instanceof Deno.errors.NotFound)) throw error;
      }
      const gateway = JSON.parse(await Deno.readTextFile(`${target}/etc/node_gateway_params.json`));
      if (saved.admin_password_hash !== expectedHash ||
          gateway.params.rtcp_port !== String(params.rtcp_port)) {
        throw new Error(`incorrect materialized password or RTCP port for ${group}`);
      }
      const claims = (jwt: string) => JSON.parse(atob(jwt.split('.')[1].replaceAll('-', '+').replaceAll('_', '/')));
      const boot = claims(saved.boot_config_jwt);
      const zone = claims(saved.zone_document_jwt);
      const device = claims(saved.device_doc_jwt);
      if (!device.ddns_sn_url) throw new Error(`missing DDNS endpoint for ${group}`);
      if (params.netid === "wan_dyn") {
        if (boot.sn || zone.sn) throw new Error("wan_dyn must not require an SN relay");
      } else if (!boot.sn || !zone.sn) {
        throw new Error(`missing relay candidate for ${group}`);
      }
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
