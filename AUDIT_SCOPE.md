# Audit scope

Written for an outside, independent security audit. Revision described:
`2a9bef612b598f0cb823cda0a33c44fd8ee013ad` on `main`, 2026-10-26. Everything below is taken from the repository's own files at
that revision; items marked NOT_RUN were not executed while writing this document.

## What this repository is

Planned local AI and a measured FPS tuner for RTX gaming PCs, starting with the RTX 5070 (12 GB).
Five planned parts: a C++/CUDA inference engine with an OpenAI-compatible HTTP server (`engine/`),
speech-to-text (`speech/`), a YouTube creator helper (`creator/`), a Windows FPS tuner (`tuner/`)
and a Windows app (`app/`). AGENTS.md: the component directories currently hold only placeholder
READMEs; there is no build system or CI yet, and nothing has been tested on an RTX 5070. Licence
AGPL-3.0.

## What an auditor can and cannot check here

- Can check: that no tracked file carries tokens, credentials or private data; that README and
  AGENTS.md state status honestly.
- Cannot check here: the engine, HTTP server, speech runtime, tuner or app. Each directory contains
  one line: `# <name> (in progress)`.

## What to review, in priority order

1. All nine tracked files for secrets or private data.
2. `AGENTS.md` "Safety and approvals": the tuner must apply only safe changes, record a receipt for
   each and be able to undo every change. This is the contract later tuner code must meet.
3. `docs/PLAN.md`: assigns `.github/` (Windows CI and release) to the `app/v0` stream. No
   workflow exists yet.

There is no `.github/workflows/` directory.

## Trust boundaries

- Planned, not implemented: network clients of the `engine/` HTTP server; settings changes on the
  user's Windows machine by `tuner/`.
- Untrusted today: pull requests from outside contributors. Trusted: maintainer review; `main` is
  merged by the desk only (AGENTS.md).

## Threat model summary

- Assets: future users' machines and settings; the honesty of performance claims.
- Attackers considered: none can be exercised against code that does not exist.
- Defended: written rules (receipts for every speed claim, reversible tuner changes).
- Not defended / explicitly out of scope: everything executable, because nothing executable exists.

## Build from source at a pin

No build exists yet (AGENTS.md). When it does, engine and speech work needs an NVIDIA RTX card
(target RTX 5070, 12 GB), and tuner and app work needs Windows.

## Run every test

There is no test suite and no CI. NOT_RUN: nothing to run.

## Known limitations and NOT_RUN

- All five components are placeholders.
- `docs/TARGETS.md` and `evidence/` do not exist yet.
- No hardware results of any kind.

## Dependencies and provenance

No lockfile, no vendored code, no dependencies, no GitHub Actions. llama.cpp is named as a
yardstick for comparison, not as a dependency.

## Evidence

Every speed claim is to link to a receipt in `evidence/` naming the card it ran on (AGENTS.md).
`evidence/` does not exist at this revision. Receipts are append-only and are not edited after the
fact.
