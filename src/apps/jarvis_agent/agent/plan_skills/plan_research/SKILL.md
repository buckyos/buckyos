---
name: plan_research
description: How to split a read-only investigation into parallel explorations, what each must return, and how to merge and cite the results. Use it when the objective is to find out, compare or explain something — in code, files, logs, documents or on the web — and ends in a report, not in changed files.
---
# Planning research

Read-only explorations side by side, then one merge.

## Frame the question

- Write the question as what must be decided or known at the end, and what a complete answer contains. "Look into X" is not a question; "which of A and B handles case C, and what would switching cost" is.
- List what is already known and where it came from.

## Cut the explorations

- Cut so that no exploration needs another's result. Good cuts: by source (one repository, one log, one set of documents), by sub-question, by candidate when comparing options, by hypothesis when looking for a cause.
- Each exploration gets one question it can answer alone, its scope (where to look and where not), and a stopping rule: what counts as answered, and when to stop and report "not found here".
- Three to six is usual. One that would take most of the work is cut again; two that read the same material are merged.
- When comparing options, every candidate's exploration gets the same list of criteria, so the results line up.
- When looking for a cause, each hypothesis gets an exploration that says what observation would confirm it and what would rule it out.
- Give an exploration a manual for the system it examines when one exists.
- A question that depends on an earlier answer is a second round: add it when the first round has reported, not before.

## What each exploration returns

- The answer to its question in a few lines, first.
- Findings, each with its source: file and line, command and output, document and section, link and date read.
- What is verified (seen, run, reproduced) kept apart from what is inferred or only claimed by a source.
- What it looked for and did not find, and where it looked. Absence is a result.
- Leads outside its scope, named but not followed.

## Merge

- Build the answer from the findings, not from the explorations' own summaries.
- Where two explorations disagree, do not average: add a short exploration that checks the disputed point against the source, and say in the result which was right.
- A claim with a single source that the conclusion depends on gets a second look from an independent source, or is marked as resting on one source.

## Cite

- Every statement of fact in the final report points to where it was found, precisely enough for a reader to open it: the path and line, the command, the document and section, the link.
- Mark each conclusion as observed, inferred from observations (say which), or reported by a source.

## Accepted when

The final report answers the question as framed, every claim it rests on has a citation, disagreements are resolved or stated, and the open gaps are listed. When the answer drives a costly decision, add an exploration that re-reads the cited sources of the key claims.
