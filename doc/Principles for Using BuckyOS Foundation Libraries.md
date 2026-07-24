# Principles for Using BuckyOS Foundation Libraries

> Status: Draft  
> Scope: The BuckyOS Rust workspace, related repositories, and all contributors and reviewers who add or modify foundation library dependencies.  
> In this document, “foundation libraries” include both third-party crates introduced through Cargo and reusable foundation code developed within BuckyOS.

## 1. Purpose

Rust and Cargo make it easy to introduce third-party libraries. This convenience improves development efficiency, but it can also cause dependency choices to gradually become a matter of personal preference: a developer may introduce a new crate based on past experience, a familiar API, a tutorial example, or a temporary requirement, while that crate brings in another set of transitive dependencies.

Without consistent constraints, this behavior gradually leads to the following system-wide problems:

1. **Continuously increasing compilation costs.** A growing dependency tree increases clean build, incremental build, and CI build times, and also enlarges final artifacts.
2. **Multiple versions of the same crate.** In common BuckyOS build configurations, Rust crates are typically linked statically into final artifacts. Different versions may coexist, each with its own global variables, registries, caches, or initialization state. This can lead to duplicate initialization, inconsistent state, or runtime problems that are difficult to diagnose.
3. **Increased software supply chain security risk.** Pinning old versions exactly, accumulating excessive transitive dependencies, or relying on confusing upstream versioning policies can prevent security patches from reaching the system promptly.
4. **System-wide engineering quality diluted by individual choices.** Using multiple libraries to implement the same capability increases the cost of understanding, maintenance, replacement, and review.

The goal of this specification is neither to eliminate third-party dependencies nor to impose a burdensome approval process on every dependency. Its goal is to keep BuckyOS foundation libraries:

- consistent;
- explicit;
- stable;
- reviewable;
- reproducible;
- upgradable;
- capable of sustainable evolution.

## 2. Normative Language

This document uses the following requirement levels:

- **MUST**: Must not be violated unless an explicit exception process has been completed.
- **SHOULD**: The default practice; any deviation must be explained in the PR.
- **MAY**: Optional depending on the specific circumstances.

## 3. Core Principles

### 3.1 Standardize First, Then Choose What Is Better

For any given system-level foundation capability, BuckyOS should retain only one shared choice.

For example, once the system has standardized on an asynchronous runtime, serialization framework, HTTP implementation, database access library, logging system, or cryptographic foundation library, new code should continue using that choice. It must not introduce a functionally overlapping library merely because of personal preference.

This principle can be summarized as:

> **Standardize first, then optimize. A better library should replace the existing choice rather than silently coexist with it.**

The purpose of standardization is not to prove that the current choice is theoretically optimal. It is to establish a consistent engineering baseline across the system. The team may continue discussing better alternatives, but the outcome should be a planned system-wide migration, not different modules using different solutions indefinitely.

Therefore:

- When a shared choice already exists for a capability domain, a new PR MUST use it.
- A PR introducing a functionally overlapping library SHOULD be rejected unless an explicit exception already exists.
- A local PR MUST NOT introduce an alternative library first and then use that fait accompli to push for system-wide adoption.
- Changing a shared choice MUST begin with an Issue or Discussion, followed by a migration plan.

### 3.2 Stability Comes First When Admitting Third-Party Libraries

When only one usable implementation exists for a capability, the choice may be binary: the system must use it or the functionality cannot be delivered. Even then, its use should be limited in scope, its risks documented, and the conditions for future replacement considered.

When multiple candidate libraries exist in the same domain, prefer the implementation with better engineering quality and lower long-term risk. Pay particular attention to the following aspects.

#### 3.2.1 A Clear Vision and Scope

The library should clearly state which problems it solves and which problems it does not solve. Its positioning should not change frequently in response to community trends.

#### 3.2.2 A Predictable Release Cadence

Maintainers should release versions with care, avoiding meaningless high-frequency releases or arbitrary breaking changes in compatibility releases. Version updates should primarily serve bug fixes, security fixes, performance improvements, compatibility improvements, and necessary enhancements within the library's defined scope.

#### 3.2.3 A Credible Record of Backward Compatibility

