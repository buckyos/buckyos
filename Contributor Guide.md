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
- `doc/` directory: Canonical engineering documentation. Treat this as the current source of truth for architecture, module behavior, API/spec details, developer guides, and operational constraints. Representative entry points include:
  - [doc/arch/README.md](./doc/arch/README.md): The architecture reading entry for the current system.
  - [doc/control_panel/README.context.md](./doc/control_panel/README.context.md): Canonical control panel entry.
  - [doc/message_hub/README.context.md](./doc/message_hub/README.context.md): Canonical message hub entry.
- `product/` directory: Product positioning, PRD-style planning material, roadmap context, and historical product intent. These documents are important background, but they are not the default engineering source of truth.
- `proposals/` directory: Pending changes and proposals that are being prepared but not yet considered current behavior.
- `notepads/` directory: Lightweight review notes, investigations, and draft analysis. Valuable ideas may start here, but content here is not canonical until promoted into `doc/`, `product/`, or `proposals/`.

## Join Us as a Long-Term Contributor

According to Git conventions, your first code submission is likely to start as a PR. BuckyOS has a complete CI/CD workflow. Please ensure your code has passed the CI/CD checks before submitting a PR. PRs are usually reviewed by the relevant module leader for the current version, who will merge them after review and complete the BDT rewards on the version's settlement page.

We welcome everyone to become long-term contributors. If you want to lead a module or a larger change, start by writing a proposal under `proposals/`, or by promoting a well-formed note from `notepads/` into `proposals/`. Explain what you plan to do, the scope, the risks, and how you plan to validate it. After the version lead or module lead reviews and accepts it, you can proceed as the responsible contributor for that area. According to BuckyOS's DAO rules, module leads have more responsibilities and power, and they will also receive more BDT rewards. All module leads automatically become long-term contributors after the version is released.
