# BuckyOS Launch!

## Why BuckyOS?

Services running on the Cloud (Server) are closely related to our lives today, and people can hardly live without services in their daily lives. However, there is no operating system specifically designed to run services.

Already have Cloud Native? Cloud Native is designed for commercial companies or large organizations, a specialized operating system for running services, which is difficult for ordinary people to install and use. From a historical perspective, traditional System V operating systems (Unix) initially ran on IBM minicomputers, far from ordinary people, but after iOS, ordinary people can easily use modern operating systems: managing their applications and data well and running stably for a long time, without understanding a lot of profound technology. iOS pioneered a new era where everyone can use personal software.

Today, services are important to everyone, and people should have the freedom to install and use services (we call this type of service Personal Service). Personal Services that can run independently without relying on commercial companies are also known as dApps, and the entire Web3 industry has already had a large number of people engaged in this, doing a lot of work based on blockchain and smart contract technology. We believe that the simplest and most direct way to achieve this goal is for people to purchase consumer-grade servers with pre-installed OS, and then people can simply install applications on this OS: the application includes both Client and Service, and related data will only be saved on the user-owned server. The OS operation is simple and easy to understand, and also ensures high reliability and high availability of services. When a failure occurs, usually only replacing the damaged hardware can restore the system to work, without relying on professional IT Support.

BuckyOS was born to solve these problems and will become the "iOS" in the CloudOS field, pioneering a new era of the Internet where everyone has their own Personal Service.

## Goals of BuckyOS

Buckyos is an Open Source Cloud OS (Network OS) aimed at end users. Its primary design goal is to allow consumers to have their own cluster/cloud (to distinguish from conventional terminology, we call this cluster Personal Zone, or Zone for short), with all devices in the consumer's home and the computing resources on these devices connected to this cluster. Consumers can install Services in their own Zone as easily as installing Apps. Based on BuckyOS, users can own all their data, devices, and services. In the future, if there is enough computing power within the Zone, they can also have Local LLM, and on top of that, have a truly AI Agent that serves themselves completely.

BuckyOS has several key design goals:

`Out-of-the-box:` After ordinary users purchase commercial PersonalServer products with BuckyOS installed, they can complete initial setup very simply. The basic process is: Install BuckyOS Control App -> Create identity (essentially a decentralized identity similar to BTC wallet address) -> Set/Create Zone ID (domain name) -> Plug in the device to power and network and turn it on -> Issue pending activation device in App -> Activate the device and add it to your Zone -> Apply default settings -> Start default Service (at least one network file system).

`Service Store:` Managing services running on BuckyOS is as simple as managing Apps on iOS. The Service Store will also build a healthy dApp ecosystem: achieving a win-win situation for developers, users, and device manufacturers. BuckyOS has a complete Service permission control system, Services run in specified containers, fully controllable, ensuring user data privacy and security.

`High Reliability:` Data is the most important asset for people in the digital age, and not losing data is an important reason why users choose to use Services rather than Software today. BuckyOS must design a reasonable hardware architecture for future commercial Personal Server products to ensure that no data will be lost in case of any hard disk failure (which is inevitable). At the same time, BuckyOS also constructs an open Backup System, choosing different backup tasks according to the importance of data, and backing up to different backup sources. Backup inevitably has costs, and the open Backup System guarantees users' right to choose. When the system is in a fully backed-up state, even if all the hardware of the system is destroyed, it can be fully restored on new hardware.

`Access Anywhere:` Users' Personal Servers are usually deployed in their own homes, and local network access is definitely fast and stable: for example, home security cameras storing important videos in the massive storage space provided by Personal Server is definitely faster and more stable than storing them in the Cloud today. But more often, we hope that App Clients running on mobile phones can connect to Personal Servers at any time. An important design goal of the BuckyOS system is to make all Services transparently obtain this feature. We mainly achieve this goal through 3 methods:

1. Better integration of IPv6
2. Integrate P2P protocols to achieve NAT traversal as much as possible
3. Encourage users to deploy Gateway Nodes on public networks. Achieve Access Anywhere through traffic forwarding.

`Zero Operation:` As time passes, any system may be damaged or need to be adjusted according to actual conditions. BuckyOS helps ordinary consumers who don't have professional operation and maintenance capabilities to complete necessary operation and maintenance operations independently by defining some simple processes, ensuring the availability, reliability, and scalability of the system.

1. Hardware damage without causing failure: Purchase new equipment, activate new equipment with the same hardware name to replace old equipment, wait for the status of new equipment to become normal, then unplug the faulty old equipment.
2. Hardware damage causing failure: If urgent, you can immediately enable cloud virtual devices and restore from backup to make the system available. Then purchase new equipment, activate new equipment with the same hardware name to replace old equipment, wait for the status of new equipment to become normal.
3. Insufficient storage space: Purchase new equipment, after activation, the system's available space increases.

