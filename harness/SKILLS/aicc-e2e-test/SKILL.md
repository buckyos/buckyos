---
name: aicc-e2e-test
description: Implement, review, or execute BuckyOS AICC T1/T1.5/T2/T3 E2E acceptance tests, especially when Provider protocol mocks, real Providers, message entrances, credentials, costs, or temporary AICC settings are involved.
---

# AICC E2E Test

Use `doc/aicc/aicc_e2e_test_requirements.md` as the normative test specification and `test/aicc_test/acceptance/README.md` for runner commands.

## CodeAgent execution authorization

Before a CodeAgent starts any T2 or T3 test execution, obtain explicit authorization for that execution from the user. A checked-in config, an enabled `allow_real_model_calls`, credentials already present on the machine, a previous run, or approval from an earlier turn is not authorization for a new execution.

Authorization is sufficient when the user's current request explicitly asks to execute the relevant T2/T3 scope. Otherwise stop before the runner command and state:

- the selected layer and case IDs or scenarios;
- the Provider drivers and message entrances that may be used;
- the maximum real model calls and cost budget;
- whether Provider credentials or AICC settings will be changed temporarily;
- whether external messages or artifacts will be created.

Do not broaden an approval to additional Providers, entrances, cases, retries, configuration changes, or a higher budget. Ask again when the authorized scope changes materially or a later run is needed.

Static preflight, unit/self-tests, report inspection, and a dry-run that neither contacts the DV environment nor mutates it do not require this authorization. T1/T1.5 Mock execution follows its own explicit configuration-mutation guard and must not make real Provider calls.

The runner's `--yes` option is for already-authorized automation; it does not grant a CodeAgent permission to execute T2/T3. This skill constrains CodeAgent actions, not CI or a human operator running commands directly.

## Execution invariants

- Use the Zone Gateway and real authentication path; do not call service ports directly.
- Build each T1.5 Provider mock and its expected wire fixtures only from that Provider's official API documentation, official schema, official SDK protocol definitions, and official error documentation. Do not derive expected protocol behavior from AICC design documents, metadata, implementation code, existing request logs, or existing mocks.
- Keep T2 within the Provider and instance scope selected by the user. Keep T3 within the approved Provider and message-entrance scope; a configured T3 instance is for credential injection and audit, not forced routing.
- Cover every independently callable metadata variant in T1.5 as its own protocol cell, including Provider model/options lowering and response parsing.
- Keep T2 coverage to the `ProviderInstance x model x API-Type` matrix. Include every active base physical model in scope, but do not add real inference cells for metadata variants. Run one minimal correctness case per cell unless an explicit rubric requires the smallest possible supplement. Do not repeat logical aliases of the same physical model, T1 routing combinations, or T1.5 wire/error coverage in T2.
- Before real calls, print and enforce call, retry, timeout, and budget limits. T1/T1.5/T2 also enforce global and per-Provider concurrency and minimum request intervals; T3 only enforces scenario concurrency.
- Preserve secrets in local ignored TOML only and redact them from commands, logs, and reports.
- Restore temporary settings and resources, then report cleanup results, actual calls, cost, failures, and a targeted retest command.
- When a failure is confirmed as an AICC/Jarvis defect, keep the test assertion correct and record expected behavior, observed behavior, and evidence instead of changing product code without a separate request.

## Development and push cadence

- During each coding round, run only the unit tests corresponding to the implementation changed in that round. Do not repeatedly build, deploy, or run T1/T1.5/T2/T3 as an inner-loop substitute for focused unit tests.
- Before each Git push, if the pushed changes update routing logic, run the affected T1 suite.
- Before each Git push, if the pushed changes update a Provider API protocol implementation, model metadata, Provider metadata, or Provider configuration, run T1.5 only for the affected Provider or Providers.
- If a push contains both routing and Provider protocol/metadata/configuration changes, run both required gates. A documentation-only or unrelated push does not acquire an E2E obligation from these rules.

## E2E automation convergence loop

When coding specifically toward T1/T1.5/T2/T3 automation, work in batches:

1. Run the selected scope and collect a batch of failures with evidence before editing implementation.
2. Classify the batch, fix the implementation issues together, then build and deploy once for that batch.
3. Test the affected batch together and record the results.
4. If implementation changed during the round, rerun the same selected scope after the round completes.
5. Continue until one complete rerun passes without any implementation change between the preceding run and that passing run.

Do not use `one failure -> one fix -> build -> deploy -> test -> accept` as the default loop. If the final rerun has only a small number of failures confirmed to be outside implementation logic, such as transient network instability, authentication failure, or insufficient account balance, the run may stop. The final report must list every such failure, affected case and Provider, supporting evidence, and why another implementation change or rerun is not required.
