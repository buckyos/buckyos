

## BuckyOS DAO Establishment

"Open source organizations have a long history and brilliant achievements. Practice has proved that an open source organization can achieve the goal of writing better code only by working in the virtual world. We believe that software development work is very suitable for DAO. We call this DAO for decentralized organizations to jointly develop software as SourceDAO." ---- from the White Paper of CodeDAO (https://www.codedao.ai)

According to the design of SourceDao, we need to determine the following key matters before the official deployment of the Buckyos DAO contract:

### DAO official website

https://dao.buckyos.org

### DAO Token Info

- Ticker: BDT (Buckyos Dao Token)
- Total Supply: 2.1 billion
- Mintable: After the official launch and the full issuance of 210 million BDT, additional tokens can be minted through a DAO governance proposal.
- Blockchain: Polygon
- Contract Address: (to be deployed)

### BuckyOS Release Plan

#### 2024

- **0.1 Demo:** 2.5% (Done)
- **0.2 PoC:** 2.5%
- **0.3 Pre-Alpha:** 5% (First complete version)
- **0.4 Alpha:** 2.5% (2024 Q4)

#### 2025

- **0.5 Beta:** 2.5%
- **0.6 RC:** 5% (First public release version)
- **0.7 First Release:** 2.5% (2025 Q3)


## BuckyOS DAO Rules Introduction

SourceDAO provides a comprehensive design for DAO-ification of open source projects. The CYFS Core Dev Team has implemented smart contracts accordingly. Using BuckyOS as an example, we outline the basic operation process. Detailed design is available in the white paper. Our approach stems from a fundamentalist perspective on open source (aligned with GPL and GNU), influenced by Bitcoin’s "man is evil if not restrained" assumption. Although some designs may seem extreme, we believe they are understandable.

(Complete rules can be viewed on the official website of Buckyos Dao)

###  Operation Process

1. **Create an organization:** Define goals, initial BDT distribution, initial members, and establish the initial Roadmap.
2. **Roadmap:** Link system maturity to token release. More mature systems release more tokens. The Roadmap outlines project phases: PoC, MVP, Alpha, Beta, Formula (Product Release), each with a BDT release plan.
3. **Development as mining:** Achieve DAO goals by advancing the Roadmap. Set plans, calculate contributions, and distribute BDT accordingly.
4. **Market behavior:** Use BDT to increase project visibility, incentivize new users, and design fission rewards.
5. **DAO governance:** BDT holders participate in governance.
6. **Financing:** Use BDT for resource acquisition.

### Staff Structure of the DAO Organization

#### Committee

The committee consists of no fewer than three members, and the number of members must be odd. Members are elected by voting on major affairs and serve for 12 months (can be re-elected). The committee is the main body for making daily decisions in the DAO, and processes regular DAO affairs by member voting. The committee needs to organize at least one formal public meeting each quarter to discuss the overall development of the DAO.

#### Alternate Members

1-3 alternate members (with priority) can be elected under the same conditions. Alternate members can participate in all activities of the committee but do not have voting rights.

#### Removal of Committee Members

Anyone can initiate a proposal (major proposal) to remove a committee member. Once the proposal is passed, the member immediately loses qualification, and the committee must elect a temporary replacement from the alternate members within 14 days (the term of the removed member is inherited). If the committee cannot implement the election (there might be an even number of remaining members), an alternate member will be selected based on priority ranking.

Committee members can also resign voluntarily. After the approval of the committee, the resignation takes effect and the member loses qualification.

#### Secretary-General of the Committee

The Secretary-General must be a member of the committee and is responsible for organizing the committee to work according to the constitution, especially in keeping written records and public work. If other committee members lose their qualifications, the Secretary-General can serve concurrently.

#### Committee Accountant

The committee appoints an accountant to handle some of the financial affairs in the regular DAO affairs. The term of office is two years. The committee accountant can receive income from the committee's budget package every month upon appointment, has no voting rights, and cannot hold other positions.

#### Market Leader

The market leader must be a committee member. The market leader is responsible for formulating marketing promotion plans and executing them.

#### CFO

The CFO must be a committee member. The main job of the CFO is to prepare budgets, design asset custody systems, and propose finance-related proposals. (Note: The CFO and the committee accountant are not the same person, and there is no hierarchical relationship.)

#### Developer

Any developer who has contributed to the BuckyOS project and has a contribution value of more than 100 will automatically qualify as a BDT Developer (lifetime term). Removal:

1. Voluntarily declare to quit.
2. The identity of a BDT Developer can be revoked through a major proposal.

#### Core Developer

BuckyOS is an open source organization, so engineers are the main members of the organization. In a sense, Core Developers are full-time participants in the DAO. They can receive a fixed income every two weeks based on the current level and DAO's financial configuration. The project manager can, in principle, assign tasks to Core Developers. Core Developers can also hold other DAO positions.

## Decision-Making Mechanism

#### Transaction Classification:

DAO transactions are classified into internal project transactions, routine DAO transactions, important transactions, and major transactions. Internal project transactions are decided by the project lead or designated responsible individuals, routine DAO transactions are decided by committee voting, and important and major transactions are decided by bidding from all BDT holders. The difference between important and major transactions lies in the minimum voting threshold (the amount of BDT available for voting). Important transactions require a minimum voting threshold of 30% of the available BDT, while major transactions require a minimum voting threshold of 40%.

#### Decision-Making Process:

Except for internal project transactions, all DAO transactions follow the following process:

1. Proposal: Committee members are eligible to initiate all proposals, while non-committee members can initiate proposals by staking BDT. The required amount of staked BDT varies depending on the type of transaction.
2. Proposals can be designed with a voting deadline (not less than 14 days, major proposals not less than 21 days). Once all committee members have voted, routine DAO transactions that require committee voting automatically produce results.
3. After the voting deadline for a proposal, there are three possible outcomes: approval, rejection, or failure to reach the minimum voting threshold. (If the proposal was initiated by staking, the staked tokens will be returned to the proposer.)
4. Some proposals are "contract proposals," such as modifying certain contract parameters. Once such a proposal is approved, it will be automatically executed.
5. For non-contract proposals that are approved, they enter the execution phase. The proposal is then handed over to designated individuals for processing.
6. After completing the proposal operations, the proposer can mark the proposal as completed.

### R&D Process Management

BuckyOS is an open-source organization, and the project development process is the primary workflow. The project development process within the DAO organization follows the principle of prioritizing efficiency in the early stages and stability and fairness in the later stages. At the level of DAO rules, we avoid designing too many detailed rules and instead delegate the implementation of specific tasks to the responsible individuals.

Based on the project management module provided by SourceDAO, explore a new open source R&D process of "open source is mining".

0. Confirming Version Leader based on committee election.
1. Module (task division): Divide the system's futures into independent modules as much as possible, the development workload should be at the level of 1 person for 2-3 weeks.
2. Discussion: Discuss whether the module division is reasonable, and design it's *BUDGET* based on the difficulty of the module and its importance to the current version (most important step)
3. Recruit module PM. The module PM is responsible for the module's test delivery: completing the set functions, constructing the basic tests, and passing the self-tests. Testing should retain at least 30% of development resources.
4. For the completed module, PM should write and publicize the Project Proposal. It contains more detailed about module goals + design ideas, participating teams (if any, there should also be a preliminary division of work within the team and calculation of contribution value), and acceptance plan design.
5. The PM completes development and self-testing. Mark the module is *DONE*.
6. Version Leader organizes the acceptance of the module (a dedicated acceptor can be appointed).
7. Version Leader organizes integration testing according to the completion situation, and the module PM fixes BUGs. The test results can be used in the nightly-channel of BuckyOS.
8. After the test passes, the Version Leader announces the version release, anyone can use this from release-chanel.
9. The committee accepts the version after the release effect. After acceptance, all participants can extract contribution rewards.

Difficulty is expressed in the mode of requirement engineer level * time (in weeks, less than 1 week is counted as 1 week). 1 week of work time is calculated as 20 hours.

## Demo current progress

- [x] System Architecture Design,@waterflier,A8
- Kernel Moels
  - [x] node_daemon,@waterflier,A8
  - Name Service System,@wugren
    - [x] Name Client,@wugren,A2
      - [x] DNS Backend,@wugren,S2
      - [ ] ENS Backend,A2
    - [x] buckycli nameservice command,@wugren,S1
  - [x] cyfs-gateway,@lurenpluto,A3
    - [x] gateway-core (socks proxy),@lurenpluto, A2
    - [x] basic tunnel support,@lurenpluto,A2
    - gateway-front
      - [x] Support EndPoint mapping,@lurenpluto,S1
    - [x] gateway-lib,@lurenpluto,S2
    - [x] buckycli gateway commands,@lurenpluto,S1
  - system config(base on etcd)  
    - [x] system-config lib,@alexsunxl,A1
    - [x] etcd installation,@alexsunxl,A2
    - [x] etcd's automatic backup and recovery,@alexsunxl,@streetycat,A2
    - [x] etcd startup phase with cyfs-gateway integration,@lurenpluto,A1
  - pkg system
    - [x] pkg system design,@waterflier,A2
    - [x] pkg_libs,@glen0125, A2
    - [x] pkg_loader,@glen0125, A1
    - [x] pkg_installer,@glen0125, A2
    - [x] pkg_repo_http_server,@glen0125, S4
    - [x] buckycli pkg commands,@glen0125, S1
  - backup system
    - [x] backup lib(client),@streetycat, A2
    - [x] backup task mgr,@streetycat,A6
    - backup servers
      - [x] http backup server,@streetycat,S2  
- Kernel Services
  - DFS
    - glusterFS
      - [x] Installation package production,@glen0125,A2
      - [x] gulsterFS core research ,@glen0125,@wugren,A4
      - [x] glusterFS and cyfs-Gateway Integration,@wugren,S2
      - [x] Integrated OpenVPN (demo only),@wugren,S2
    - DCFS (pre-research),@photosssa,@waterflier
      - [x] Infrastructure design,@waterflier,A2
      - [x] Adjust disk-map for small clusters，@photosssa,A4
      - [x] Performance research with fuse integration，@photosssa,A2
- Frame Services
  - [x] smb-service,@wugren,A1
  - [x] k8s-service,@wugren,A2
  - [ ] http-fs-service,A1
- System Tools
  - [x] buckycli,@alexsunxl,@wugren,A2
  - [x] demo install.sh,@wugren,A2
  - [x] demo quickstart.md @waterflier,A1
  - [x] docker build files,@wugren,S1
  - [ ] dev scripts,
- BuckyOS DAO System
  - [x] DAO official website,@alexsunxl,S2
  - [x] DAO Contract,@weiqiushi,A4

## Token Allocation Calculation Based on Contributions

Based on the statistics of work completed during the DEMO phase, we can calculate each individual's contribution proportion for this version according to the following rules (the DEMO does not consider the accuracy of plan execution and delivery quality factors):

- The workload of tasks is divided into A (Architect level) and S (Software Engineer level), with the number following each letter representing the required work weeks. The minimum time requirement for each task is 1 week.
- The score coefficient for A-level work is 3, and for S-level work, it is 2.
- If multiple people collaboratively complete a task (which is not encouraged), unless otherwise specified, the score is equally distributed among them.

Using these rules, we can calculate each individual's contribution for the DEMO phase.

- **waterflier**
  - System Architecture Design: 24 (A8)
  - node_daemon: 24 (A8)
  - pkg system design: 6 (A2)
  - DCFS Infrastructure design: 6 (A2)
  - demo quickstart.md: 3 (A1)

  **Total Points: 63**

- **wugren**
  - Name Client: 6 (A2)
  - DNS Backend: 4 (S2)
  - buckycli nameservice command: 2 (S1)
  - glusterFS core research: 6 (A4 -> 12/2)
  - glusterFS and cyfs-Gateway Integration: 4 (S2)
  - Integrated OpenVPN (demo only): 4 (S2)
  - smb-service: 3 (A1)
  - k8s-service: 6 (A2)
  - buckycli: 3 (A2 -> 6/2)
  - demo install.sh: 6 (A2)
  - docker build files: 2 (S1)
  - http-fs-service: 3 (A1)

  **Total Points: 49**

- **lurenpluto**
  - cyfs-gateway: 9 (A3)
  - gateway-core (socks proxy): 6 (A2)
  - basic tunnel support: 6 (A2)
  - Support EndPoint mapping: 2 (S1)
  - gateway-lib: 4 (S2)
  - buckycli gateway commands: 2 (S1)
  - etcd startup phase with cyfs-gateway integration: 3 (A1)

  **Total Points: 32**

- **alexsunxl**
  - system-config lib: 3 (A1)
  - etcd installation: 6 (A2)
  - etcd's automatic backup and recovery: 6 (A2 -> 12/2)
  - buckycli: 3 (A2 -> 6/2)
  - DAO official website: 4 (S2)

  **Total Points: 22**

- **streetycat**
  - etcd's automatic backup and recovery: 6 (A2 -> 12/2)
  - backup lib(client): 6 (A2)
  - backup task mgr: 18 (A6)
  - http backup server: 4 (S2)

  **Total Points: 34**

- **glen0125**
  - pkg_libs: 6 (A2)
  - pkg_loader: 3 (A1)
  - pkg_installer: 6 (A2)
  - pkg_repo_http_server: 8 (S4)
  - buckycli pkg commands: 2 (S1)
  - Installation package production: 6 (A2)
  - gulsterFS core research: 6 (A4 -> 12/2)

  **Total Points: 37**

- **photosssa**
  - Adjust disk-map for small clusters: 12 (A4)
  - Performance research with fuse integration: 6 (A2)

  **Total Points: 18**

- **weiqiushi**
  - DAO Contract: 12 (A4 -> 24/2)

  **Total Points: 12**



### Workload Percentage

- **waterflier: 63 / 267 ≈ 0.2360 = 23.60%**
- **wugren: 49 / 267 ≈ 0.1836 = 18.36%**
- **lurenpluto: 32 / 267 ≈ 0.1199 = 11.99%**
- **alexsunxl: 22 / 267 ≈ 0.0824 = 8.24%**
- **streetycat: 34 / 267 ≈ 0.1273 = 12.73%**
- **glen0125: 37 / 267 ≈ 0.1386 = 13.86%**
- **photosssa: 18 / 267 ≈ 0.0674 = 6.74%**
- **weiqiushi: 12 / 267 ≈ 0.0449 = 4.49%**



These percentages reflect each individual's workload proportion relative to the total project workload.

## BuckyOS Committee

According to the SourceDAO rules, an officially operating SourceDAO requires a 3-person committee. 

Once the DEMO is officially released, the BuckyOS DAO contract will go live. My proposal is to have the first committee composed of the `top three contributors` based on the DEMO phase contributions.


----------------------
Everyone is welcome to make an opinion. We plan to formally deploy a contract with the Buckyos DAO after the DEMO is released, and to issue BDT to the contributor of the DEMO stage according to the contribution ratio of the DEMO stage.