A mature library should treat its public API seriously. When incompatible changes are necessary, they should be managed through explicit major versions, migration guidance, and reasonable transition arrangements.

#### 3.2.4 Sufficient Real-World Usage and a Strong Quality Reputation

User count is not the only criterion, but broad and sustained real-world usage helps expose edge cases, platform differences, and hidden defects. Consider maintenance responsiveness, historical critical defects, the ability to handle security incidents, and the consistency of release quality.

#### 3.2.5 Tests That Cover Real Boundaries

A candidate library should have reliable tests, especially for error paths, extreme inputs, concurrent behavior, cross-platform differences, compatibility, and historical regressions. Test quality often reveals whether maintainers truly understand the boundaries of their library.

#### 3.2.6 A Clear Signal for Stable Versions

Upstream maintainers should clearly tell users which version series is suitable for stable use and what compatibility and security maintenance that series will receive. This signal may take the form of an LTS release, a stable channel, a long-term maintenance branch, an officially recommended major version, or an equivalent mechanism.

If a library can only be trusted to work when users pin one exact version, it generally lacks a credible compatibility commitment and release policy. Unless no alternative exists, such a library should not become a long-term shared dependency of BuckyOS.

### 3.3 Clear Boundaries Do Not Mean Smaller Is Always Better

BuckyOS values small, well-designed libraries, but rejects taking this idea to an extreme.

A library having clear boundaries does not mean that every function, algorithm, or small utility should become an independent crate. The traditional NPM ecosystem has demonstrated that excessive fragmentation rapidly increases dependency count, supply chain risk, compilation cost, and maintenance cost.

For mature foundation domains such as networking, cryptography, serialization, file systems, time handling, database access, compression, and concurrency, prefer domain libraries with reasonable completeness instead of introducing a separate crate for every small feature.

When evaluating scope, the domain boundaries of System libraries on mature platforms such as Java and C# can serve as useful references because those boundaries have been validated by large-scale engineering. This does not mean copying their APIs, but learning from their long-term engineering judgment about the boundaries of foundation capabilities.

Before introducing a small crate, first determine:

> Does this functionality truly constitute an independent domain, or is it merely an ordinary component of a mature domain?

For simple string processing, byte conversion, retry loops, format validation, small data structures, or a small amount of general-purpose logic, consider the following options in order:

1. Whether the Rust standard library already provides it;
2. Whether the current shared domain library already provides it;
3. Whether a BuckyOS internal foundation library already implements it;
4. Whether it is worth implementing locally;
5. Whether introducing a new supply chain node is truly justified.

### 3.4 Dependency Declarations Follow Upstream; Release Builds Are Reproduced Through the Lockfile

When BuckyOS declares a direct dependency, it SHOULD use a reasonable compatible version range, usually with Cargo's caret or partial version requirements. As a rule, exact version pinning should not be used.

Recommended:

```toml
some-crate = "1.4"
```

Generally avoid:

```toml
some-crate = "=1.4.7"
```

Exact pinning MAY be used only when a specific compatibility problem exists and an exception has been recorded. The conditions for removing the pin MUST also be documented.

The goal of this policy is to track compatible upstream changes early, so that security patches, compatibility changes, and potential breakage are exposed in CI as early as possible instead of remaining hidden behind old versions for long periods.

`Cargo.toml` and `Cargo.lock` have different responsibilities:

- `Cargo.toml` expresses the range of compatible versions accepted by the project.
- `Cargo.lock` records the complete dependency graph actually resolved for a particular build.

Therefore, not pinning an exact version in `Cargo.toml` does not mean abandoning reproducible builds. Official CI results MUST preserve and associate:

- the source code commit;
- `Cargo.lock`;
- the Rust toolchain version;
- critical build configuration;
- the final build artifacts.

CI SHOULD support both of the following:

1. Reproducible builds using the established `Cargo.lock`;
2. Periodic refreshes within compatible version ranges to detect upstream changes early.

### 3.5 Reusable Code Follows a Three-Layer Model

Not all reusable code should be obtained through external dependencies. BuckyOS divides reusable code into three layers.

#### Layer 1: External General-Purpose Domain Libraries

Third-party libraries introduced through Cargo should primarily provide foundation capabilities that are independent of the BuckyOS product, have mature domain boundaries, and are reusable by a broad range of projects.

