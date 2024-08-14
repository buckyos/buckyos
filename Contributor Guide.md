# BuckyOS Contributor Guide

Welcome to becoming a contributor to BuckyOS! This is a quick guide to help you quickly understand BuckyOS's workflow, and we hope you can become part of our team soon!

## What Can You Gain as a Contributor?

- A sense of accomplishment by contributing to the next-generation Personal Server operating system.
- Financial benefits: BuckyOS operates as a Distributed Autonomous Organization (DAO), and contributors can earn corresponding Tokens (Ticker: BDT) as rewards. These Tokens can be seen as "shares" of the entire organization. On the one hand, these Tokens are proof of your contributions and allow you to participate in project governance according to the rules. On the other hand, BDT may become valuable in the future.

According to BuckyOS's [DAO Rules](./DAO%20Rules.md), contributors can earn Token rewards by completing tasks of varying complexity. For long-term contributors, you can estimate the number of Tokens you may receive based on the project plan. The rules also include result-oriented reward and penalty mechanisms. Completing high-difficulty tasks with high quality will earn more Tokens, while delays or low-quality work may result in Token deductions.

In addition to coding, any positive contribution to the project may earn you BDT rewards: providing ideas, helping to improve documentation, reporting bugs, etc. We welcome contributions in any form!

- The BuckyOS DAO's official website is [https://dao.buckyos.org/](https://dao.buckyos.org/), which provides a UI to execute the above rules and also has the latest information about BuckyOS DAO.

## Read the Documentation Before Coding

The BuckyOS documentation structure is as follows:

- Root directory: Contains short introductory and rule-based documents. We are very cautious about adding documents here.
- `doc` directory: Contains long documents of global design, as well as key process document templates. Currently, the documents in this directory include:
  - [BuckyOS Architecture.md](./doc/BuckyOS%20Architecture.md) + [BuckyOS.drawio](./doc/buckyos.drawio): The complete BuckyOS architecture design document, which should be read by everyone before coding. This document is very important to us, and we encourage all contributors to help maintain it.
  - [plan.md](./doc/plan.md): A project plan written by the current version lead, a key document to understand "what everyone is working on." To make it easier for everyone to understand the project's progress, each lead will also update the latest progress on the GitHub project.
- `doc/PRD` directory: Product requirements documents. To reduce historical baggage, these documents always fully describe the complete requirements for the "next" version. By checking the git log, you can see the history of requirement changes. To provide a better product experience for end-users, we encourage thorough multilingual work at the requirement stage (different languages use different versions of the requirements). Currently, our capacity only supports Chinese and English, and we also encourage contributors to contribute in this area.
- `doc/PM` directory: Contains documents related to project management process requirements for the current version. According to the BuckyOS DAO rules, each directory represents a module, and the directory contains documents like `proposal.md`, `plan.md`, and `test_plan.md` written by the module lead for that version.
- `doc/old/$version` directory: This is the old version of the `doc` directory. A standard operation after each BuckyOS release is to complete the migration of the `doc` directory. This design ensures that the documents in the `doc` directory are always up-to-date while also making it easy to view historical documents.

## Join Us as a Long-Term Contributor

According to Git conventions, your first code submission is likely to start as a PR. BuckyOS has a complete CI/CD workflow. Please ensure your code has passed the CI/CD checks before submitting a PR. PRs are usually reviewed by the relevant module leader for the current version, who will merge them after review and complete the BDT rewards on the version's settlement page.

We welcome everyone to become long-term contributors! If, after reading the current version of `plan.md`, you wish to become a leader for a specific module, you can initiate a PR and submit a `proposal.md` document under the current version's `./doc/PM/$module` directory, explaining what you plan to do and how you plan to do it. After the version leader reviews and merges the PR, you will become the module leader for the current version. According to BuckyOS's DAO rules, module leads have more responsibilities and power, and they will also receive more BDT rewards. All module leads automatically become long-term contributors after the version is released.

