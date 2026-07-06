import * as os from "node:os";
import * as path from "node:path";

// devenv_config.ts - shared user-data seed description for dev provisioning.
//
// 种子案例书，与 cyfs-gateway 仓库 src/devenv_config.ts 同源同步：SN（cyfs-gateway
// 的 make_sn_config.ts）是该种子的消费方之一，本仓库的 make_config.ts 是另一个。
// 两仓各持一份副本，修改任意一份时必须同步另一份（本注释即同步义务的锚点）。
//
// This file is not generated runtime config. It is the compact case book of the
// real user facts that the make_*_config scripts materialize through the public
// Web SDK before services start: username, zone DID/name, owner device name,
// network shape, SN/Web3 bridge placement, trust roots, and dev CA identity.
//
// make_config.ts (this repo) uses these seed facts to write the OOD/rootfs-owned
// local user and device environment. make_sn_config.ts (cyfs-gateway) uses the same
// facts to write the SN/web3-gateway-owned seed view. Keeping both scripts
// pointed at one seed description makes the module boundary visible: this file
// describes the minimal truth source, while each owning module lazily constructs
// its derived configs, indexes, caches, and private databases from that truth.
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

### ENV 实例
我们分3个环境
- devenv
- nightly
- release

## system users
固定的system user,通常用来做一些系统配置（比如appdoc）的owenr

- did:bns:buckyos (做owner都是用这个)

下面几个目前没用到，但保留
- did:web:buckyos.ai
- did:web:buckyos.org
- did:web:buckyos.com
- did:web:buckyos.io


## user(owenr)
使用did:bns:user 标识用户,使用两个密钥对(ed25519 + evm)来拥有权限
注意SN的注册用户alice，必定拥有did:bns:alice,但did:bns:alice可以不是SN注册用户

> 用户密钥对使用固定助记词构造，非随机
> 需要签名的document现场构建并使用上述密钥对签名
### user configs
- 构造owner-document,并用evm key上链
- 在sn上注册账号

### zone configs

用OwnerKey构造4个JWT： ZoneDocument with (ZoneBootDocument,Gateway-Doc,Gateway-DeviceMiniDoc)
用evm可以执行必要的publishDocument

- did:bns:$zoneid 需要构造zone-document并执行publishDocument
- did:web:$zoneid 需要使用sn的user-domian机制保存ZoneDocument

acme流程直接走自签证书（非标准流程,标准环境下还需要调用sn的add_dns_txt_record或publishDocument(dns_txt_record)
注意sn的add_dns_txt_record对通过sn注册的did:bns:xxx用户也有效（代发tx),其目的是让sn用户无手续费操作链


## 关键主机

SN-Node: 测试环境中的VM，都需要把默认DNS服务器设置到该主机
如果当前开发机也参加测试，但不方便设置默认DNS服务器，则需要设置下面Host文件

$SN-Node.ip dns.devtests.org
$SN-Node.ip sn.devtests.org
$SN-Node.ip bns.devtests.org
$SN-Node.ip web3.devtests.org

# 下面涉及到具体用户，按需配置即可
$SN-Node.ip alice.web3.devtests.org
$SN-Node.ip public.alice.web3.devtests.org

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
  /** false = 纯 Web3 用户（只在 BNS 上链，不建 sn_user 账号）。缺省 true。 */
  sn_account?: boolean;
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
  // 纯 Web3 用户位（seed-v2）：did:bns:dave 只在 BNS 上链，不建 sn_user 账号
  // （上方注释"did:bns:alice 可以不是 SN 注册用户"的测试实例）。用于验证
  // lazy-init 的"解除 sn_user 前提"——纯钱包用户不注册 SN 也能被解析。
  // make_sn_config.ts getSeedUserSpecs() 将该组标记为 snAccount=false。
  "dave.ood1": {
    username: "dave",
    zone_id: "dave.bns.did",
    node_name: "ood1",
    netid: "wan",
    rtcp_port: 2980,
    sn_base_host: "devtests.org",
    web3_bridge: "web3.devtests.org",
    trust_did: DEFAULT_TRUST_DID,
    force_https: false,
    ca_name: "buckyos_test_ca",
    sn_account: false,
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
