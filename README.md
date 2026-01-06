
# BuckyOS Beta1 (0.5.1) Release!

Beta1 delivers the first public Beta release of BuckyOS. Major highlights include:

* **Kernel networking migration**: move the network kernel to the new process-chain based `cyfs-gateway`, making the networking kernel largely stable
* **BuckyOSApp**: manage private keys with a Web3 wallet style experience, and complete the end-to-end activation flow
* **Installer optimization for Beta targets**: systematic improvements for Windows/macOS installation scenarios
* **Scheduler improvements**: make core scheduling logic more independent and enable more advanced scheduling approaches
* **Developer workflow**: introduce `buckyos-devkit` to streamline daily build/install/test loops
* **Multipass-based dev loop**: improve complex scenario iteration with `buckyos-devkit`
* **Resolve-DID improvements**
* **pkg-meta updates** based on new `ndn-lib` core objects
* **App install protocol** based on the new pkg-meta version

Join us on this journey! Feel free to submit issues or pull requests. Let’s build the next generation of **Distributed Personal AI Operating System** together!

After Beta1, we have entered the Beta1.2 development cycle (planned release: 2026-02-15). This is a fast-iteration release with minimal kernel-level changes:

- Integrate with buckyos backup suite (system backup & restore)
- Add OPTask support in kernel (required by backup & restore)
- Ship system control service and the control panel UI
- Ship App install protocol UI and support the first large “killapp” `gitpot.ai`
- Integrate foundational services like slog/klog
- Deliver AI infrastructure needed by `gitpot.ai`
- Organize and refresh BuckyOS documentation

---

## Getting Started

