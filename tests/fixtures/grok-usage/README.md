# Grok usage fixture

`updates.jsonl` comes from a fresh Grok Build 1.0.34 interactive session on 2026-09-20 that wrote one tiny file. Prompts, generated text, file paths, tool payloads, and identifiers were removed or replaced; lifecycle events and native token counters are unchanged. The separate native `usage.json` agreed: 40,430 input tokens (including 19,456 cached reads), 191 output tokens, zero cache-creation tokens, and two model calls.

The parser uses completed prompt bills rather than context-window telemetry. Grok's [notification schema](https://github.com/xai-org/grok-build/blob/main/crates/codegen/xai-grok-shell/src/extensions/notification.rs) defines the input/cache split and `usageIsIncomplete`; its [usage ledger](https://github.com/xai-org/grok-build/blob/main/crates/codegen/xai-chat-state/src/usage.rs) folds child spend into the parent. Child transcripts and `modelUsage` breakdowns must not be summed again.

Run `cargo test usage::` for offline parsing, schema round-trip, duplicate-bill, malformed/incomplete-counter, resumed-turn, overflow, and host completion checks. The completion test constructs a primary session and a child, verifies that the child cannot release shutdown, then verifies that the final primary bill is saved before shutdown and counted once.
