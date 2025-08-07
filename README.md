
# BuckyOS Alpha3 (0.4.1) Release!

Alpha3 is the final planned Alpha release. Its primary goals are to **complete features left over from Alpha2**, improve system stability by establishing a more extensive testing system, and prepare for the public Beta release.

The main features planned for Alpha3 include:

* Fixing kernel components that were rushed in the previous release to ensure robustness, and improving related test cases
* Improving the standard distributed development environment and building test cases based on it
* Optimizing the repository structure to prepare for the independent productization of `cyfs-gateway`
* Building a comprehensive nightly channel with support for automatic version updates
* Improving `ndn-lib` with support for `chunklist` in `fileobject`
* Adding basic support for containers and `DirObject` (Git mode) in `ndn-lib`
* Supporting access to `smba` services via USB4 on macOS
* Supporting quick addition of Docker URLs in BuckyOS and access through `appID.$zoneID`
* Postponing the productization of backup and `cyfs-gateway` to Beta1
* Temporarily shelving the plan to support an etcd backend for `system_config` due to integration concerns with `dfs`

Join us on this journey! Feel free to submit issues or pull requests. Let’s build the next generation of **Distributed Personal AI Operating System** together!

We are currently in the DAO acceptance phase for Alpha3 and plan to start the development of Beta1 ASAP. Beta1 is a key version of BuckyOS. As planned, it will integrate with [OpenDAN](https://github.com/fiatrete/OpenDAN-Personal-AI-OS), providing the essential AI capabilities required by OpenDAN within BuckyOS.

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

```bash
git clone https://github.com/buckyos/buckyos.git && cd buckyos && python3 devenv.py && python3 src/build.py
```

Once the build script completes, the local installation is ready (test identity info is included by default). To start BuckyOS in its initial state:

```bash
sudo /opt/buckyos/bin/node_daemon --enable_active
```

### Common Scripts in the Source Directory

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

* To reinstall BuckyOS using a specified configuration group:

```bash
cd src
python3 start.py -reinstall $group_name
```

If `group_name` is empty, BuckyOS will start with an empty config and enter the pending activation state.

Currently, there are two built-in config groups:

* `dev`
* `dev_no_docker`

The old `python3 start.py --all` is now equivalent to `python3 start.py --reinstall dev`.

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
* **0.4 Alpha2** – 2.5% (Released April 2025, Done)

#### 2025

* **0.4.1 Alpha3** – 2.5% (This Release)
* **0.5 Beta1** – 5% (Public Release in October 2025)
* **0.6 Beta2** – 2.5% (Planned for Q4 2025)

---

## License

BuckyOS is a free, open-source, decentralized system. We encourage vendors to build commercial products based on BuckyOS, fostering fair competition. Our licensing aims to ensure ecosystem win-win, preserve decentralization, protect contributors, and support long-term sustainability.

We adopt dual licensing:

* A traditional LGPL-based license: kernel modifications must comply with GPL. Closed-source applications are allowed but **cannot** be core system components.
* A SourceDAO-based license: DAO-token issuing organizations using BuckyOS must donate a portion of their tokens to the BuckyOS DAO under this license.

Since no existing license fully meets our needs, we are temporarily using the BSD license during the DEMO phase. A formal license will be introduced once the PoC is finalized.

