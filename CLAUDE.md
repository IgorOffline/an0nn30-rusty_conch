# Conch — Claude Instructions

## Branching Rules (STRICT)

- **Claude must never commit or push directly to `main`.**
- The repo owner (`an0nn30`) may push directly to `main` when appropriate.
- Every feature, fix, or change — no matter how small — must go on its own branch.
- Branch naming convention:
  - `feat/short-description` — new features
  - `fix/short-description` — bug fixes
  - `chore/short-description` — docs, config, tooling, cleanup
  - `perf/short-description` — performance improvements
- Before starting any work, check the current branch. If on `main`, create a new branch first.
- Push the branch to origin and open a PR for the user to review and merge.
- Never use `--force` push.

## Commit Rules

- Never add Co-Authored-By lines to commits.
- Write concise, descriptive commit messages in the imperative mood.

## General

- This is a public, open-source repo. Be thoughtful about what goes into commits.
- PRs should be small and focused — one concern per PR.