`Zero Depend:` The operation of BuckyOS does not depend on any commercial company or any centralized infrastructure. From the user's perspective, there's no need to worry about functionality issues after the manufacturer of the purchased Personal Server with BuckyOS goes bankrupt. The standard open-source distribution of BuckyOS uses decentralized infrastructure, such as integrating decentralized storage (ERC7585) to achieve decentralized backup. The commercial distribution of BuckyOS can integrate some paid subscription services (such as traditional backup subscription services), but these services must give users the right to choose another supplier.

`Upgrade to High Availability:` Considering the home attribute of Personal Server, BuckyOS is usually installed on a small cluster composed of 3-5 Servers. In such a small-scale cluster, our trade-off is to try to ensure high reliability rather than high availability. This means that we allow the system to enter a read-only or unavailable state at some times, as long as the system can be restored to availability after some simple operation and maintenance operations by the user. However, when BuckyOS is installed on a medium-sized cluster composed of dozens or even hundreds of servers, this cluster is usually owned by small and medium-sized enterprises and can be configured to be highly available with simple IT Support: when expected hardware failures occur, the system still maintains complete availability.

## System Architecture Design

I have already designed the overall architecture of BuckyOS, which I think can achieve the above goals. The complete system architecture of BuckyOS definitely needs in-depth discussion and repeated iterations, and we will have a continuously updated document to specifically discuss it. Here, I want to stand at the origin and describe the entire architecture as macroscopically as possible. Let engineers who are exposed to BuckyOS for the first time have a rough understanding of the key design of BuckyOS in the shortest time. As BuckyOS iterates, the specific design of the system will be continuously adjusted, but I am confident that some of the most basic design principles and framework thinking on the main processes mentioned in this article will have a long life.

At the time of completion of this article, BuckyOS has completed the demo version (0.1), so many designs have been verified to some extent. Many designs can already be observed through the crude code of DEMO.

### Some Basic Concepts

![Typical Topology of BuckyOS](./doc/pic/buckyos-Desc.svg)

Referring to the typical topology diagram above, understand some of the most important basic concepts in BuckyOS:   
- A group of physical servers form a cluster, BuckyOS is installed on this cluster, this cluster becomes a Zone.
- Zone is composed of logical Nodes. For example, 6 virtual machines running on the servers in this cluster form the mutually isolated ZoneA and ZoneB, a formal Zone consists of at least 3 Nodes.

`Zone: A cluster installed with BuckyOS is called a Zone`, the devices in the cluster usually physically belong to the same organization. According to BuckyOS's assumed scenario, this organization is usually a family or small business, so the devices in the Zone are usually mostly connected to the same local area network. BuckyOS itself supports multiple users, a Zone can serve multiple users, but the same logical user (identified by DID) can only exist in one Zone.

Each Zone has a globally unique ZoneId. ZoneId is a human-friendly name, preferably a domain name, for example, buckyos.org can be considered a ZoneId. BuckyOS will also natively support Blockchain-Based name systems, such as ENS. Anyone can query the current public key, configuration file (`ZoneConfig`), and signature of the configuration file through ZoneId. Possessing the private key corresponding to the current public key of the Zone gives the highest authority of the Zone.

`Node: The Server that makes up the Zone is called a Node.` A Node can be a physical Server or a virtual Server. Any two Nodes within the same Zone cannot run on the same physical Server.

Each Node has a unique NodeId within the Zone. NodeId is a human-readable friendly name, which can accurately point to a Node in the form of $node_id.zone_id. In a normally running Zone, the public key of the Node, configuration file (`NodeConfig`), and signature of the configuration file can be queried through NodeId. The private key of the Node is usually stored in the Node's private storage area and changed periodically, and the BuckyOS kernel services running on the Node use this private key to periodically declare identity and obtain correct permissions.

### System Startup
Below is an introduction to the key process of BuckyOS from system startup to application service startup:

