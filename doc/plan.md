# BuckyOS Current Development Plan Beta

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
- **0.2 PoC :** 2.5% (Internal Release ,Done)
- **0.3 Pre-Alpha1:** 2.5% (Done!)
- **0.4 Alpha2:** 2.5% (Done!,Delay to Q1 2025)

### Phase 2: Integration of AI (Beta, Q2 2025)
System integration with the OpenDAN Framework, with hardware integration with security (AI) cameras.

- **0.5 Alpha3:** 2.5% (🔥🔥🔥 `Current Version` Available for user testing, Q2 2025)
  - Integrate the AI Compute Kernel functionality required by OpenDAN, upgrading from a Personal Server to a Personal AI Server, testing our system's scalability.
- **0.6 Beta1:** 2.5% (Q2 2025)
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

All completed tasks are the task of next release veriosn : Alpha2 (0.4.0).
Functions marked with `*` are those I believe must be completed in Alpha2. Functions without `*` may depend on some basic components.

- DFS (wugren & photosssa)
  - [ ] Select the underlying solution and conduct research on key needs(A2,@wugren,@photosssa)
  - [ ] DFS (A1) integrated with rbac
  - [ ] DFS Support soft RAID: 4 hard disks can damage any hard disk without losing data (A2)
  - [ ] DFS expand from 1 node to 2 node (A2)
  - [ ] DFS expand from 2 node to 3 node (A2)
  - [ ] DFS Support SSD Read/Write Cache (A2)
  - DCFS (listed separately)

- BuckyOS Backup Suite (independent product with separate points, additional rewards from the DMC fund), an independent cross-platform backup software, refer to its independent PRD.
  - [ ] *Backup Suite Framework (A4,@waterflier)
  - [ ] *Backup basic libs (A1,@waterflier)
  - [ ] *General high-performance dir backup source (A2,@photosssa, @waterflier)
  - [ ] *Web UI（S3，@streetycat,@waterflier）
  - [ ] DMC Backup Target (Alpha3) (A6,@photosssa)
  - [ ] http DAV target server
  - [ ] *Installation package (A1,@streetycat)
  - [ ] *Integrated with BuckyOS (S1,@waterflier)9

- Port Apps:Integrated BuckyOS SDK, integrated single sign-on
  - [ ] Sync Drive App : seafile?
  - [ ] Photo Library: ?
  - [ ] Video Library:Jellyfin
  - [ ] Music App: ?
  - [ ] Note App: ?
  - [ ] Download tools: Xunlei,qBittorrent
  - [ ] Dev tools: VSCode-Server, Jupyter lab
  
- *BuckyOS offical website (S2)



## Project Management Process

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
