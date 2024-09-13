# BuckyOS Current Development Plan PoC (Pre-Alpha1)

## Overview of the Overall Plan

### Phase 1: Private Storage Cloud (Alpha, 2024)

- The core focuses are cluster management, storage system optimization, backup and recovery, and file sharing. All these core functions should be provided with a complete and usable UI through an app/browser + device approach (similar to iStoreOS). Especially from the perspective of competing with NAS, we need very comprehensive product design to support product sales.

- The three pre-configured nodes through device sales are: storage server, portable WiFi (SBOX), and public Gateway. The basic function of the portable WiFi is to serve as a second node at home, with a smaller storage capacity, mainly for "hot data replicas." Its base operating system is likely Android. However, when carried by the user, it can be powered by a battery, and in a weak network (e.g., on an airplane), it can provide read-only access to the file system, allowing access to recently used hot data and newly written data. This should be sufficient in most cases.

- This is the first official version of the system, so we need to establish a complete system kernel framework, particularly the u/k separation, as well as a comprehensive basic permission control system, and support user-space app.server installation and updates in the basic system management.

- Once the backup system is integrated with DMC, it can provide a truly decentralized backup ecosystem. Token incentives will be the main selling point of our first version.

- File sharing will be developed through applications, which will be our main method of attracting new users in the early stages. Public file publishing requires the user to have a Gateway Node on the public network, which could bring us some potential subscription revenue.

- Our system architecture should already be running in the traditional cloud, capable of easily creating a cluster on the cloud and migrating from the cloud to the edge through the backup and recovery logic.

- We need to use GlusterFS in production environments with fine granularity. This requires us to clearly achieve our core functionality through configuration. Our self-developed dcfs-lite will be our core technical moat, specifically designed for private storage cloud scenarios: more flexible configuration, more stable operation, simpler maintenance, and better performance.

This phase is planned to be completed in 2024 through the following minor versions:

- **0.1 Demo:** 2.5% (Done)
- **0.2 PoC (Pre-Alpha1):** 2.5% (🔥🔥🔥 `Current Version`)
- **0.3 Pre-Alpha2:** 5% (First complete version)
- **0.4 Alpha:** 2.5% (Available for user testing, Q4 2024)

### Phase 2: Integration of AI (Beta, Q2 2025)

System integration with the OpenDAN Framework, with hardware integration with security (AI) cameras.

- Integrate the AI Compute Kernel functionality required by OpenDAN, upgrading from a Personal Server to a Personal AI Server, testing our system's scalability.
- Meet the demand for new capabilities combining traditional AI with GenAI. Our Library application should support AI management of vast multimedia files.
- Integrate with traditional security cameras, achieving more privacy-controlled and long-term data storage, and perform semantic analysis of large amounts of raw data based on AI.

### Phase 3: Perfecting Key Application Development (Official Version, Q2 2026)

AI-driven media processing. Support for download, streaming, and other traffic services.

