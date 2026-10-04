# Claude prompt comparison results

Measured on 2026-10-04 with rootless Podman on `mcpclaude`, Claude Code 2.1.287, `claude-opus-5-5`, and medium effort. All 60 protocol-2 samples passed external correctness, complete native accounting, requested-model verification, and zero-cache checks. Each sample had one attempt and zero MCP calls.

The candidate was built from baseline `0f41ed8` plus the working source snapshot; that baseline has the same product tree as merged main `60224d7`. The feature commits preserve the measured implementation. These measurements predate committing the implementation; binary and runner hashes identify the actual artifacts.

- Candidate binary SHA-256: `4aad2fce8c170d566b112d551e80addda31493c7570bd48171a118f052735621`.
- Comparison runner SHA-256: `19b65c88aa9aaa17b0d7db1bdef8f83c1f21ccb60ceb85e0a33f9dafaee93c2a`.
- Fixture revision: `f02ed2d81584cadcad67c4593b6d252b2387f721`.
- Container image ID: `a1a292372abefa87f5f89b232a0f4e879cb862031ccbd1494becbb7f11edb106`.

## Paired differences

Values are right minus left, in repetition order. Total tokens sum uncached input, output, cache reads, and cache writes. Both cache buckets were zero throughout.

| Workload | Comparison | Five total-token differences | Median | Range |
| --- | --- | --- | ---: | --- |
| one-turn | `container-print` → `scsh-headless` | 0, 0, 0, 0, 0 | 0 | 0…0 |
| one-turn | `container-print` → `container-interactive` | 12377, 12375, 12372, 12378, 12378 | 12377 | 12372…12378 |
| repository | `container-print` → `scsh-headless` | -48, -57, 5, -173, -40 | -48 | -173…5 |
| repository | `scsh-interactive` → `scsh-headless` | -37360, -37178, -37331, -37272, -37128 | -37272 | -37360…-37128 |
| repository | `scsh-headless` → `scsh-compact` | -561, -549, -531, -501, -520 | -531 | -561…-501 |

Exact token parity held in all five one-turn direct-container versus headless pairs, with identical input and output counters separately. Interactive mode added about 12,400 first-request input tokens under the matched controls. This isolates CLI mode; it does not identify the contents of hidden provider context. Multi-turn repository comparisons vary with generated tool requests and conversation history. Compact saved a median 531 total tokens against the standard headless contract in this repository fixture.

The one-turn compact arm adds instructions to a verbatim task and uses 87 more tokens in each repetition. It is not a shortening comparison. Host-print intentionally uses a different host environment. These launcher controls do not test memory savings, and exact parity is not promised for arbitrary stochastic tasks.

Protocol 1 remains preserved under local `tmp/token-lab/results-initial/`, alongside the separate remote campaign artifacts: 47 of 60 samples passed. Its repository Stop hook tried to parse JSON from the final chat reply and rejected 13 replies with extra prose. Protocol 2 requires the model to write the result file and makes that hook leave repository results untouched; it also matches the direct controls' Git branch and identity to the workflow clone. The samples below are a new complete campaign, not replacements selectively mixed into protocol 1.

## Every sample

Repetitions are numbered 1–5. `Wall` includes launcher execution; `Setup` measures fixture preparation before launch. Provider-only duration was unavailable, so no model-only timing is claimed. Input and output are native token counters; both cache counters and MCP-call counts are zero for every row. All rows passed correctness and accounting.