1. Each Node independently starts the node_daemon process. The following process occurs simultaneously on all Nodes in the Zone.
2. When the node_daemon process starts, it will know its zone_id and node_id according to the node_identity configuration. A node that cannot read this configuration indicates that it has not joined any Zone.
3. Query zone_config based on zone_id through the nameservice component.
4. Prepare etcd according to zone_config. etcd is the most important basic component in the BuckyOS system, providing reliable consistent structured storage capabilities for the system.
5. The successful initialization of the etcd service means that BuckyOS booting is successful. Subsequently, node_daemon will further start kernel services and application services by reading node_config stored on etcd.
6. The processes of application services are all stateless, so they can run on any Node. Application services manage state by accessing kernel services (DFS, system_config).
7. BuckyOS exposes specific application services and system services to outside the Zone through the cyfs-gateway kernel service,
8. When the system has changed, the buckyOS scheduler will work, modifying the node_config of specific nodes to make this change effective
9. Changes in Node_config will cause 3 types of things to happen:
    - a. Kernel service processes start or stop on a certain Node
    - b. Application service processes start or stop on a certain Node
    - c. Execute/cancel a specific operation and maintenance task on a certain Node (such as data migration)
10. Adding new devices, installing new applications, system failures, system setting modifications may all cause system changes.
11. After the system changes, the BuckyOS scheduler will start working, the scheduler will reassign which processes run on which Nodes, which data is stored on which Nodes.
12. Scheduling is transparent to 99% of applications, the scheduler can make the system better exert the capabilities of the hardware, improving the reliability, performance and availability of the system.

From an implementation perspective, the running BuckyOS is a distributed system, definitely composed of a series of processes running on Nodes. By understanding the core logic of these important processes, we can further understand BuckyOS:

![BuckyOS MainLoop](./doc/pic/buckyos-MainLoop.svg)

"Any operating system is essentially a loop", the above figure shows the two most important loops in BuckyOS.

Node Loop: The main logic of the most important kernel module "node_daemon". The core purpose of this loop is to manage processes and operation and maintenance tasks running on the current node according to node_config. This flowchart also shows the boot process of node_daemon starting etcd.

Scheduling loop: The most important loop in traditional operating systems, handling important system events, updating node_config and implementing purposes through NodeLoop. Due to space limitations, specific examples cannot be given here, but after our deduction, the above two loops can simply and reliably implement the key system design goals of BuckyOS.

The above dual-loop design also has the following advantages:
1. Low-frequency scheduling: The scheduler only needs to start when events occur, reducing the consumption of system resources.
2. The scheduler can crash: Because updating NodeConfig is an atomic operation, the scheduler can crash at any time during operation. And the system only needs to have one scheduler process, without the need for complex distributed design. Even if the system splits, NodeLoop will continue to work faithfully according to the state of the last system.
3. Scheduling logic can be extended, specific scheduling of large-scale systems can be manually involved: Node Loop doesn't care how node_config is generated. For large-scale complex systems where it's difficult to write automated scheduling logic, it can be handled by professionals, making the scheduler a pure node_config construction auxiliary tool.
4. Simple and reliable: During the operation of NodeLoop, it does not involve any network communication operations and is completely independent. This structure does not have any additional assumptions about the distributed state consistency of BuckyOS.

After understanding the above process, let's take an overall look at the architecture layering and key components of BuckyOS:

### System Architecture and Key Components

![System Architecture Diagram of BuckyOS](./doc/pic/buckyos-Architecture.svg)


The BuckyOS system architecture consists of three layers:

1. **User Apps**:

    - App-services managed by users and run in user-mode. They can be developed in any language.
    - BuckyOS user-mode isolation is ensured by the App-Container.
    - App-services cannot depend on each other.

2. **System Frame (Kernel) Services**:

    - Frame-Service is a kernel-mode service that exposes system functions to App-Services through kernel RPC.
    - It can be extended by the system administrator, similar to installing new kmods in Linux.
    - Frame-service usually runs in a container, but this container is not a virtual machine.

3. **Kernel Models**:

    - The Kernel Models are not extensible. This layer prepares the environment for Kernel Services.
    - As the system's most fundamental component, it aims to reach a stable state as quickly as possible, minimizing modifications.
    - Some basic components in this layer can load System Plugins to extend functions, such as pkg_loader supporting new pkg download protocols through predefined SystemPlugin.

Below, we provide a brief overview of each component from the bottom up.

- **etcd:** A mature distributed KV storage service running on 2n+1 nodes, implementing BuckyOS’s core state distributed consistency. All critical structured data is stored in etcd, which requires robust security measures to ensure only authorized components can access it.

- **backup_client:** Part of BuckyOS Backup Service. During node boot, it attempts to restore etcd data from backup servers outside the Zone based on zone_config. This ensures cross-zone data reliability.

- **name_service_client:** Crucial to BuckyOS Name Service, it resolves information based on given NAMES (zoneid, nodeid, DID, etc.). Before etcd starts, it queries public infrastructures like DNS and ENS. Once etcd is running, it resolves based on etcd configurations. It aims to be an independent, decentralized open-source DNS Server.

