# BuckyOS Beta2.2 (0.7.0)

**English** | [简体中文](README_zhCN.md)

BuckyOS is an open-source personal AI operating system. It brings your devices, applications, data, and AI agents together in a **Zone**: a personal cloud under your control.

The current source tree targets **Beta2.2 / 0.7.0**, with the official launch planned for **October 15, 2026**. This is a breaking-change release: system configuration, identity, application packaging, and agent configuration have changed. Backward compatibility with earlier betas is not guaranteed.

## What's in Beta2.2

- **Web Desktop and Control Panel**: a shared desktop with application windows, Settings, Users & Agents, AI Center, Task Center, and application management.
- **FileBrowser and Preview**: built-in file browsing and preview interfaces. FileBrowser connects to `nfs-server` for file operations; background copy jobs use Task Manager.
- **MessageHub and Message Center**: a built-in messaging app backed by `msg-center`, with conversation history, attachments, read state, and session archive/restore/delete. The service provides DID-based mailboxes, contacts, self-hosted groups, native messaging, and a Telegram tunnel.
- **OpenDAN and Jarvis**: the Rust agent runtime supports configurable session classes and behavior loops, memory and workspace tools, message/event routing, delegated tasks, and requests for human input. Jarvis is packaged as a versioned `.pikg` application; its prompts, behaviors, and translations live in [`src/apps/jarvis_runtime/agent`](src/apps/jarvis_runtime/agent).
- **AI Compute Center (AICC)**: provider and model management, logical model routing, usage logging, and adapters for text and media capabilities. AI Center exposes these controls in the desktop.
- **Workflow and Task Manager**: workflow definitions and runs, scheduled tasks, task progress/control, and human approval or intervention. Workflow and OpenDAN integrate with Task Manager and `kevent` to exchange task updates.
- **Application delivery and SDK/CLI**: `.pikg` packages, validated installation plans, per-user application instances, and gateway routing. The TypeScript SDK and `buckyos` CLI are built together and bundled into the system; Rust services use `buckyos-api`.
- **System infrastructure**: `system-config`, the scheduler, and `node-daemon` manage desired state and deployment; `verify-hub` and RBAC handle login and authorization. `kmsg` and `kevent` provide messaging and event notification, while `repo-service` and named-object storage support content delivery.
- **Development workflow**: platform build/packaging pipelines, local development verification (DV) tests, and contributor guidance under [`harness/`](harness/README.md).

### Current boundaries and ongoing work

Beta2.2 is still being refined. The presence of a service or UI does not mean every planned capability is complete:

- Workflow's running service currently keeps definitions, runs, and its object store in memory. Schedule definitions are mirrored to Task Manager, but durable workflow recovery and the `func::*` path into the scheduler are still pending.
- The scheduler has FunctionObject/Thunk support, but its core still contains OPTask. The planned replacement with Function Instance is not complete.
- Telegram is the external message tunnel implemented in this repository. Lark and other external channels remain future work.


**Issues and pull requests are welcome. Help us build the next generation of personal AI operating systems.**

## Getting Started

The source build supports macOS, Linux, and Windows. The commands below use a macOS/Linux shell. BuckyOS Desktop is the Mac/Windows desktop distribution; Linux builds target servers and development environments.

### Step 1. Get the sources and prepare the environment

Clone these repositories into the same parent directory, using the `main` branch:

```bash
git clone --branch main https://github.com/buckyos/buckyos.git
git clone --branch main https://github.com/buckyos/cyfs-gateway.git
cd buckyos
```

To develop the SDK/CLI locally, also clone `buckyos-websdk` alongside these repositories with `git clone --branch main https://github.com/buckyos/buckyos-websdk.git`. This checkout is optional; without it, the build uses the published npm package `buckyos@latest`.

The build needs a stable Rust toolchain and platform C/C++ build tools, Python 3.12+, `uv`, Node.js with npm and pnpm, and Deno. Docker is needed for container applications; tmux is used by development tools. The current CI uses Node.js 24, pnpm 10.13.1, and Deno 2.9.2.