Get Active Code:
[https://github.com/buckyos/buckyos/discussions/70](https://github.com/buckyos/buckyos/discussions/70)

### Install Without Docker

We know everyone loves Docker!

However, since BuckyOS is designed to be a "Deploying Kubernetes at Home with No IT Support" solution, it relies on container technology but should not run inside Docker itself. To offer a Docker-like experience, BuckyOS distributes all binaries as statically linked files, so in 99% of cases, you won’t encounter dependency issues.

### Installing from `.deb`

Suitable for x86\_64 Linux distributions using `apt` or WSL2. The process takes about 5–10 minutes depending on your network speed.

To install on x86\_64:

```bash
wget https://www.buckyos.ai/static/buckyos_amd64.deb && dpkg -i ./buckyos_amd64.deb
```

To install on ARM devices (like Raspberry Pi):

```bash
wget https://www.buckyos.ai/static/buckyos_aarch64.deb && dpkg -i ./buckyos_aarch64.deb
```

The installer will automatically download dependencies and default application Docker images, so make sure your internet connection is stable and can access apt/pip/Docker repositories.

You may encounter some permission errors during installation, but most are harmless. After installation, open your browser and visit:

```
http://<your_server_ip>:3180/index.html
```

You will see the BuckyOS startup setup page. Follow the instructions to complete the setup.

During the Alpha phase, access to relay and D-DNS services from `sn.buckyos.ai` requires an invitation code (Get it from our issue page). If you already have your own domain and have configured port forwarding on your router, you can try BuckyOS directly without relying on any `sn.buckyos.ai` services.

#### Ports & entry points (Beta1 quick reference)

- **Web onboarding page**: `http://<device_ip>:3180/index.html`
- **Device discovery / activation service**: `http://<device_ip>:3182/` (see `notepads/设备激活协议.md`)

### Install on Windows

Coming soon.

### Install on macOS

Coming soon.

### Install on Linux without `.deb` support

Coming soon.

---

## Installing from a Virtual Machine

We are preparing VM images to support BuckyOS on Windows, macOS, and popular NAS platforms that lack WSL environments. We promise to complete this work before the Alpha2 release.

---

## Installing from Source

Installing from source is a great way to explore BuckyOS and is the first step toward contributing. It also allows you to run BuckyOS on macOS.

### Install buckyos-devkit (required)

After cloning the repo, install `buckyos-devkit` first. It provides CLI commands such as `buckyos-build` and `buckyos-install`.

```bash
git clone https://github.com/buckyos/buckyos.git
cd buckyos
python3 -m venv venv
source venv/bin/activate
python3 -m pip install -U "buckyos-devkit @ git+https://github.com/buckyos/buckyos-devkit.git"
```

### Build

Before building, you can refer to `devenv.py` to bootstrap your environment. We mainly depend on the `Rust toolchain`, `Node.js + pnpm`, `Python 3.12`, and `docker.io`.

```bash
cd buckyos
buckyos-build
```

### Install

- **Full reinstall (recommended for first-time setup)**: clean + install_app_data + update module build outputs
- **Incremental overwrite install**: only copy updated module build outputs

```bash
# Full reinstall (recommended)
buckyos-install --all

# Incremental overwrite (without --all)
# buckyos-install
```

Then generate a config group (the `release/dev/...` below are config group names):

```bash
python3 make_config release
```

### Build & install cyfs-gateway (separate repo, order-sensitive)

Currently `cyfs-gateway` is built/installed in a separate repo. You must build/install it before running in most scenarios:

```bash
git clone https://github.com/buckyos/cyfs-gateway.git
cd cyfs-gateway
buckyos-build
buckyos-install --all
```

### Transitional Dev bootstrap (from clone to SN test group config)

If you are bootstrapping from scratch and need **buckyos + cyfs-gateway** built/installed, and want to generate the **sn** test group config (including 3 OOD identities), follow the order below.

Reference: [buckyos/buckyos issue #321](https://github.com/buckyos/buckyos/issues/321)

```bash
# 0) Prepare a working directory
mkdir -p ~/work && cd ~/work

# 1) Clone repos
git clone https://github.com/buckyos/buckyos.git
git clone https://github.com/buckyos/cyfs-gateway.git

# 2) Install buckyos-devkit (provides buckyos-build / buckyos-install)
cd ~/work/buckyos
python3 -m venv venv
source venv/bin/activate
python3 -m pip install -U "buckyos-devkit @ git+https://github.com/buckyos/buckyos-devkit.git"

# 3) (Common pitfall during transition) update Rust lockfile
cargo update

# 4) Build + full install in buckyos repo
buckyos-build
buckyos-install --all

# 5) Build + full install in cyfs-gateway repo
cd ~/work/cyfs-gateway
cargo update
buckyos-build
buckyos-install --all

# 6) Back to buckyos repo: generate identities + sn config
cd ~/work/buckyos
python3 make_config.py alice.ood1
python3 make_config.py bob.ood1
python3 make_config.py charlie.ood1
python3 make_config.py sn
```

Notes:

- `buckyos-build` / `buckyos-install` are **NOT** scripts in this repo; they are installed by `buckyos-devkit`.
- `--app=<name>` is optional:
  - `buckyos-build --app=<name>` builds a specific app
  - if `--app` is omitted, it builds/installs **all apps** defined in `bucky_project.yaml`
- `--all` semantics:
  - `buckyos-install --all` does a full reinstall (clean → install_app_data → update module build outputs)
  - `buckyos-install` without `--all` does an incremental overwrite install (only copies module build outputs)

### Common pitfalls (during transition)

- You may need to run `cargo update` frequently (especially in a fresh environment).
- `buckyos-devkit` evolves fast; installing from git is common during transition.
- `make_config.py` depends heavily on `buckycli`; in practice you usually need to run `buckyos-build && buckyos-install` in the buckyos repo first to ensure `buckycli` is available.
- Test CA root certificate reuse is implicit (convenient for browser install, confusing when switching/cleaning envs).

### Start & activate

To start BuckyOS and enter the same activation flow as packages: (because this is a dev start, after activation it won't auto-restart; run it once more to run in activated mode)

```bash
sudo /opt/buckyos/bin/node-daemon/node_daemon --enable_active
```

Note: depending on platform/build, the binary path can be either `node-daemon/node_daemon` or `node_daemon/node_daemon`. If one doesn't exist, try the other.

### Common scripts in the source directory

* To build only the Rust components:

```bash
cd src
python3 build.py --no-build-web-apps
```

* To copy the compiled binaries and launch `/opt/buckyos`:

```bash
cd src
python3 start.py
```

Open http://test.buckyos.io/ to verify that the system is working properly (remember to confirm that the host file is configured with test.buckyos.io pointing to 127.0.0.1)

* To reinstall BuckyOS using a specified configuration group:

```bash
cd src
python3 start.py -reinstall $group_name
```

If `group_name` is empty, BuckyOS will start with an empty config and enter the pending activation state.

Currently, the repo includes several commonly used config groups:

* `release` (production-like, uses buckyos.ai SN facilities)
* `dev` (no SN, dev-friendly, no external dependencies)
* `alice.ood1`, `bob.ood1`, `charlie.ood1` (preset identities for the planned virtual test environment)
* `sn` (SN node config for the virtual test environment)

The old `python3 start.py --all` is now equivalent to `python3 start.py --reinstall dev`.

### App install protocol & UI docs

- App install protocol: `notepads/app安装协议.md`
- App install UI draft: `notepads/app安装UI.md`

### SN VM test environment (sntest)

If you want to use the SN VM test environment (`sntest`) and iterate between `buckyos (ood)` and `cyfs-gateway (sn)`, see: `notepads/sntest环境使用.md`

---

## BuckyOS Vision

* **Internet is BuckyOS**: Build a new dApp ecosystem with a decentralized (and necessarily open-source) infrastructure where applications are more interconnected, more modular, and better integrated with AI. This supports apps an order of magnitude more complex than today’s, while reducing development and operational costs by the same factor — boosting productivity 100x.

* **The internet’s infrastructure should not be owned by corporations**: A decentralized infrastructure eliminates platform taxes and unfair policies. By distributing tokens, the base infrastructure can be co-owned by developers, evangelists, users, and capital, allowing shared revenue and governance.

* **"Killing apps" through LLMs**: The core idea is using LLMs to solve the “information filtering crisis.” While AI-generated content helps some users, **everyone** needs AI-based filtering. LLMs can help users filter incoming information, solving the "echo chamber" problem. The impact is immediate and positive, both socially and technically. CYFS can also connect every user’s KnowledgeBase into a semantic network, giving LLMs real-time and accurate information for better results.

---

### Learn More About BuckyOS

* BuckyOS Architecture Design (Coming Soon)
* Hello BuckyOS! (Coming Soon)
* BuckyOS dApp Developer Manual (Coming Soon)
* BuckyOS Contributor Guide (Coming Soon)

---

## The Next Generation of GPL: A New Model for Open Source Collaboration

> “Open source organizations have a long history and brilliant achievements. Practice has proven that better code can be written purely in the virtual world. We believe that software development is well suited for the DAO model. We call this DAO for collaborative decentralized software development: SourceDAO.” — from the CodeDAO White Paper ([https://www.codedao.ai](https://www.codedao.ai))

BuckyOS’s open-source community operates like a DAO. Our goal is to address the problem of unpaid open-source labor:

* **Code mining**: Improve version quality by aligning incentives
* **Shared interests**: A copyleft-like mechanism (inspired by GPL) links upstream and downstream contributors into a common interest structure
* **Automated revenue sharing**: Through smart contracts, contributors to core libraries receive stable and long-term income — what they rightfully deserve

For governance, shared token ownership aligns users and developers to make rational, consensus-based decisions. Even conflicts remain "family arguments."

**Open, Transparent, Free to Participate, and Result-Oriented**

SourceDAO is our DAO smart contract based on this philosophy. Visit [https://dao.buckyos.org/](https://dao.buckyos.org/) for more.

---

## Preliminary Version Plan

#### 2024

* **0.1 Demo** – 2.5% (Done)
* **0.2 PoC** – 2.5% (Done)
* **0.3 Alpha1** – 2.5% (Done)

#### 2025

* **0.4 Alpha2** – 2.5% (Done)
* **0.4.1 Alpha3** – 2.5% (Done)
* **0.5.1 Beta1** – 4% (This Release, Dec 2025)

#### 2026

* **0.5.2 Beta1.2** – 1% (Q1 2026)
* **0.6 Beta2** – 2.5% (Q2 2026)

---

## License

BuckyOS is a free, open-source, decentralized system. We encourage vendors to build commercial products based on BuckyOS, fostering fair competition. Our licensing aims to ensure ecosystem win-win, preserve decentralization, protect contributors, and support long-term sustainability.

We adopt dual licensing:

* A traditional LGPL-based license: kernel modifications must comply with GPL. Closed-source applications are allowed but **cannot** be core system components.
* A SourceDAO-based license: DAO-token issuing organizations using BuckyOS must donate a portion of their tokens to the BuckyOS DAO under this license.

Since no existing license fully meets our needs, we are temporarily using the BSD license during the DEMO phase. A formal license will be introduced once the PoC is finalized.

