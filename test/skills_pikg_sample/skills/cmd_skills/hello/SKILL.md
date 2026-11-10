---
name: hello
description: Sample work skill for the skill PIKG install test — how to print a greeting. Use it to check that the manual of an installed skill reaches a work session.
requires: Nothing beyond the shell of the runtime.
scope: work
baseline: standard
---
# hello

Print a greeting with the shell's own `echo`; when no name is given, greet `world`:

```sh
echo "hello, <name>"
```

Success: the output is the line `hello, <name>` and the exit code is 0.