- **node_daemon:** A fundamental service in BuckyOS, running NodeLoop. Servers in a Zone must run node_daemon to become a functional Node.

- **machine_active_daemon:** Strictly speaking, not part of BuckyOS, but its BIOS. It supports server activation to become a node. Hardware manufacturers are encouraged to design more user-friendly activation services to replace machine_active_daemon.

- **cyfs-gateway:** A critical service in BuckyOS, implementing SDN logic. We will provide a detailed article on cyfs-gateway. Its long-term goal is to be an independent, open-source web3 gateway, meeting various network management needs.

- **system_config:** A core kernel component (lib) that semantically wraps etcd access. All system components should use etcd through system_config.

- **acl_config:** A core kernel component that further wraps ACL permissions management based on system_config.

- **kLog:** An essential service providing reliable logging for all components, crucial for debugging and performance analysis in production. While based on the raft protocol, kLog is optimized for write-heavy scenarios. We aim to merge kLog and etcd for simplicity and reliability in the future.

- **pkg_loader:** A core kernel component with two primary functions: loading pkgs based on pkg_id and downloading pkgs from repo-server based on pkg_id. Pkgs are akin to software packages (similar to apk) and can include version, hash, and other information.

- **dfs:** As a critical Frame-Service, dfs provides reliable data management. The current backend implementation uses GlusterFS, but we plan to design a custom distributed file system called "dcfs" for new hardware and smaller clusters. 

- **rdb:** We may provide a standard RDB for developers in the future, enabling migration of sqlite/mysql/redis/mongoDB to BuckyOS.

- **kRPC:** Short for Kernel-Switch RPC, it authenticates frame-service callers and supports ACL queries. It also aids developers in exposing functions reliably (similar to gRPC).

- **control_panel:** Exposes SystemConfig's read/write interface via kRPC and includes the default BuckyOS management WebUI. It modifies system configurations and waits for the scheduler to apply changes.

- **Scheduler:** The brain of BuckyOS, implementing the scheduling loop. It provides advanced capabilities of a distributed OS, maintaining simplicity and reliability in other components.

BuckyOS will also include some extension frame-services for essential product features, not detailed here.

### Application Installation and Operation

We will now describe key processes from an application-centric perspective:

![BuckyOS App Install Flow](./doc/pic/buckyos-App%20Install.svg)

The above flow illustrates the application installation and startup process, emphasizing Zero Dependency & Zero Trust principles.

1. The system needs external services to download pkgs during installation. This doesn’t lead to a complete dependency on external services; users can configure alternative pkg repo servers if the default one fails.
2. Since pkg_id can include hash information, Zero Trust is achieved by verifying pkgs from any source.
3. Once installed, the system stores app-pkgs in an internal repo-server, backed up as part of system data. Node app-pkg downloads rely solely on zone-internal repo servers.

Next is the application startup process.

![BuckyOS App Start Flow](./doc/pic/buckyos-App%20Container.svg)

The diagram outlines the app container startup process and how kRPC service access control is implemented.

BuckyOS aims to reduce the development threshold for AppServices:
a. Compatible with containerized apps, running them on BuckyOS with appropriate permissions.
b. Trigger configuration (compatible with fast-cgi), further reducing development complexity and resource usage.
c. Apps developed or modified with the BuckyOS SDK can access all frame service functions.

## Coding Principles

BuckyOS uses Rust as the primary development language. Leveraging Rust’s system programming insights, we don’t need to reiterate traditional principles in resource management, multithreading, and exception handling. However, BuckyOS, being a distributed system, involves inherent complexity. Below are distilled principles for developing high-quality distributed system code. Non-compliant code will not be merged.

### Simplicity and Reliability

Define clear component boundaries and dependencies to reduce cognitive load. Implement functions outside the kernel or system services whenever possible. Design core components as independent products, avoiding a tightly coupled system that obscures true component boundaries.

Write evidently reliable code. Distributed system complexity cannot be mitigated solely through extensive testing. For high-reliability components, reduce code volume, making it comprehensible and evidently reliable. Minimize mental burden during code review, cautiously introduce proprietary concepts, and prudently select simple, reliable third-party libraries. For simplicity, module design may forgo some reusability. For distributed systems, KISS always trumps DRY.

### Avoid Potential Global State Dependencies

Making decisions based on current cluster state is intuitive but often unwise. Distributed systems cannot reliably and entirely capture global state, and directives based on this state are challenging to fully and timely execute.

### Let It Crash

Do not fear crashes in distributed systems. Log incidents and exit (crash) on unexpected situations. Do not retry or restart dependencies. Only a few components may retry or start another process, subject to thorough review.

### Log First

