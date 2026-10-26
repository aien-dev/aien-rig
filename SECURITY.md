# Security policy

## Reporting a vulnerability

Report privately. Do not open a public issue, pull request or discussion about it.

1. GitHub: open this repository's Security tab and choose "Report a vulnerability". This
   creates a private advisory that only the maintainers can see. If that option is not shown,
   private vulnerability reporting has not been enabled for this repository yet; use email.
2. Email: drake@aienos.com with the subject `[aien-rig] security report`.

Include the commit or release you tested, the component (path), steps to reproduce, and what an
attacker gains. If the report involves a secret, say where it was exposed and do not send the
secret itself. Encrypted mail is not required; do not post proof-of-concept payloads publicly.

## Response targets

This project has one maintainer; the targets below are best effort and stated honestly.

- Acknowledgement: within 3 working days.
- Triage and severity: within 7 days of acknowledgement.
- Fix or mitigation for confirmed issues: aim for 30 days for high and critical, 90 days
  otherwise, with a status update at least every 14 days until closed.
- Disclosure: coordinated. An advisory is published with credit (unless you ask otherwise)
  once the fix is on the default branch. Default embargo 90 days, negotiable.

## Scope

At this revision the component directories (`engine/`, `speech/`, `creator/`, `tuner/`, `app/`)
hold only placeholder READMEs; there is no build system or CI yet (AGENTS.md).

In scope:

- Tokens, credentials or private data committed to any file.
- Once code lands: the planned OpenAI-compatible HTTP server in `engine/` (network input), and the
  Windows FPS tuner in `tuner/`, which changes settings on users' machines and must apply only safe
  changes, keep a receipt for each and be able to undo every change (AGENTS.md).

Out of scope:

- Third-party engines used as yardsticks (for example llama.cpp), NVIDIA drivers, Windows and the
  games the tuner measures.

## Supported versions

Only the default branch `main`; there is no release. Reports against older commits are welcome but fixes land on the default branch only.

## Status

Pre-alpha, under active build. Not tested on an RTX 5070 yet (README). No certification, no bug
bounty. See AUDIT_SCOPE.md for what to review and how to build and test it.
