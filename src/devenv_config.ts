import * as os from "node:os";
import * as path from "node:path";

// devenv_config.ts - shared user-data seed description for dev provisioning.
//
// This file is not generated runtime config. It is the compact case book of the
// real user facts that the make_*_config scripts materialize through the public
// Web SDK before services start: username, zone DID/name, owner device name,
// network shape, SN/Web3 bridge placement, trust roots, and dev CA identity.
//
// make_config.ts uses these seed facts to write the OOD/rootfs-owned local user
// and device environment. make_sn_configs.ts uses the same facts to write the
// SN/web3-gateway-owned registration view. Keeping both scripts pointed at one
// seed description makes the module boundary visible: this file describes the
// minimal truth source, while each owning module lazily constructs its derived
// configs, indexes, caches, and private databases from that truth.
//
// Iteration direction:
// Keep this file focused on durable seed facts that a third-party tool or agent
// could know and write via the Web SDK. Do not add service-private generated
// output, compatibility caches, or data duplicated from another module's derived
// state. If a boot path needs more data than this seed can reasonably describe,
// prefer improving that module's lazy/seed initialization boundary.

/*
对seed配置的整理 
先说明配置类型，再我们定义了哪些实例（这些实例不一定再devenv config中）

## ENV

列出运行环境的"关键区别" ，并以此推导出系统的一些关键的密钥

- bns链用哪个？
- web3_bridge的配置（决定了 did:bns:alice 如何转换成浏览器可访问的hostname)
- 

<TODO>

### ENV 实例
我们分3个环境
- devenv 
- nightly
- release

## system users
固定的system user,通常用来做一些系统配置（比如appdoc）的owenr

- did:bns:buckyos (做owner都是用这个)

### 在测试环境里的实例
- sn.devtests.org
- did:web:buckyos.ai
- did:web:buckyos.org
- did:web:buckyos.com
- did:web:buckyos.io


## user(owenr)
使用did:bns:user 标识用户,使用两个密钥对(ed25519 + evm)来拥有权限

> 用户密钥对使用固定助记词构造，非随机
> 需要签名的document现场构建并使用上述密钥对签名

- user configs
- zone configs
- device(ood)-configs

## 关键主机


*/

export interface OODGroupParams {
  username: string;
  zone_id: string;
  node_name: string;
  netid: string;
  rtcp_port: number;
  sn_base_host: string;
  web3_bridge: string;
  trust_did: string[];
  force_https: boolean;
  ca_name: string;
}

// Historical location for dev user envs and the dev CA material used by these
// seed cases. It is a provisioning workspace path, not a BuckyOS runtime root.
export const ENV_ROOT_DIR = path.join(os.homedir(), "buckycli");

export const DEFAULT_TRUST_DID = [
  "did:web:buckyos.org",
  "did:web:buckyos.ai",
  "did:web:buckyos.io",
];

const DEV_GROUP_PARAMS: OODGroupParams = {
  username: "devtest",
  zone_id: "test.buckyos.io",
  node_name: "ood1",
  netid: "wan",
  rtcp_port: 2980,
  sn_base_host: "",
  web3_bridge: "web3.devtests.org",
  trust_did: DEFAULT_TRUST_DID,
  force_https: false,
  ca_name: "buckyos_test_ca",
};

const DEVTESTS_OOD1_GROUP_PARAMS: OODGroupParams = {
  username: "devtests",
  zone_id: "devtests.org",
  node_name: "ood1",
  netid: "wan",
  rtcp_port: 2980,
  sn_base_host: "",
  web3_bridge: "web3.devtests.org",
  trust_did: DEFAULT_TRUST_DID,
  force_https: false,
  ca_name: "buckyos_test_ca",
};

export const OOD_GROUPS: Record<string, OODGroupParams> = {
  dev: DEV_GROUP_PARAMS,
  "alice.ood1": {
    username: "alice",
    zone_id: "alice.bns.did",
    node_name: "ood1",
    netid: "lan",
    rtcp_port: 2980,
    sn_base_host: "devtests.org",
    web3_bridge: "web3.devtests.org",
    trust_did: DEFAULT_TRUST_DID,
    force_https: false,
    ca_name: "buckyos_test_ca",
  },
  "bob.ood1": {
    username: "bob",
    zone_id: "bob.bns.did",
    node_name: "ood1",
    netid: "wan_dyn",
    rtcp_port: 2980,
    sn_base_host: "devtests.org",
    web3_bridge: "web3.devtests.org",
    trust_did: DEFAULT_TRUST_DID,
    force_https: false,
    ca_name: "buckyos_test_ca",
  },
  "charlie.ood1": {
    username: "charlie",
    zone_id: "charlie.me",
    node_name: "ood1",
    netid: "portmap",
    rtcp_port: 2981,
    sn_base_host: "devtests.org",
    web3_bridge: "web3.devtests.org",
    trust_did: DEFAULT_TRUST_DID,
    force_https: false,
    ca_name: "buckyos_test_ca",
  },
  devtests_ood1: DEVTESTS_OOD1_GROUP_PARAMS,
  devtest_ood1: DEV_GROUP_PARAMS,
  sn_web: DEVTESTS_OOD1_GROUP_PARAMS,
};

export function getParamsFromGroupName(groupName: string): OODGroupParams {
  const params = OOD_GROUPS[groupName];
  if (!params) {
    throw new Error(`invalid group name: ${groupName}`);
  }
  return params;
}