Logging is critical infrastructure in distributed systems, serving as a diagnostic tool during development and a foundation for fault detection and performance analysis in production. Understand logging standards (especially for info level and above) and carefully design log outputs.

### Avoid Request Amplification

Responding to one request often involves issuing multiple requests. While controlled amplification is acceptable (e.g., three DFS requests for a file download), beware of uncontrolled amplification, like issuing requests to all eligible nodes or indefinite retries. "Query-control" operations can exacerbate resource strain during system congestion.

### Zero Dependency & Zero Trust

Minimize dependencies on external Zone facilities. Consider necessity, frequency, and alternative designs for accessing external servers. Distrust external Zone servers, relying on public-private key systems for content verification. Verify content creators, not sources, and build a validation system on this principle.

### Comprehensive Understanding of Processing Chains

Avoid adding implicit intermediaries to solve design problems. Over-architecting distributed systems leads to complexity despair. Solve issues by locating the optimal position within the process chain, avoiding superficial solutions like additional caches or queues. Only one cache per processing chain.

### Respect for Persistent Data

User data is precious. Distinguish between structured and unstructured data, thoughtfully deciding on state persistence. Utilize mature services for state preservation when possible. Direct disk operations require comprehensive design and testing.

### ACID Guarantees by Core Components

Distributed transactions are notoriously complex. Avoid implementing distributed transactions whenever possible. Use mature distributed transaction services instead of custom implementations, as achieving correctness is highly challenging.

### Appropriate Network Request Handling

BuckyOS’s cyfs-gateway extends network protocol paradigms (tcp/udp/http) to include NamedObject (NDN) semantics. Understanding network request paradigms optimizes resource usage and request handling efficiency.

## Using SourceDAO for Open Source Collaboration

"Open source organizations have a long history and brilliant achievements. Practice has proved that an open source organization can achieve the goal of writing better code only by working in the virtual world. We believe that software development work is very suitable for DAO. We call this DAO for decentralized organizations to jointly develop software as SourceDAO." ---- from the White Paper of CodeDAO (https://www.codedao.ai)

SourceDAO provides a comprehensive design for DAO-ification of open source projects. The CYFS Core Dev Team has implemented smart contracts accordingly. Using OpenDAN as an example, we outline the basic operation process. Detailed design is available in the white paper. Our approach stems from a fundamentalist perspective on open source (aligned with GPL and GNU), influenced by Bitcoin’s "man is evil if not restrained" assumption. Although some designs may seem extreme, we believe they are understandable.

### Basic

 Operation Process

1. **Create an organization:** Define goals, initial DANDT distribution, initial members, and establish the initial Roadmap.
2. **Roadmap:** Link system maturity to token release. More mature systems release more tokens. The Roadmap outlines project phases: PoC, MVP, Alpha, Beta, Formula (Product Release), each with a DANDT release plan.
3. **Development as mining:** Achieve DAO goals by advancing the Roadmap. Set plans, calculate contributions, and distribute DANDT accordingly.
4. **Market behavior:** Use DANDT to increase project visibility, incentivize new users, and design fission rewards.
5. **DAO governance:** DANDT holders participate in governance.
6. **Financing:** Use DANDT for resource acquisition.

Currently, the BuckyOS DAO contract plans to deploy on Polygon, with a total of 2.1 billion tokens, abbreviated as `BDT` (BuckyOS DAO Token). Initial deployment requires an initial version plan to outline BDT release speed and establish the first committee (at least three people, possibly selected from DEMO contributors). 

### Preliminary Version Plan:

#### 2024
- **0.1 Demo:** 2.5% (Done)
- **0.2 PoC:** 2.5%
- **0.3 Pre-Alpha:** 5% (First complete version)
- **0.4 Alpha:** 2.5% (2024 Q4)

#### 2025
- **0.5 Beta:** 2.5%
- **0.6 RC:** 5% (First public release version)
- **0.7 First Release:** 2.5% (2025 Q3)

## License

BuckyOS is a free, open-source, decentralized system encouraging vendors to build commercial products based on BuckyOS, fostering fair competition. Our licensing choice aims to achieve ecosystem win-win, maintain a decentralized core, protect contributor interests, and build a sustainable ecosystem. We adopt dual licensing: a traditional LGPL-based license requiring GPL compliance for kernel modifications, allowing closed-source applications (which cannot be essential system components), and a SourceDAO-based license. When a DAO-token issuing organization uses BuckyOS, it must donate a portion of its tokens to the BuckyOS DAO according to this license.


There is currently no license that meets our requirements, so we will temporarily use the BSD license for DEMO. I think we will definitely have a formal license ready when the PoC is completed.