- Organizing and processing photos and videos is an eternal theme. With the foundation of data storage and AIGC, we can collaborate with more digital image processing teams to create the next-generation personal media center.
- NAS must have download capabilities.
- Strong RTC capabilities are the foundation for a future local AI Agent with real-time sensing capabilities. Leveraging P2P expertise, we will continue to integrate BDT based on practical scenarios.
- Transmission proof + Tokenization of BitTorrent, enabling seed file sharing, download acceleration, and Tokenization of uploads.
- Traditional VPN business, supporting PCDN business through transmission proof (cyfs-gateway downward compatibility with today's Clash ecosystem), aiming to attract a large number of users from this vast existing ecosystem.
- Cloud hosting: Further expanding the cloud hosting ecosystem based on VPN. Users can not only connect to their devices more easily via RDP but also securely lend out their limited devices.
- This phase focuses on new application development, striving to complete it through external collaboration.

## Overall Plan for the Alpha Phase

Functions marked with `*` are those I believe must be completed in the next release version 0.2 (PoC). Functions without `*` may depend on some basic components.

- Kernel Models
  - [x] *node_daemon (A4 @waterflier)
    - [x] *app & service loader (A4 @waterflier), implement formal permission management and container isolation
    - [ ] node task execute system (A4), usually used for maintenance tasks; implement if unavoidable
  - [x] system config service(A2 @waterflier)
    - [ ] *Support etcd in more than 3 OOD clusters through scalable backends (A2)
    - [x] *system-config lib (A2 @waterflier)
    - [x] *rbac libs (A4 @waterflier), basic rbac usage and management
    - [ ] *system config event support (A2 @waterflier), use websocket for config-change notification
  - system status for system status monitoring
  - kRPC @waterflier
    - [x] *rust kRPC libs (A4)
    - [ ] *typescript kRPC libs (A2)
    - [x] *verify_hub service  (A2)
      - [ ] improve user and device register logical. (A2)
  - kLog, a reliable logging library, is the foundation for automatic fault diagnosis in the system.
    - [ ] *kLog lib (A4), defines the basic interfaces for kLog output and reliable behavior logic, can handle server downtime
    - [ ] *kLog server (S2), PoC version should implement a simple version to ensure reliability
  - kMQ message queue, supports custom event systems
  - pkg system
    - [x] *Improve lib (waterflier,S2) to facilitate use by other components
    - [ ] Integrate with the task system (A4)
- Kernel Services
  - [x] *scheduler (A2 @waterflier), a key module to be implemented in the PoC version, automatically generating node_config and establishing an initial extensible framework
    - [x] *boot scheduler (A2), the first scheduler to be implemented, mainly for system initialization
    - [ ] *Making scheduling logic for single OOD(A2)
    - [ ] *Implement the scheduling logic for multi-OOD(A2)
    - [ ] *When single OOD scale to multiple OOD, realize the scheduling logic with OP task(A4)
  - [ ] *Task Manager (A4), providing a general stateful background task management service, supporting reliable execution of critical tasks
  - DFS
    - [ ] *glusterFS (A2) integrated with rbac
    - DCFS (listed separately)
  - dApp manager, the `apt` tool in BuckyOS, provides basic reliable pkg management capabilities for the system.
    - [ ] *basic API support (A4), source management, installed management, permission configuration, installer
    - [ ] *CLI tools (S4), command-line tools similar to apt based on basic API
    - [ ] *in-zone pkg repo service (S4), a stable repo service running within the zone
    - [ ] *Installer UI (A4), meets the minimum requirements for dApp permission control in the system
  - backup system (listed separately)
  - cyfs-gateway (listed separately)
- Frame Services
  - [ ] *smb-service (A2), integrated with ACL
  - [ ] k8s-service, integrated with ACL
  - [ ] *http-fs-service (A2), integrated with ACL
  - [ ] Notify Manager
  - [ ] *msg_bus(S4), the user's system inbox, where all applications can send messages
  - [ ] dApp Store
  - [ ] *Control panel (S8) Provide basic system management functions according to the requirements document (including related web pages), can be further divided.
- Demo dApps 
  - [ ] *Home Station App(A4),home page app for user.
- [ ] *CyberChat App (A4), BuckyOS control app modified based on the CYFS wallet app.
  - [ ] *Account management (S4), mainly local DID account management, much logic can be reused from CYFS wallet
  - [ ] *Name management (S4), manage friendly names owned
  - [ ] *Zone management (S2), support relevant requirements of the first version
  - [ ] *Storage management (S2), pure display in the first version
  - [ ] *File UI (S4), provide basic UI to access DFS
- Web2.5 Services
  - [ ] *BuckyOS Web general lib design (A4 @waterflier), the basic library used by all web pages
  - [ ] *Account management + signature service (A2), including signature history
  - [ ] *did resolution and name resolution (S2), mainly implemented in cyfs-gateway, this mainly handles formal online operations
  - [ ] *Name application and management (S4), allowing users to easily and freely own a $name.buckyos.org name, obtained during account registration
  - [ ] gateway service (subscription management)
  - [ ] *http backup server (S2), mainly functions from BuckyOS Backup Suite, simple online operation at first (with size restrictions), followed by subscription implementation
  - [ ] Cloud zone support: two research approaches
  - [ ] Support migration from Web2.5 to Web3
- [ ] *BuckyOS Backup Suite (independent product with separate points, additional rewards from the DMC fund), an independent cross-platform backup software, refer to its independent PRD.
  - [ ] Backup Service
  - [ ] UI
  - [ ] Backup basic libs
  - [ ] http DAV target client
  - [ ] http DAV target server
  - [ ] dmc target client
  - [ ] dmc target server
- [ ] *CYFS Gateway (independent product with separate points), detailed requirements document is still being written
  - [ ] *Support buckyos demo needs: TAP Device-based VPN, allowing needed services to work transparently with the main OOD in the same LAN
  - [ ] *Expose FrameService/dApp services to the outside of the Zone securely via reverse proxy
  - [ ] *Establish a basic framework, including rule engine, tunnel management, and support for reverse proxy https server
  - [ ] *cyfs-nameservice service, supporting our name system and did-document system
- CI/CD Support
  - [ ] *Nightly CI/CD system (A4), based on Github Action
  - [ ] *Rapid cloud development environment setup (A4), referencing Github's CodeSpace
  - [ ] Rapid independent CI/CD environment setup based on specific branches
  - [ ] Set up typical responsible test environments
- DCFS Formal architecture design is ongoing based on the Demo phase research results.
- SDK Initially integrated according to internal needs, formal planning will begin in version 0.3.

(TODO: Continuously improve the brief introduction of each module)

## Project Management Process

0.2 PoC is the first relatively official version of BuckyOS. To ensure quality, we have formulated the following project management process based on the actual situation:

### Step 1: Read Existing Documents and Discuss

- Understand the requirements of the current version
- Understand the overall system architecture of BuckyOS
- Understand the current version's plan, module division, functional boundaries of each module, and implementation ideas

If you have any questions while reading the documents, you can discuss them by creating an issue.

The issue should contain a clear question, and you should try to provide your own understanding or opinion before the discussion.

### Step 2: Apply to Become a Module Lead

Understand the potential benefits of becoming a project lead. This document has listed the expected points for relevant modules (which is usually the minimum difficulty of the component). You can choose components that you are proficient in or that offer higher rewards for further research, based on BuckyOS DAO Rules to estimate expected rewards.
  
Once you have decided on the module you want to work on, you can write a `proposal.md`. The document should contain two main sections, with no length limit, and should be as concise as possible:

1. What this module will do (functional boundaries)
2. How you plan to implement it and approximately how long it will take

Since multiple people may apply for the same module, you can also highlight your advantages in becoming the lead in this document. After writing it, initiate a PR and wait for the version lead's review.

### Step 3: PR Review

The version lead (or other authorized long-term contributors) will discuss with the applicant in the PR through comments. Once an agreement is reached, the PR will be merged, and the PR submitter will become the module lead. If multiple people apply for the same module, the version lead will choose one person to become the lead based on the discussion results.

### Step 4: Write a More Detailed Plan

After becoming a module lead, the lead needs to continue submitting the `plan.md` document (no longer required through PR). This document should break down the plan in as much detail as possible and provide a more accurate time estimate. It can also adjust the points for this module according to the plan. The `plan.md` should include:

1. Necessary document writing plan
2. Development plan
3. Testing plan

The module lead or version lead may initiate an issue to discuss the `plan.md`. After the discussion, the version lead will usually update the module points in the version plan (this document).

### Step 5: Enter Development and Testing Phase

Once in the development phase, the module lead can develop in their branch. We also allow direct development on the main branch. However, note that submissions on the main branch must pass the nightly CI/CD tests. Submissions that do not pass the test will be DISSed by everyone~

The version lead will update the current version chart on the Github Project once a week and provide risk warnings. The module lead can proactively update the status on the Project.

### Step 6: Announce Development Completion

After the module lead believes the module has been completed and passed testing, they can submit a `test_report.md` in the PM directory, indicating that the component has been completed and is waiting for the version release.
At this stage, focus on bug issues and resolve them promptly.

### Step 7: Version Release and Settlement

After all modules are completed, the version lead will push for integration and experience testing. When the version lead feels it is appropriate, they will announce the version Release. If no major issues arise within a week, it will enter the DAO settlement stage according to the rules. After settlement, everyone can receive the corresponding BDT rewards based on their actual points.

## Discussion

If you have any questions or suggestions about this document, please discuss them in the following issue:

[https://github.com/buckyos/buckyos/issues/15](https://github.com/buckyos/buckyos/issues/15)