| Workload | Arm | Repeat | Input | Output | Total | Rounds | Tools | Setup (s) | Wall (s) |
| --- | --- | ---: | ---: | ---: | ---: | ---: | ---: | ---: | ---: |
| one-turn | `host-print` | 1 | 15967 | 9 | 15976 | 1 | 0 | 0.135 | 10.320 |
| one-turn | `container-print` | 1 | 15857 | 9 | 15866 | 1 | 0 | 0.161 | 8.601 |
| one-turn | `container-interactive` | 1 | 28234 | 9 | 28243 | 1 | 0 | 0.079 | 8.987 |
| one-turn | `scsh-interactive` | 1 | 28235 | 9 | 28244 | 1 | 0 | 0.093 | 11.910 |
| one-turn | `scsh-headless` | 1 | 15857 | 9 | 15866 | 1 | 0 | 0.088 | 6.456 |
| one-turn | `scsh-compact` | 1 | 15944 | 9 | 15953 | 1 | 0 | 0.081 | 7.112 |
| one-turn | `container-print` | 2 | 15854 | 9 | 15863 | 1 | 0 | 0.074 | 3.732 |
| one-turn | `container-interactive` | 2 | 28229 | 9 | 28238 | 1 | 0 | 0.081 | 8.855 |
| one-turn | `scsh-interactive` | 2 | 28237 | 9 | 28246 | 1 | 0 | 0.089 | 11.327 |
| one-turn | `scsh-headless` | 2 | 15854 | 9 | 15863 | 1 | 0 | 0.078 | 7.645 |
| one-turn | `scsh-compact` | 2 | 15941 | 9 | 15950 | 1 | 0 | 0.109 | 6.919 |
| one-turn | `host-print` | 2 | 15964 | 9 | 15973 | 1 | 0 | 0.079 | 3.855 |
| one-turn | `container-interactive` | 3 | 28217 | 9 | 28226 | 1 | 0 | 0.082 | 8.895 |
| one-turn | `scsh-interactive` | 3 | 28221 | 9 | 28230 | 1 | 0 | 0.145 | 11.866 |
| one-turn | `scsh-headless` | 3 | 15845 | 9 | 15854 | 1 | 0 | 0.075 | 7.164 |
| one-turn | `scsh-compact` | 3 | 15932 | 9 | 15941 | 1 | 0 | 0.072 | 8.446 |
| one-turn | `host-print` | 3 | 15955 | 9 | 15964 | 1 | 0 | 0.075 | 3.462 |
| one-turn | `container-print` | 3 | 15845 | 9 | 15854 | 1 | 0 | 0.085 | 3.818 |
| one-turn | `scsh-interactive` | 4 | 28243 | 9 | 28252 | 1 | 0 | 0.076 | 12.294 |
| one-turn | `scsh-headless` | 4 | 15863 | 9 | 15872 | 1 | 0 | 0.077 | 7.142 |
| one-turn | `scsh-compact` | 4 | 15950 | 9 | 15959 | 1 | 0 | 0.083 | 7.694 |
| one-turn | `host-print` | 4 | 15973 | 9 | 15982 | 1 | 0 | 0.085 | 3.389 |
| one-turn | `container-print` | 4 | 15863 | 9 | 15872 | 1 | 0 | 0.086 | 3.583 |
| one-turn | `container-interactive` | 4 | 28241 | 9 | 28250 | 1 | 0 | 0.077 | 8.812 |
| one-turn | `scsh-headless` | 5 | 15850 | 9 | 15859 | 1 | 0 | 0.084 | 7.690 |
| one-turn | `scsh-compact` | 5 | 15937 | 9 | 15946 | 1 | 0 | 0.079 | 7.548 |
| one-turn | `host-print` | 5 | 15960 | 9 | 15969 | 1 | 0 | 0.089 | 3.372 |
| one-turn | `container-print` | 5 | 15850 | 9 | 15859 | 1 | 0 | 0.080 | 3.974 |
| one-turn | `container-interactive` | 5 | 28228 | 9 | 28237 | 1 | 0 | 0.087 | 8.827 |
| one-turn | `scsh-interactive` | 5 | 28230 | 9 | 28239 | 1 | 0 | 0.081 | 12.798 |
| repository | `host-print` | 1 | 49402 | 397 | 49799 | 3 | 2 | 0.077 | 10.252 |
| repository | `container-print` | 1 | 49046 | 360 | 49406 | 3 | 2 | 0.089 | 8.577 |
| repository | `container-interactive` | 1 | 86180 | 378 | 86558 | 3 | 2 | 0.084 | 12.859 |
| repository | `scsh-interactive` | 1 | 86284 | 434 | 86718 | 3 | 2 | 0.081 | 17.410 |
| repository | `scsh-headless` | 1 | 49032 | 326 | 49358 | 3 | 2 | 0.079 | 15.971 |
| repository | `scsh-compact` | 1 | 48472 | 325 | 48797 | 3 | 2 | 0.077 | 12.279 |
| repository | `container-print` | 2 | 49038 | 363 | 49401 | 3 | 2 | 0.087 | 9.304 |
| repository | `container-interactive` | 2 | 86249 | 411 | 86660 | 3 | 2 | 0.087 | 12.772 |
| repository | `scsh-interactive` | 2 | 86169 | 353 | 86522 | 3 | 2 | 0.142 | 17.117 |
| repository | `scsh-headless` | 2 | 49029 | 315 | 49344 | 3 | 2 | 0.077 | 14.284 |
| repository | `scsh-compact` | 2 | 48465 | 330 | 48795 | 3 | 2 | 0.079 | 13.009 |
| repository | `host-print` | 2 | 49317 | 340 | 49657 | 3 | 2 | 0.081 | 8.420 |
| repository | `container-interactive` | 3 | 86177 | 346 | 86523 | 3 | 2 | 0.083 | 15.749 |
| repository | `scsh-interactive` | 3 | 86267 | 429 | 86696 | 3 | 2 | 0.074 | 18.335 |
| repository | `scsh-headless` | 3 | 49024 | 341 | 49365 | 3 | 2 | 0.075 | 13.500 |
| repository | `scsh-compact` | 3 | 48478 | 356 | 48834 | 3 | 2 | 0.076 | 14.607 |
| repository | `host-print` | 3 | 49319 | 359 | 49678 | 3 | 2 | 0.083 | 8.568 |
| repository | `container-print` | 3 | 49014 | 346 | 49360 | 3 | 2 | 0.076 | 8.450 |
| repository | `scsh-interactive` | 4 | 86250 | 374 | 86624 | 3 | 2 | 0.081 | 19.270 |
| repository | `scsh-headless` | 4 | 49026 | 326 | 49352 | 3 | 2 | 0.081 | 17.738 |
| repository | `scsh-compact` | 4 | 48492 | 359 | 48851 | 3 | 2 | 0.081 | 14.057 |
| repository | `host-print` | 4 | 49396 | 395 | 49791 | 3 | 2 | 0.077 | 8.663 |
| repository | `container-print` | 4 | 49120 | 405 | 49525 | 3 | 2 | 0.077 | 8.679 |
| repository | `container-interactive` | 4 | 86177 | 394 | 86571 | 3 | 2 | 0.077 | 12.919 |
| repository | `scsh-headless` | 5 | 49047 | 325 | 49372 | 3 | 2 | 0.092 | 15.953 |
| repository | `scsh-compact` | 5 | 48494 | 358 | 48852 | 3 | 2 | 0.078 | 15.134 |
| repository | `host-print` | 5 | 49415 | 406 | 49821 | 3 | 2 | 0.081 | 9.335 |
| repository | `container-print` | 5 | 49061 | 351 | 49412 | 3 | 2 | 0.079 | 8.565 |
| repository | `container-interactive` | 5 | 86207 | 372 | 86579 | 3 | 2 | 0.086 | 15.477 |
| repository | `scsh-interactive` | 5 | 86179 | 321 | 86500 | 3 | 2 | 0.091 | 18.199 |

The remote full reports, prompt manifests, native ledgers, and logs are retained at `~/scsh-token-lab/comparison-20261004-matched/`; local copies of the aggregate reports and pinned runner are under `tmp/token-lab/results-matched/`. Reproduction and interpretation requirements are in [CLAUDE-PROMPTS.md](CLAUDE-PROMPTS.md).
