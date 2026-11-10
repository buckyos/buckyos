---
name: hello
description: Sample work skill for the skill PIKG install test — prints a greeting with the `hello` command. Use it to check that an installed skill's manual and command reach a work session.
requires: The `hello` command of this skill on PATH (tools/hello/bin).
scope: work
baseline: standard
---
# hello

Run `hello [name]` to print a greeting. Success: it prints `hello, <name>` and exits 0.
