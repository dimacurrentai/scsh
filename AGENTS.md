# Agent Instructions

Read [`CONTRIBUTING.md`](CONTRIBUTING.md) first — it maps the repository, the `tmp/` rule, the gates, and the deliberate waivers.

The house rules are codified in [`dkorolev/principles`](https://github.com/dkorolev/principles) and apply here in full: [`ENG-PRINCIPLES.md`](https://github.com/dkorolev/principles/blob/main/ENG-PRINCIPLES.md) governs engineering doctrine (typing, CLI ergonomics, testing, error handling, git, config, publishing), and [`WEB-UI-PRINCIPLES.md`](https://github.com/dkorolev/principles/blob/main/WEB-UI-PRINCIPLES.md) governs the session browser. The rare, deliberate waivers are listed at the end of `CONTRIBUTING.md` — everything else is binding.

## Do not hide command output

Never pipe long-running or important commands through `| tail`, `| head`, or similar truncators (e.g. `cargo test 2>&1 | tail -5`). Waiting with no live progress is worse than a long log. Run the command as-is so stdout/stderr stream; if you need a durable log, use `tee` (and still keep the live stream). Details in [`CONTRIBUTING.md`](CONTRIBUTING.md).

## Pull request bodies

Pull request descriptions contain summary and implementation-detail prose only. Testing stays in the branch and CI, never in the pull request body.

## Exact Beecast and Packdiff versions

Use exact versions for the first-party Beecast and Packdiff crates. Pin `beecast-player`,
`beecast-page`, and `packdiff` with Cargo's exact `=X.Y.Z` syntax. Never use a range or
“latest”. Upgrade every manifest, lockfile, test, and documentation reference together so
the repository names one exact version.
