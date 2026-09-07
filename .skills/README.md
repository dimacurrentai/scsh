# Agent skills used by `scsh`

This directory contains the skills that `scsh` develops, exercises, or embeds. Each skill is a folder with a `SKILL.md` (YAML frontmatter plus Markdown body) and optional `scripts/`, `references/`, and `assets/`.

The source of truth depends on the skill family:

- The example, smoke, and self-test skills are authored here because they document or exercise `scsh` itself.
- The five reviewer bodies are synchronized mirrors of [`dkorolev/code-review-skills`](https://github.com/dkorolev/code-review-skills), their canonical authoring repository. `src/config.rs` pins their content hashes to a named canonical revision, and tests reject an unpinned local edit. Reviewer changes land in `code-review-skills` first and are then mirrored here.
- Delivery and publishing workflows such as `big-beautiful-build`, `code-gorgeous-review`, and `gh-gorgeous-review` are authored in [`dkorolev/beautiful-skills`](https://github.com/dkorolev/beautiful-skills). They are installed from that repository and are deliberately not bundled into the `scsh` binary.

Edit a skill here only when this repository is its source of truth or when deliberately mirroring a canonical reviewer revision. Do not edit through the tool-specific paths below; they are symlinks.

## Tool discovery paths

| Tool | Project path | Notes |
| --- | --- | --- |
| Skills | `.skills/<name>/` | Repository-local skill bodies |
| Cursor | `.cursor/skills/` -> `.skills` | Also `~/.cursor/skills/` for personal skills |
| Claude Code | `.claude/skills/` -> `.skills` | Also `~/.claude/skills/` |
| Codex | `.agents/skills/`, `.codex/skills/` -> `.skills` | Also `~/.agents/skills/`, `~/.codex/skills/` |
| OpenCode | `.opencode/skills/` -> `.skills` | Also reads `.claude/skills` and `.agents/skills` |

All repository discovery symlinks point at this directory, so a legitimate edit is visible to every host.

## Skills in this repo

| Skill | Ownership | Purpose |
| --- | --- | --- |
| [conventions-reviewer](conventions-reviewer/SKILL.md) | Mirrored from `code-review-skills` | Enforce the repository's own conventions |
| [justification-reviewer](justification-reviewer/SKILL.md) | Mirrored from `code-review-skills` | Challenge scope, necessity, and complexity |
| [reviewability-reviewer](reviewability-reviewer/SKILL.md) | Mirrored from `code-review-skills` | Review commit and PR presentation for humans |
| [sanity-reviewer](sanity-reviewer/SKILL.md) | Mirrored from `code-review-skills` | Catch obvious security, performance, and resource-leak problems |
| [testing-reviewer](testing-reviewer/SKILL.md) | Mirrored from `code-review-skills` | Check that changed behavior is verifiable and test tooling cleans up |
| [scsh-harness-demo-and-selftest](scsh-harness-demo-and-selftest/SKILL.md) | Authored here and bundled | Follow `DEMO.md` to bootstrap and verify a tiny `scsh` project |
| [harness-smoke](harness-smoke/SKILL.md) | Authored here | Minimal JSON smoke test for the configured subscription-first harness routes |
| [add](add/SKILL.md) | Authored here | Add `A` and `B`; scaffolded by `scsh init-demo-project` |
| [subtract](subtract/SKILL.md) | Authored here | Subtract `D` from `C`; scaffolded by `scsh init-demo-project` |
| [multiply](multiply/SKILL.md) | Authored here | Multiply required `X` and `Y`; scaffolded by `scsh init-demo-project` |
| [demo-pr](demo-pr/SKILL.md) | Authored here | Produce a minimal fake PR for commit-integration demonstrations |

The root `.scsh.yml` is the manifest embedded by no-URL `scsh installskills`: it defines the bundled five-reviewer `code-review` profile and the repository's harness-smoke profiles. The demo skills scaffolded by `scsh init-demo-project` use the separate embedded manifest at `src/demo.scsh.yml`.

## Changing a skill

1. Identify its source of truth above. For a reviewer, change `dkorolev/code-review-skills` first, then mirror the exact body here and update the pinned hashes and canonical revision in `src/config.rs`.
2. Keep the directory name equal to the skill's frontmatter `name`.
3. Author only under `.skills/<name>/`, never through a symlinked host path.
4. Update the appropriate manifest only when the skill is actually shipped by that manifest, then run the repository test suite.