#### Layer 2: Shared Libraries Within a Wrapper or Subsystem

If code is reused only within a particular wrapper, subsystem, or group of adjacent modules, it should be placed in a shared library within that scope. This limits the impact of changes and avoids designing system-level abstractions prematurely.

#### Layer 3: BuckyOS Base

When a capability has been proven to be shared by multiple wrappers, multiple repositories, or the entire system, moving it up into BuckyOS Base may be considered.

BuckyOS Base is intended to deliberately consolidate:

- shared code across wrappers;
- shared capabilities across repositories;
- system abstractions specific to BuckyOS;
- infrastructure closely tied to the BuckyOS runtime environment;
- foundation components of the future System Framework.

At present, BuckyOS Base primarily serves the internal ecosystem and does not guarantee strict external API compatibility. However, it must still maintain clear boundaries and must not become a miscellaneous collection of arbitrary helpers.

Shared code should move up through the layers according to actual reuse requirements:

> Current module → shared wrapper/subsystem library → BuckyOS Base → if necessary, extraction into a true general-purpose domain library.

### 3.6 Wrappers Should Isolate Only Real Instability

Do not add a Wrapper around a third-party library by default merely because its Provider might be replaced in the future.

The following reasons are insufficient to justify a new Wrapper:

- The team has not yet decided which library to choose.
- Different developers have different personal preferences.
- There is a theoretical concern that a mature library might change its interface in the future.
- All third-party APIs are assumed to require isolation.
- The code is intended only to look more architecturally sophisticated.

For a mature domain such as SQLite, once a reliable Rust library has been selected, it should be used directly and consistently. A thin abstraction with no independent system semantics usually does little more than duplicate the API, lose access to underlying capabilities, and increase maintenance risk.

Typical cases in which a Wrapper may be appropriate include:

- The upstream domain is very new.
- Multiple candidate libraries are all in rapidly changing `0.x` stages.
- APIs, data models, or platform support change frequently.
- No candidate library demonstrates sufficient maturity.
- The capability is nevertheless critical to BuckyOS and will be used in multiple places.

In these cases, the purpose of the Wrapper is to isolate real instability, not to avoid making a technical choice. It MUST be designed around the stable semantics BuckyOS actually needs and clearly define:

- the specific risks that need to be isolated;
- the minimum supported capability boundary;
- whether Provider replacement is genuinely feasible;
- the testing strategy;
- whether the Wrapper is permanent or transitional;
- the conditions under which the Wrapper can be removed or its implementation fixed.

This leads to two basic patterns:

1. **For mature, stable capabilities with clear domain boundaries: standardize on an excellent third-party library and use it directly.**
2. **For capabilities on which the system depends heavily but whose upstream ecosystems remain immature: isolate real instability through an internal Wrapper.**

### 3.7 Exceptions Are Allowed, but They Must Be Explicit, Visible, and Traceable

Every principle has exceptions. When the system requires a special capability that the existing shared choice genuinely cannot provide, and no other reasonable solution exists, an exceptional dependency may be introduced.

A legitimate exception is usually an explicit binary requirement—a capability either exists or it does not—not an API preference, tutorial habit, or unverified assumption about performance.

Every exception MUST meet all of the following requirements:

1. Explain the reason in a comment in the corresponding `Cargo.toml`.
2. Explain why the existing shared choice cannot satisfy the requirement.
3. Limit the scope of use.
4. Mark experimental or temporary status and known risks.
5. Define the conditions for removal or reevaluation.
6. Record it in the centralized exception registry.
7. Continue tracking it in subsequent periodic reviews.

Example:

```toml
# Exception:
# Used only for resumable hashing of very large files.
# The workspace-standard hashing library does not expose serializable
# intermediate state. Remove this dependency when resumable hashing is
# no longer required by the chunking strategy.
resumable-hash = "0.x"
```

Introducing an experimental library for checkpoint-based streaming hashes of very large files may once have been reasonable. If the system later limits chunk sizes and no longer requires resumption, the exception should naturally be removed.

> **An exception is not permanent authorization. Exceptions are temporary by default and should be removed when the requirements that justified them no longer exist.**

## 4. Dependency Inventory and Continuous Governance

