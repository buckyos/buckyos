# BuckyOS Alpha2(0.4.0) Launch!

This is second alpha release for developers.	The core goal of Alpha2 is to ***stabilize the design***.

In this version, we stabilized key designs for developers, built the first version of the SDK, and spent a lot of work on the bottom layer, hoping to stabilize the most basic data storage structure and the new cyfs:// design. We need to protect the intellectual investment of BuckyOS application developers and do our best to reduce future breaking changes.

The following are the main updates of this version:

- Stabilize the kernel design, stabilize the development interface of `frame service` and `dApp` (including websdk)
- Stabilize the design of ndn-related protocols in cyfs://, including the interface design of key modules such as DID system, URL construction, NamedDataManager, etc.
- Stabilize the Tunnel framework of cyfs-gateway, and implement the rtcp protocl and socks protocol base this framework
- Stabilize the dfs://, kv:// data directory structure design of buckyos, as well as the isolation logic and rbac logic of multiple users.
- Stabilize pkg-system, and realize the installation and auto-update of applications and services through `subscription source -> repo-server -> ood nodes`
- Stabilize the Product UI framework design of BuckyOS. Including the structure of BuckyOS Desktop/App, and the implementation of the system Control Panel
- (Delay to Alpha3) Stabilize the "processing chain configuration" (probe->matcher->process->post_resp_filter) of cyfs-gateway to implement a scalable intelligent gateway on a consistent basic design
- (Delay to Alpha3) Realize the backup and recovery of the system and the export and import of user data.

Join us on this journey! Please feel free to submit issues or pull requests! Let's build the next generation of `Dsstributed Personal AI Operating System` together!