To bootstrap the platform dependencies, inspect and run [`devenv.py`](devenv.py):

```bash
python3 devenv.py
```

The root `pyproject.toml` lets `uv run` resolve `buckyos-devkit` automatically. A separate manually created virtual environment is not required.

### Step 2. Build and install cyfs-gateway

From the `buckyos` repository root:

```bash
cd ../cyfs-gateway/src
uvx --from "buckyos-devkit @ git+https://github.com/buckyos/buckyos-devkit.git@main" buckyos-build
uvx --from "buckyos-devkit @ git+https://github.com/buckyos/buckyos-devkit.git@main" buckyos-install --all
cd ../../buckyos/src
```

### Step 3. Build BuckyOS

From `buckyos/src`:

```bash
uv run buckyos-build.py
```

The wrapper first prepares the SDK/CLI with a bundled Deno runtime, then invokes the devkit build. If the sibling `buckyos-websdk` checkout exists, it builds from that source; otherwise, it downloads the published npm package `buckyos@latest`. Set `BUCKYOS_SDK_TOOL_SOURCE` if your SDK checkout is elsewhere. This preparation also runs with `--skip-web` or `-s <module>`. The published-package path requires npm and Deno but no SDK checkout or pnpm; local source builds also require pnpm. A failed local build stops the build rather than falling back to the published package.

Build output is assembled under `src/rootfs`. **`buckyos-build.py` does not update the installed runtime.** Use `start.py` to copy the latest artifacts into the installation and restart it. For the prebuilt SDK/CLI inputs used by release builds, see [`src/readme.md`](src/readme.md).

### Step 4. Initialize and start a Zone

Choose one initialization mode below. Both commands run from `buckyos/src`.

**Local development:**

```bash
uv run start.py --all
uv run check.py
```

`--all` selects the `dev` configuration: Owner `devtest`, Zone `test.buckyos.io`, and a preconfigured identity, without manual activation or an SN relay. Open `http://test.buckyos.io` on the development machine, with that domain and its app subdomains resolving to the local runtime. For application development and the test login, see the [app development guide](doc/sdk/app-dev-quickstart.md).

**First installation with interactive activation:**

```bash
uv run start.py --reinstall release
uv run check.py
```

This prepares an unactivated system using the `buckyos.ai` environment. Open `http://127.0.0.1:3182` to activate it. The `nightly` group uses the `buckyos.io` environment instead.

**Use `--all` and `--reinstall` only when deliberately reinitializing a system.** They reset configuration and runtime state. The installation layout preserves `data/home`, `data/srv`, and `storage`, so reinitialization is not a complete data wipe. Back up an existing installation before changing beta versions or resetting it.

For normal updates and restarts:

```bash
uv run start.py
```

`start.py` stops known BuckyOS processes, updates installed artifacts, and starts `node_daemon --enable_active` in the background. The runtime root defaults to `/opt/buckyos` on macOS/Linux or `%APPDATA%\buckyos` on Windows; `BUCKYOS_ROOT` overrides it. Ensure the current user has the required runtime-directory and port permissions. Source startup does not register a host startup service.

If an existing runtime is managed by systemd, launchd, or a Windows keepalive service, stop it through that service manager before installing or updating artifacts. `stop.py` alone does not disable automatic restarts. Keep source development and BuckyOS Desktop testing in separate environments to avoid competing for the same runtime, services, and ports.

### Common development commands

Run these from `buckyos/src`:

| Purpose | Command |
| --- | --- |
| Build the system and Web UI | `uv run buckyos-build.py` |
| Skip Web UI builds (SDK/CLI still builds) | `uv run buckyos-build.py --skip-web` |
| Build a specific module | `uv run buckyos-build.py -s <module>` |
| Update installed artifacts and restart | `uv run start.py` |
| Restart without updating artifacts | `uv run start.py --skip-update` |
| Inspect activation and runtime health | `uv run check.py` |
| Stop local processes | `uv run stop.py` |
| Debug Jarvis in the foreground | `./debug_jarvis.sh` |
| Run Rust unit tests | `cargo test` |

