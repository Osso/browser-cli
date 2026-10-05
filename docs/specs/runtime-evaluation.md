# Runtime evaluation

`CdpConnection::eval` in `src/cdp.rs` evaluates JavaScript through CDP. See [CDP communication](../../CLAUDE.md#cdp-communication) for the connection API.

## What it must do

- [x] Reject `Runtime.evaluate` responses containing `exceptionDetails`, even without a top-level protocol error, before returning any value, description, or null.
- [x] Report the exception object's description when available, otherwise the exception text.
- [x] Preserve successful JSON values, description-only results, and undefined results represented as null.

## How it works

- [CDP communication](../../CLAUDE.md#cdp-communication)

## Implementation inventory

- `src/cdp.rs` — WebSocket transport and runtime evaluation result handling.

## Tests asserting this spec

- `src/cdp.rs`, `eval_tests` — real loopback WebSocket CDP fixture; exception descriptions, exception text taking precedence over values, exceptions without values/descriptions, and successful result compatibility.

## Verification evidence

`cargo test --locked --bin browser-cli cdp::eval_tests -- --nocapture` at `2d10d367a8ec47e431ffc65f3d2429989fd2a6b0`: four tests passed. Before the fix, the three exception regressions failed by returning description, value, and null; successful result compatibility passed.

## Out of scope

- Promise awaiting: `src/commands.rs::wait_for_selector` passes `WAIT_SELECTOR_SCRIPT_TEMPLATE` (a Promise) to `eval`, which does not set `awaitPromise`. This contract covers reported `exceptionDetails`, not pending Promise resolution or rejection.
- Browser/account integration, JavaScript changes, and network behavior changes.
