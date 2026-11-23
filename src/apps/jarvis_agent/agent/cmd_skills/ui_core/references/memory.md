# Writing a memory observation

`agent-session ctl perceive "<text>" [--tag <topic>]... [--object <path|obj:// URL>]... [--cite <hint id>]... [--suggest preference|correction|experience|fact] [--explicit] [--occurred-at <RFC 3339>]`

The memory maintenance goal consolidates observations later. Work and conversations leave no memory material unless someone perceives it. Keep requirements in the task and permissions in the owner's approval.

- Preference, requirement or boundary: preserve the owner's wording, project, audience, duration and exceptions. Label a suggestion or tendency accordingly. Use `--explicit` only when the owner asked to remember.
- Judgment: say who judged which deliverable, person, supplier or tool, on what occasion and why. Attribute third-party claims. Identify your own inference.
- Correction: quote the correction and its scope. For an existing hint use `--cite <hint id> --suggest correction`.
- Surprise, mistake, decision or successful method: state the expectation, conditions, action, result and what remains unknown. Distinguish attempted, reported, verified and owner-approved. Perceive a lesson from child work when relaying the report.
- Tool observation or project convention: record the version, environment, object and call or source. Date earlier events with `--occurred-at`.
- Fact, role, alias, relation or private term: keep names and pointers exact, including ports, paths and purpose. Prefer a link to an authoritative contact or configuration object. Only merge identities the owner connects.

Tag recurring concepts in English lowercase with hyphens (`weekly-report`), and names, projects and IDs in their original spelling (`陈宁`, `proj-a`, `ClawChat`). Use `--object` for the project, tool or file identity.

| What was said | What to capture |
| --- | --- |
| “What's the weather in Shanghai?” | One-off query. Skip it. |
| “I'm in Shanghai until Friday.” / “I live in Shanghai.” | Temporary stay with end date / long-term residence. |
| “Default to Shanghai for weather.” | Weather preference. |
| “More detail on this report.” / “From now on, project A weekly reports should fit one page.” | Skip one edit / record scoped requirement. |
| “Create the directory first.” | Owner's suggested method. Record any later test with tool version and result. |
| “Supplier X was late on project A. I'm not sure about them.” | Owner's doubt about timing, the project and reason. |
| Tool v1 failed without `--out` and passed with it. | Record observed results and environment. If directory permissions may also matter, leave the cause open. |

Skip budgets, greetings, capabilities, repeated error lines, state retrievable from an object, secrets and incidental task details.
