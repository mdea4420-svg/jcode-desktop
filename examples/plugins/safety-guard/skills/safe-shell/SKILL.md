---
name: safe-shell
description: Rules for running shell commands safely. Use before deleting files, rewriting git history, or running commands with side effects.
---

# Safe shell

- Prefer read-only commands to explore. Delete only files you created.
- Never run `rm -rf` on a path you did not just create, `git push --force`,
  `git reset --hard` over uncommitted work, or `git clean -fdx`.
- When a destructive step is truly needed, stop and ask the user first.
