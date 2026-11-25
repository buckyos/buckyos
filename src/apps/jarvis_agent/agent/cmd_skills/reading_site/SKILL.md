---
name: reading_site
description: Build and extend the owner's personal reading site — set it up, add a website as a source (a program that fetches its articles, checked against the real pages and shown translated on the owner's phone), carry out an upgrade of a source or of the layout, take the site over from another Agent — and hand it to the goal that keeps it up to date. For work that adds a website to the reading site, builds the site, or does an upgrade its goal asked for.
requires: a work session on the Agent's private runtime in the site's workspace, started by the owner's conversation (it sets up or revises the maintaining goal); read_write Agent permission (a read-only Agent cannot write the owner's bucket); the ext-core extension package (agent-browser, chromium) to look at pages and for sources that need the browser
commands: site, agent-browser
scope: work
baseline: extended
---
#### What you work on

The owner's reading site: the websites they read, each fetched by a program (a source package), translated into the site's language and shown at `https://reflow.<zone>/` and in Agent Browser's home, whether or not anything is running. One site per owner; one goal of one Agent keeps it (`maintainer` in site.json). Keep three things apart: the workspace holds the source of what you write; what runs is a copy installed from it (the package in the bucket, the goal's jobs, a template version); the content is the owner's, in their bucket. `site --help` is the reference for every subcommand and the package format.

```text
SITE.md              per source: what the owner asked, what you told them, what does not work and why
sources/<source>/    a package: source.json, extract.ts, NOTES.md, fixtures/ (and .mjs scripts for the browser)
reading/README.md    program directory of the job site-reading: one line of text, the job runs a command
check.ts             the goal's check script
template/            only after a layout change: the edited copy of the default template
upgrades/            upgrade notes the goal writes
```

Paths here are relative to the workspace root: start commands there (`cd "$OPENDAN_WORKSPACE_DIR" && …`) and keep trial output in the session directory. Start with `site status`. No site yet: `site init --title "<title>" --language <the owner's language, BCP 47> --topics <a,b,c>` (up to 16 topics in that language; articles are tagged with them). `maintainer` names another Agent: read "Taking over" in the hand-over reference first.

#### A source, step by step

1. Decide what the owner reads there from the request (`URL`, `Title`, `Note`): the whole site or one section. `site match <url>` says whether a source has it already. Copy the nearest sample from `/opt/buckyos/bin/opendan/buckyos-agent/site-sources/` (on a node: `$BUCKYOS_ROOT/libexec/buckyos-agent/site-sources/`) to `sources/<source>/`, write its `source.json`, and `site source put sources/<source>`: the owner sees it as being set up.
2. Understand the site as a signed-out reader gets it: where the list of new articles is, where an article's whole text is (one page or several), its pictures, its comments. Look with plain requests (`curl -sL <url>`) and compare with what the browser shows (`agent-browser open <url>`, `agent-browser snapshot`). Then take the cheapest way that gives whole articles, in this order, stopping at the first that works: a feed or interface the site offers (RSS/Atom named in the page's `<link rel="alternate">`, JSON the page embeds or calls) → plain requests for the HTML → the Agent's browser (`"needs": "browser"`: a browser start on every sync, the last resort) → the owner's sign-in (`"needs": "login"`: only when the articles exist for signed-in readers alone).
3. Write `extract.ts`, `NOTES.md` and fixtures: read `$OPENDAN_AGENT_ROOT/cmd_skills/reading_site/references/extract.md`.
4. Try it without touching the bucket, then read the verdict:

