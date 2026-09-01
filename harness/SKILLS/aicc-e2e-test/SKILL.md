---
name: aicc-e2e-test
description: Implement, review, or execute BuckyOS AICC T1/T2/T3 E2E acceptance tests, especially when real Providers, message entrances, credentials, costs, or temporary AICC settings are involved.
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

Static preflight, unit/self-tests, report inspection, and a dry-run that neither contacts the DV environment nor mutates it do not require this authorization. T1 Mock execution follows its own explicit configuration-mutation guard and must not make real Provider calls.

The runner's `--yes` option is for already-authorized automation; it does not grant a CodeAgent permission to execute T2/T3. This skill constrains CodeAgent actions, not CI or a human operator running commands directly.

## Execution invariants

- Use the Zone Gateway and real authentication path; do not call service ports directly.
- Keep T2 within the Provider and instance scope selected by the user. Keep T3 within the approved Provider and message-entrance scope; a configured T3 instance is for credential injection and audit, not forced routing.
- Before real calls, print and enforce call, retry, timeout, and budget limits. T1/T2 also enforce global and per-Provider concurrency and minimum request intervals; T3 only enforces scenario concurrency.
- Preserve secrets in local ignored TOML only and redact them from commands, logs, and reports.
- Restore temporary settings and resources, then report cleanup results, actual calls, cost, failures, and a targeted retest command.
- When a failure is confirmed as an AICC/Jarvis defect, keep the test assertion correct and record expected behavior, observed behavior, and evidence instead of changing product code without a separate request.
