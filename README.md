# BuckyOS Beta2 (0.6.0) Release!

Beta2 is a major BuckyOS update for the AI era. Key additions include:

- Two new kernel components: `kmsgqueue` + `kevent`, which together enable high-performance distributed event notification
- A complete BuckyOS Desktop WebUI
- A completed port of [OpenDAN](https://github.com/fiatrete/OpenDAN-Personal-AI-OS), reimplemented in Rust
  - Built-in Jarvis agent
  - Core UI-Session <-> WorkSession architecture
  - An Agent-Behavior Loop that supports certain behavior patterns more accurately than skills alone
  - Agent Tool redesigned around the Intent Engine, together with the necessary meta-tools
  - Upgraded Agent Memory using `set_memory` + `topic`, plus automatic memory query/compression and filesystem-based manual lookup by agents
  - Support for a TODO-list-based SubAgent system
  - A Runtime Sandbox with fully controlled isolation between agents
- A new AI Computer Center for unified cluster AI capability management and model routing
- A new Msg Center that provides unified Message Inbox/Outbox management for DID entities and serves as the foundation for the planned default apps Message Hub and Home Station
- Msg Center support for Msg Tunnel extensions, with full Telegram API support already implemented
- A new Workflow engine with Agent-Human-Loop support, serving as the foundation of the Agent Intent Engine (*currently under development)
- A complete refactor of the Named Store storage layer in ndn-lib
- A reimplemented repo-service, upgraded from an "app source" into a general-purpose digital content management and distribution infrastructure service
- Kernel development for CYFS (a distributed file system based on `cyfs://`) is complete and planned to be enabled in Beta3
- Multiple cyfs-gateway updates that expand Server configuration and further strengthen process-chain capabilities
- A rewritten BuckyOS cluster-routing process-chain that is more modular, supports richer gateway security, and protects system installation from the source
- Rtcp protocol security upgrades are in progress and are planned to be completed in the first two Beta2 iterations
- Support for virtual machine management, with VMs assignable to agents (*currently under development)
- Scheduler support for Function Instance, replacing the originally planned OPTask (*currently under development)
- The BuckyOS TypeScript SDK is becoming a first-class citizen and will gain feature parity with the Rust SDK (*in progress)
  - Developers can choose either TypeScript or Rust to build BuckyOS native apps
- Infrastructure for Harness Engineering has been added, and we will fully switch to an AI-native development workflow in this release

**Join us on this journey. Issues and pull requests are always welcome. Let’s build the next generation of distributed personal AI operating systems together.**

After the first Beta2 release, we will move into a rapid iteration phase, with the goal of shipping user-experience improvements every week.
On the kernel side, we are pushing toward "the first commercial-grade, Zero OP personal distributed private cloud," with current work focused on data reliability and system self-healing. That version is planned as Beta3 and is currently targeted for late April 2026.

## Getting Started

Get the active code first:
[https://github.com/buckyos/buckyos/discussions/70](https://github.com/buckyos/buckyos/discussions/70)

Installing from source is a good way to understand BuckyOS and the first step toward contributing. BuckyOS can be built on macOS, Linux, and Windows.

```bash
git clone https://github.com/buckyos/buckyos.git
```

After cloning, install `buckyos-devkit` first. It provides commands such as `buckyos-build` and `buckyos-install`. It is recommended to create and activate a virtual environment in the project directory before installing it:

```bash
cd buckyos
python3 -m venv venv
source venv/bin/activate
python3 -m pip install -U "buckyos-devkit @ git+https://github.com/buckyos/buckyos-devkit.git"
```

Before building, you can refer to `devenv.py` to prepare the environment. The main dependencies are the Rust toolchain, Node.js + pnpm, Python 3.12, and `docker.io`. Once those are ready, use the following steps.

### Step 1. Build cyfs-gateway

BuckyOS currently depends on cyfs-gateway, so you need to build cyfs-gateway from source before running BuckyOS:

```bash
cd ~/
git clone https://github.com/buckyos/cyfs-gateway.git
cd cyfs-gateway/src
buckyos-build
buckyos-install --all
```

### Step 2. Build buckyos

Return to the BuckyOS repository and run:

```bash
cd buckyos/src
buckyos-build
```

### Step 3. Start buckyos

For the first installation:

```bash
python3 start.py --reinstall release
```

A source install does not automatically register BuckyOS as a startup service. For later manual starts, run:

```bash
python3 start.py
```

**Important: do not run `python3 start.py --reinstall release` again after the initial setup. It will soft-reset your system.**

`start.py` ultimately runs the command below. You can add it to your system startup service list manually:

```bash
sudo /opt/buckyos/bin/node-daemon/node_daemon --enable_active
```

#### Common pitfalls and troubleshooting during the transition period

- **You may need `cargo update` frequently**: especially in a fresh environment or when the lockfile has drifted.
- **`make_config.py` depends on `buckycli`**: in practice, you usually need to run `buckyos-build && buckyos-install` in the buckyos repo first to make sure `buckycli` is available.

### Common scripts in the source tree

- Build only the Rust parts:

```bash
cd src
python3 build.py --no-build-web-apps
```

- Update only the compiled artifacts and then start `/opt/buckyos`:

```bash
cd src
python3 start.py
```

- Reinstall BuckyOS using a specified config group:

```bash
cd src
python3 start.py --reinstall $group_name
```

If `group_name` is empty, BuckyOS starts with an empty config and enters the pending activation state.

The system currently includes several commonly used config groups:

- `release` (production use, backed by buckyos.ai SN infrastructure)
- `dev` (development config without SN and without dependencies on off-machine components)
- `alice.ood1`, `bob.ood1`, `charlie.ood1` (three preset identities intended for the planned virtual test environment at `devtests.org`)
- `sn` (the SN node config for the virtual test environment)

## BuckyOS Vision

- `Internet is BuckyOS`: Build a new dApp ecosystem on top of decentralized, and therefore necessarily open-source, infrastructure. Applications become more interconnected, more modular, and more AI-friendly. This should support applications an order of magnitude more complex than today’s while reducing both development and operational costs by an order of magnitude as well. (A 100x productivity gain.)
- The internet’s infrastructure should not be controlled by corporations. Decentralized infrastructure can eliminate platform taxes and unfair platform rules. Through token-based ownership, the base platform can be jointly owned by developers, evangelists, users, and capital, sharing revenue and agreeing on fairer platform rules together.
- The core logic behind `kill app` is "using LLMs to solve the scarcity of information filtering." Using AI to generate information is a need for a minority; using AI to filter information is a need for everyone. AI can apply common sense to help users filter the information they receive and address today’s echo-chamber problem. That has obvious value for users and a positive social impact. For the AI industry, the semantic network formed by linking every user’s KnowledgeBase through CYFS can also help LLMs produce better results on top of real-time, accurate information.

### Learn More About BuckyOS

- BuckyOS Architecture Design (Coming Soon)
- Hello BuckyOS! (Coming Soon)
- BuckyOS dApp Developer Manual (Coming Soon)
- BuckyOS Contributor Guide (Coming Soon)

## The Next Generation of GPL: A New Open Source Collaboration Model

"Open source organizations have a long history and remarkable achievements. Practice has shown that better code can be written purely through collaboration in the virtual world. We believe software development is especially well suited to the DAO model. We call this kind of DAO, where decentralized organizations collaboratively develop software, SourceDAO." — from the CodeDAO White Paper ([https://www.codedao.ai](https://www.codedao.ai))

The BuckyOS open-source community operates as a DAO. Our goal is to solve the problem of open-source contributors giving without reward, or simply being exploited:

- Code mining: improve release quality through aligned incentives
- A GPL-like viral mechanism: create a shared-interest structure across upstream and downstream participants
- Automatic revenue sharing through smart contracts: contributors to foundational libraries that keep the world running should receive stable, long-term income because they have earned it

From a governance perspective, unified token ownership and aligned interests help users and developers reach rational decisions under a shared consensus. Even arguments remain arguments within the same community.

`Open, transparent, free to join and leave, and result-oriented`

SourceDAO is the open-source DAO smart contract built on these ideas. For more details, visit [https://dao.buckyos.org/](https://dao.buckyos.org/).

## Preliminary Version Plan

#### 2024

- **0.1 Demo:** 2.5% (Completed in June 2024)
- **0.2 PoC:** 2.5% (Completed in September 2024)
- **0.3 Alpha1:** 2.5% (Completed in December 2024)

#### 2025

- **0.4 Alpha2:** 2.5% (Completed in March 2025)
- **0.4.1 Alpha3:** 2.5% (Completed in September 2025)
- **0.5.1 Beta1:** 4% (Completed in December 2025)

#### 2026

- **0.6.0 Beta2:** 2.5% (This release in Q1 2026; iterative development is ongoing)
- **0.7.0 Beta3:** 2.5% (Planned for late April 2026)

## License

BuckyOS is a free, open-source, decentralized system. We encourage vendors to build commercial products on top of BuckyOS and promote fair competition. Our licensing choices are designed to create a win-win ecosystem, preserve the decentralized core, protect contributors, and support a sustainable long-term ecosystem.

We use a dual-license model. One side is a traditional LGPL-based license that requires kernel modifications to follow GPL terms. Closed-source applications are allowed, but they cannot become core system components. The other side is a SourceDAO-based license. When an organization that issues DAO tokens uses BuckyOS, it must donate a portion of those tokens to the BuckyOS DAO under that license.

There is still no existing license that fully matches our needs, so during the DEMO phase we are temporarily using the BSD license. I believe that once the PoC is complete, we will be ready with the formal license.