To discover and run DV tests, use the repository root after starting the required development environment:

```bash
uv run src/check.py
uv run test/run.py --list
uv run test/run.py -p aicc_test
```

### Configuration groups

`start.py --reinstall <group>` regenerates configuration for the selected environment. Current groups are defined in [`src/devenv_config.ts`](src/devenv_config.ts) and [`src/make_config.ts`](src/make_config.ts):

| Group | Purpose |
| --- | --- |
| `dev`, `devtest_ood1` | Preconfigured local DV Zone at `test.buckyos.io` |
| `release` | Unactivated system using the `buckyos.ai` environment |
| `nightly` | Unactivated system using the `buckyos.io` environment |
| `vmtest` | VM activation test without a preseeded identity |
| `alice.ood1`, `bob.ood1`, `charlie.ood1`, `dave.ood1` | Preset identities and network cases for the distributed test environment |
| `devtests_ood1`, `sn_web` | The `devtests.org` OOD used by the test environment |

SN configuration generation has moved to `cyfs-gateway/src/make_sn_config.ts`; `sn` and `sn_server` are not supported by BuckyOS's `make_config.ts`.

## BuckyOS Vision

- `Internet is BuckyOS`: Build a new dApp ecosystem on top of decentralized, and therefore necessarily open-source, infrastructure. Applications become more interconnected, more modular, and more AI-friendly. This should support applications an order of magnitude more complex than today’s while reducing both development and operational costs by an order of magnitude as well. (A 100x productivity gain.)
- The internet’s infrastructure should not be controlled by corporations. Decentralized infrastructure can eliminate platform taxes and unfair platform rules. Through token-based ownership, the base platform can be jointly owned by developers, evangelists, users, and capital, sharing revenue and agreeing on fairer platform rules together.
- The core logic behind `kill app` is "using LLMs to solve the scarcity of information filtering." Using AI to generate information is a need for a minority; using AI to filter information is a need for everyone. AI can apply common sense to help users filter the information they receive and address today’s echo-chamber problem. That has obvious value for users and a positive social impact. For the AI industry, the semantic network formed by linking every user’s KnowledgeBase through CYFS can also help LLMs produce better results on top of real-time, accurate information.

### Learn More About BuckyOS

- [Architecture and core concepts](doc/arch/README.md)
- [Application development quickstart](doc/sdk/app-dev-quickstart.md)
- [Rust API runtime](doc/sdk/buckyos-api-runtime.md)
- [OpenDAN agent development](doc/sdk/OpenDAN_Agent_Dev_Guide.md)
- [Source tree and SDK/CLI build](src/readme.md)
- [Runtime directories](doc/path_usage.md)
- [Contributor workflow](harness/README.md) and [repository guidelines](AGENTS.md)

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

- **0.6.0 Beta2:** 4% (Completed in April 2026)
- **0.7.0 Beta2.2:** 7.5% (Official launch planned for October 15, 2026)
- **0.8.0 Beta3:** 2.5% (Complete distributed kernel release, planned for the end of 2026)

The goal of **0.8.0 / Beta3** is to deliver the complete distributed kernel, with launch planned for the end of 2026. Work toward this release includes distributed storage integration, backup and recovery, data reliability, and system self-healing.


## License

BuckyOS is a free, open-source, decentralized system. We encourage vendors to build commercial products on top of BuckyOS and promote fair competition. Our licensing choices are designed to create a win-win ecosystem, preserve the decentralized core, protect contributors, and support a sustainable long-term ecosystem.

We use a dual-license model. One side is a traditional LGPL-based license that requires kernel modifications to follow GPL terms. Closed-source applications are allowed, but they cannot become core system components. The other side is a SourceDAO-based license. When an organization that issues DAO tokens uses BuckyOS, it must donate a portion of those tokens to the BuckyOS DAO under that license.

There is still no existing license that fully matches our needs, so during the DEMO phase we are temporarily using the BSD license. I believe that once the PoC is complete, we will be ready with the formal license.
