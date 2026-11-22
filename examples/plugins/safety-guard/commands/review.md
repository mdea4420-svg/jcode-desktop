---
description: Review uncommitted changes for bugs and risky edits
argument-hint: [path or focus]
---
Review the current uncommitted changes (`git diff` and `git diff --staged`).

Report, in order of severity:
1. Bugs and behavior regressions, with file and line.
2. Risky operations (data loss, security, breaking API changes).
3. Missing tests for the changed behavior.

Do not modify files. Focus: $ARGUMENTS
