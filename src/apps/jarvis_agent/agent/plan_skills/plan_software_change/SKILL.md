---
name: plan_software_change
description: How to split, order and accept a change to code or configuration in a project — fixing a bug, adding a feature, refactoring, upgrading a dependency. Use it when the objective ends in changed files that must build and pass checks.
---
# Planning a software change

Three stages in order: explore, change, verify.

## Explore

- Start with read-only explorations, side by side, one question each: where the behaviour lives, what calls it, how the project builds and tests, what the current failure looks like.
- For a bug, one exploration reproduces it and reports the exact steps and output. Do not plan a fix for a failure nobody has reproduced; if it cannot be reproduced, that is the finding.
- Add change Todos after the explorations reported, unless the objective already names the files and the change exactly.

## Change

- One writer per workspace at a time: change Todos that touch the same workspace run one after another.
- Cut by what can be checked alone. A good change Todo leaves the project building and its own check passing. Split a change that carries more than one idea; do not split one idea across Todos that only work together.
- Order: what others depend on first (types, schema, interfaces), then their users, then tests and documentation that describe the final state.
- Every change Todo states:
  - scope: the files or the area it may touch, and what it must leave alone;
  - the change, in terms of behaviour — what is different afterwards;
  - acceptance criteria that can be observed: a test that passes, an output that changes, an error that no longer appears;
  - how to check: which builds and tests to run, and what result counts;
  - the exploration findings it rests on.
- Give a change Todo a manual for its kind of work (the project's build and release flow, the platform it targets) when one exists.
- A fix for a reproduced bug includes the reproduction as its check: it fails before the change and passes after.

## Verify

- End with a verification Todo that did not write the change: a read-only check of the acceptance criteria of the whole objective, with the exact steps — what to build, which tests to run, which files to read, what the output must show.
- It reports per criterion: met, not met, or could not be checked, each with the evidence. "The change Todo said it passed" is not evidence.
- A user-visible change is checked through its effect (the output, the page, the reply), not only through unit tests.

## When things go wrong

- A failed check goes back as a fixing Todo that names the failing criterion and its output, then verification runs again.
- Two failed fixes for the same criterion mean the understanding is wrong: add an exploration of the cause before a third attempt.

## Accepted when

Every acceptance criterion of the objective has evidence from the verification Todo, the project builds, the checks named in the plan pass, and the final report lists the changed files and anything left undone.