We are currently in the DAO acceptance phase for Alpha2, and we plan to start the development of Alpha3 next week. Alpha3 is a key version of BuckyOS, and we will integrate this version with [OpenDAN](https://github.com/fiatrete/OpenDAN-Personal-AI-OS) as planned, providing the key AI capabilities required by OpenDAN in BuckyOS.

## Let's Get Started

Get Active Code First:
[https://github.com/buckyos/buckyos/discussions/70](https://github.com/buckyos/buckyos/discussions/70)

### No Docker Installation Method

We know everyone loves Docker!

However, since BuckyOS can be considered a "Deploying Kubernetes at Home with No IT Support" it relies on container technology but shouldn't run inside Docker itself. To provide a Docker-like experience, BuckyOS releases all binaries as statically linked files, so in 99% of cases, you won't face "depends issues."

### Installing from deb

Suitable for x86_64 Linux distributions using apt and WSL2. Depending on your internet speed, the process takes around 5-10 minutes.

Run the following command to download and install buckyos.deb:

```bash
wget https://buckyos.ai/static/buckyos_amd64.deb && dpkg -i ./buckyos_amd64.deb
```

If you're installing on ARM devices like Raspberry Pi, use buckyos_aarch64.deb:

```bash
wget https://buckyos.ai/static/buckyos_aarch64.deb && dpkg -i ./buckyos_aarch64.deb
```

The installation process will automatically download dependencies and default application Docker images, so make sure you have a stable internet connection that can access apt/pip/Docker repositories.

During installation, you may see some permission errors, but most of them are not significant. After installation, open your browser and go to:

```
http://<your_server_ip>:3180/index.html
```

You will see the BuckyOS startup setup page, follow the instructions to complete the setup, and you're good to go! During the Alpha testing phase, using the `web3.buckyos.ai` relay and D-DNS services requires an invitation code (Get Invitation Code here), which you can obtain from our issue page. (If you have your own domain and have set up port forwarding on your router, you don't need any of the services from `web3.buckyos.ai` and can try it without an invitation code.)

### Install on Windows

Coming Soon.

### Install on MacOS

Coming Soon.

### Install on Linux without .deb support

Coming Soon.


## Installing from a Virtual Machine

We are preparing related images to support running BuckyOS on Windows, macOS, and major NAS brands that do not have WSL environments. We promise to complete this work before the Alpha2 release.

## Installing from Source Code

Installing from source is a great way to learn more about BuckyOS and is the first step towards contributing. By installing from source, you can also install BuckyOS on macOS.


```bash
git clone https://github.com/buckyos/buckyos.git && cd buckyos && python3 devenv.py && python3 src/build.py
```

Once the build script completes, the installation is done on your local machine (for convenience, it includes test identity information by default). Run the following commands to start BuckyOS in its initial state:

```bash
sudo /opt/buckyos/bin/node_daemon --enable_active
```

## BuckyOS's Vision


- **Internet is BuckyOS**: By creating a new decentralized (and necessarily open-source) infrastructure, we aim to build a new dApp ecosystem where applications are more interconnected, more modular, and better integrated with AI. This approach can support building applications an order of magnitude more complex than those we have today, while also reducing both development and operating costs by a similar scale—ultimately increasing productivity by a factor of 100.

- **The infrastructure of the Internet cannot be controlled by corporations**: Services running on the Cloud (Server) are closely related to our lives today, and people can hardly live without services in their daily lives. However, there is no operating system specifically designed to run services. A decentralized infrastructure can eliminate platform taxes and unfair platform policies. By distributing Tokens, the underlying infrastructure can be co-owned by developers, evangelists, all users, and capital, sharing in the revenue and jointly setting fairer rules for the platform.

- **“Killing apps” through LLMs**: The underlying logic here is “use LLMs to solve the shortage of information filtering.” While AI-based content generation is needed by a subset of users, AI-based filtering is needed by everyone. By leveraging AI’s general reasoning abilities, we help users filter the information they receive—tackling the “echo chamber” problem. This yields obvious benefits for users and positive social impact. Moreover, for the AI industry, connecting every user’s KnowledgeBase into a semantic network through CYFS provides LLMs with real-time and accurate information, resulting in better outcomes.

###  Learn more about BuckyOS

- BuckyOS Architecture Design (Coming Soon)
- Hello BuckyOS! (Coming Soon)
- BuckyOS dApp Developer Manual (Coming Soon)
- BuckyOS Contributor Guide (Coming Soon)

## The Next Generation of GPL: Creating a New Model for Open Source Collaboration

"Open source organizations have a long history and brilliant achievements. Practice has proved that an open source organization can achieve the goal of writing better code only by working in the virtual world. We believe that software development work is very suitable for DAO. We call this DAO for decentralized organizations to jointly develop software as SourceDAO." ---- from the White Paper of CodeDAO (https://www.codedao.ai)

BuckyOS’s open-source community operates in a DAO-like manner. Our goal is to address the issue where open-source contributors typically receive no direct return, or are treated as free labor:

- **Coding mining**: Enhance the quality of version releases by aligning interests.
  - We employ a GPL-like “copyleft” mechanism to build a shared interest structure among upstream and downstream stakeholders.  
  - Through automatic smart contract–based revenue sharing, contributors to fundamental libraries that keep the world running smoothly can enjoy stable and long-term income (which they deserve).
- **Governance**: By unifying token holdings and aligning everyone’s interests, we create a common ground among both users and developers. This lets us make rational decisions based on shared benefits (even disagreements are “family disputes”).
- **Return to a standard open-source development process**.

`Open, Transparent, Free to Come and Go (everyone can participate), and Result-Oriented`

SourceDAO is our open source DAO smart contract based on the above concept. Visit [https://dao.buckyos.org/](https://dao.buckyos.org/) for more details.


## Preliminary Version Plan:

#### 2024

- **0.1 Demo:** 2.5% (Done)
- **0.2 PoC:**  2.5% (Done)
- **0.3 Alpha1:** 2.5% (DONE)
- **0.4 Alpha2:** 2.5% (Last Release!)

#### 2025
- **0.5 Alpha3:** 2.5% (2025Q2 First Public Test)
- **0.6 Beta** 5% (First public release version)
- **0.7 Release:** 2.5% (2025 Q4)



## License

BuckyOS is a free, open-source, decentralized system encouraging vendors to build commercial products based on BuckyOS, fostering fair competition. Our licensing choice aims to achieve ecosystem win-win, maintain a decentralized core, protect contributor interests, and build a sustainable ecosystem. We adopt dual licensing: a traditional LGPL-based license requiring GPL compliance for kernel modifications, allowing closed-source applications (which cannot be essential system components), and a SourceDAO-based license. When a DAO-token issuing organization uses BuckyOS, it must donate a portion of its tokens to the BuckyOS DAO according to this license.

There is currently no license that meets our requirements, so we will temporarily use the BSD license for DEMO. I think we will definitely have a formal license ready when the PoC is completed.