### 4.1 Maintain a Registry of Known System Dependencies

BuckyOS SHOULD maintain a public registry of known system dependencies organized by capability domain. It should include at least:

| Field | Description |
|---|---|
| Capability domain | Asynchronous runtime, serialization, database, logging, cryptography, and so on |
| Current shared choice | The crate or internal foundation library used consistently |
| Scope of use | Entire system, specific wrapper, or specific repository |
| Status | Standard, Experimental, Deprecated, and so on |
| Version policy | Recommended major version, stable branch, or other constraints |
| Upstream status | Maintenance activity, stability signals, and significant risks |
| Multiple versions | Whether multiple versions exist and why |
| Migration status | Whether an upgrade, replacement, or removal is planned |
| Notes | Design background and important limitations |

This registry should gradually answer not only “what is the system using?” but also “why was it selected?”

### 4.2 Maintain an Exception Dependency Registry

Exceptional dependencies should be recorded centrally. The registry should include at least:

| Field | Description |
|---|---|
| crate | Name of the exceptional dependency |
| Location | Repository, wrapper, or crate where it is used |
| Introducing PR | Provides traceability to the original context |
| Reason for exception | The specific capability that cannot be replaced |
| Scope of use | Limits the impact |
| Risks | Experimental status, maintenance status, security risk, or compatibility risk |
| Removal conditions | When it can be removed |
| Owner | Current person responsible for follow-up |
| Latest review | Date of the most recent reconfirmation |

Comments in `Cargo.toml` answer “why is it used here?” The centralized registry answers “how many exceptions remain across the system?”

### 4.3 Ordinary Additions Go Through PRs

At the current stage, a complex pre-approval process is not required for every third-party library.

If a new dependency complies with the principles in this document, it may be introduced through a normal PR. The PR MUST:

- explain the new capability and its scope of use;
- confirm that no shared choice already exists, or explain the exception;
- examine the impact of direct and transitive dependencies;
- use the correct version policy;
- update the registry of known system dependencies;
- update the exception dependency registry when necessary.

### 4.4 Changes to Shared Choices Must Be Discussed First

Migrating from library A to library B, or replacing the system-wide asynchronous runtime, serialization framework, database access library, cryptographic implementation, or another shared foundation capability affects multiple crates, existing code, and release artifacts. Such changes MUST first be discussed through an Issue or Discussion.

The discussion should cover at least:

- the real problems with the current choice;
- the necessity and urgency of replacement;
- the advantages and risks of candidate solutions;
- the scope of migration impact;
- compatibility, performance, and security changes;
- how to avoid long-term coexistence of the old and new solutions;
- the complete migration and verification plan.

After a conclusion is reached, the system-wide migration can be completed through one or more PRs.

### 4.5 Periodic Reviews

Dependency governance is an ongoing responsibility, not a one-time cleanup.

As a general rule:

- **Run an automated scan once a week.** Fixed scripts and AI-assisted checks can detect new dependencies, multiple versions, exact pins, security advisories, upstream maintenance status, growth in transitive dependencies, changes in compilation time, and changes in artifact size.
- **Conduct a manual review once a month.** Focus on functionally overlapping libraries, expired exceptions, persistently outdated versions, unmaintained upstream projects, unusual dependency growth, and whether a replacement discussion should be initiated.

AI can assist with summarization and risk discovery, but final decisions should still follow the normal engineering review process.

## 5. Basic Criteria for PR Review

When reviewing a PR that adds or modifies dependencies, reviewers should focus on the following questions:

1. Does a shared choice already exist for this capability?
2. Is the new dependency truly an independent, general-purpose domain capability?
3. Is a crate being introduced merely for one small function?
4. Does upstream have clear boundaries, a stable cadence, adequate tests, and a compatibility commitment?
5. Does it introduce many unnecessary default features or transitive dependencies?
6. Does it cause multiple versions of the same crate to coexist?
7. If multiple versions exist, do they create risks involving global state, registries, runtimes, or initialization?
8. Is the version requirement pinned too tightly?
9. Is a Wrapper being used incorrectly to avoid making a shared choice?
10. Does an exception have explicit semantics, a limited scope, and removal conditions?
11. Has the system dependency registry been updated accordingly?

## 6. Quick Checklist (Before Contributors Submit a PR)