```sh
site try sources/<source> --no-translate --record --out "$OPENDAN_SESSION_DIR/try-<source>"
site try sources/<source> --fixtures --no-translate
site try sources/<source> --limit 1 --out "$OPENDAN_SESSION_DIR/try-<source>"
```

   - Exit 0: the run passed the check. Each `published` line gives characters, blocks, images (`missing`: not fetched) and comments; each `rejected` line gives a reason: no title, a body shorter than the source's usual (you took the teaser or the frame), U+FFFD (wrong encoding), the same body three times (you took the page, not the article). More than half rejected, or a list that came back empty, fails the whole run (`site try => program`).
   - Open two articles in `<out>/_data/a/` beside the original pages: first and last paragraph, order, pictures; no navigation, related lists, cookie or subscribe text.
   - `--record` stored the pages as fixtures; the `--fixtures` run must give the same articles offline. That is what a later repair is tested against.
   - The third run translates one article with the site's language and topics: `model token(s)` times the articles a day must fit `limits.translate_tokens_per_day`. `waiting (translate_config: …)` means this Zone has no model for translation: nothing can be published until the administrator sets one up — report that as blocked.
5. See it as the owner will, at phone width: read `$OPENDAN_AGENT_ROOT/cmd_skills/reading_site/references/template.md` ("Look at a page").
6. Install it and hand it over (below).

#### Install and hand over

The general method is the manual `goal_handover` (`$OPENDAN_AGENT_ROOT/cmd_skills/goal_handover/SKILL.md`): follow it. What is particular to a reading site — the goal's definition, its check script, its skill `reading_site_ops`, the jobs, `maintainer`, the translation limit — is in `$OPENDAN_AGENT_ROOT/cmd_skills/reading_site/references/handover.md`. In short: the site has one maintaining goal, created by the work that builds the site (before its first job) and revised by every later one; each source is one job of it:

```sh
site source put sources/<source> --state active
agent-session goal job put src-<source> --goal <goal_id> --dir sources/<source> --command "site sync <source> --dir {dir}" --every <every_secs> --timeout 600 --title "<source title>"
agent-session goal job run src-<source> --goal <goal_id> --wait
site status
```

A job installs the whole directory: nothing but the package may be in `sources/<source>/` (no trial output, never run `site sync` inside it). A new version syncs at once; the first sync publishes what it can translate in 8 minutes and the next ones go on.

#### What you may not do, and what to do instead

- A site that refuses programs or sells its articles is not got around: no faked browser identity, no paywall or sign-in wall bypass, no borrowed accounts. Take what it gives every reader: when only headlines and leads are open, make it a list-only source (`check.min_chars` lowered, why in `NOTES.md`) whose "read the original" goes to the site — and say so in `SITE.md` and in your report, so the owner is told before they rely on it.
- A source behind the owner's sign-in reads with the cookies in the Agent's browser, never with a password or an exported cookie. The owner signs in themselves: `agent-browser open <login page>`, `agent-browser handover --reason password --note "<site>: sign in"`, then `ask_user "<answer when you have signed in and handed the browser back>"`. Tell them it syncs every 6 hours and stops when the sign-in ends.
- Part of the task does not work: hand over what works. Each source that does not: `site source put sources/<source> --state failed --note "<why, in the owner's language>"`, no job, the reason and what you tried in `SITE.md` and in the report. The owner can retry it later.
- `hide_topics`, pausing and removing sources are the owner's: change them only when the task says so.

#### Upgrades and the layout

An upgrade (`Upgrade:`, `File:`, `Goal:` in the task) starts from the goal's note: read the file; the source it speaks of is in the workspace. Repair or rebuild there, record new fixtures, try, install as above (`goal job put` keeps a stopped job stopped: `agent-session goal job enable src-<source> --goal <goal_id>`, since the goal stopped it when it asked), `site upgrade close <id>`, and revise the goal when what it keeps changed. A new layout is an upgrade too: template code changes only on the owner's word (`$OPENDAN_AGENT_ROOT/cmd_skills/reading_site/references/template.md`); the order of columns, items per column and folded columns are parameters (`site config set template.params.<name> <json>`).

#### Report

`result`: `goal_id`, and `sources` with `id`, `state`, `every_secs`, `needs` and `note` for each one you touched. The text: what the owner can read now and where, what each source gives (full text, list only, needs their sign-in), what failed and why, what the goal does from now on and when they hear from it.