Use the following checklist to quickly determine how a foundation library should be used. Before submitting a PR involving dependencies, confirm each item.

### A. First Confirm That a New Dependency Is Truly Necessary

- [ ] The Rust standard library cannot provide the functionality directly.
- [ ] Existing workspace dependencies or BuckyOS Base cannot provide the functionality.
- [ ] This is not a “small function crate” containing only a small amount of simple code; a local implementation would not be significantly simpler or safer.
- [ ] The functionality has a reasonable, complete domain boundary rather than being an excessively fragmented component of a mature domain.

### B. Check Whether a Shared Choice Already Exists

- [ ] The registry of known system dependencies has been consulted.
- [ ] No shared choice exists for this capability domain, or this PR uses the existing shared choice.
- [ ] No functionally overlapping library has been introduced because of personal habits, tutorial examples, or API preferences.
- [ ] If the shared choice is to be replaced, an Issue or Discussion has already been opened and a system-wide migration plan exists.

### C. Evaluate Candidate Library Quality

- [ ] The library's vision and scope are clear.
- [ ] Its release cadence is predictable, without frequent or arbitrary breaking changes.
- [ ] It has a credible record of API backward compatibility or a clear stable/LTS signal.
- [ ] Its tests, documentation, and error handling are sufficiently reliable.
- [ ] Upstream is still maintained and responds acceptably to security issues.
- [ ] The size of its dependency tree is proportionate to the value of the functionality.

### D. Check Cargo Usage

- [ ] The dependency is preferably declared and reused consistently through workspace dependencies.
- [ ] Only the features actually needed are enabled, avoiding unnecessary default capabilities.
- [ ] A reasonable compatible version range is used, without an unjustified `=x.y.z` requirement.
- [ ] `cargo tree` has been checked to confirm there are no unexpected duplicate versions or unusual transitive dependencies.
- [ ] If multiple versions exist, risks involving global variables, registries, runtimes, caches, and initialization have been evaluated.
- [ ] Official CI results can be associated with the corresponding `Cargo.lock`, toolchain, and build configuration.

### E. Determine the Appropriate Layer for the Code

- [ ] A third-party library is introduced through Cargo only for a mature, general-purpose domain capability that is entirely independent of the product.
- [ ] Code reused only within a wrapper or subsystem is placed in a shared library within that scope first.
- [ ] Shared system capabilities spanning wrappers or repositories are moved up into BuckyOS Base only after discussion.
- [ ] BuckyOS Base is not treated as an unbounded collection of helpers.

### F. Confirm Whether a Wrapper Is Truly Necessary

- [ ] The Wrapper is not used to avoid choosing a library or to accommodate personal preferences.
- [ ] The Wrapper isolates real upstream instability, not a theoretical possibility of future replacement.
- [ ] The Wrapper defines the stable semantics BuckyOS itself needs instead of merely forwarding a third-party API.
- [ ] The Provider replacement strategy, testing approach, and exit conditions are documented.

### G. Review Exceptions

- [ ] Any departure from the shared choice or version policy addresses an explicit, irreplaceable binary requirement.
- [ ] The reason for the exception is documented in a comment in `Cargo.toml`.
- [ ] The scope of use, risks, and removal conditions are documented.
- [ ] The exception dependency registry has been updated, with responsibility assigned for subsequent reviews.

### H. Final Checks Before Submission

- [ ] This PR updates the registry of known system dependencies.
- [ ] The compilation time, artifact size, and supply chain costs introduced by the new dependency are acceptable.
- [ ] Reviewers can understand the choice from the code, comments, and registries alone, without relying on verbal context.
- [ ] When the future requirement disappears or upstream matures, the dependency, exception, or Wrapper can be removed through a clearly defined process.

If any critical item above cannot be confirmed, use the existing shared choice, reduce the implementation scope, or open an Issue or Discussion instead of adding a new dependency directly to the system.

---

**The principle in one sentence:**

> **Standardization takes priority over local optimization; stability takes priority over novelty; choose complete domain libraries over fragmented dependencies; use `Cargo.lock` to ensure reproducible builds and continuous CI to track upstream changes; use Wrappers only to isolate real instability; and make every exception explicit, visible, and removable.**
