# Runtime evaluation

`CdpConnection::eval` in `src/cdp.rs` evaluates JavaScript through CDP. See [CDP communication](../../CLAUDE.md#cdp-communication) for the connection API.

## What it must do

- [x] Reject `Runtime.evaluate` responses containing `exceptionDetails`, even without a top-level protocol error, before returning any value, description, or null.
- [x] Report the exception object's description when available, otherwise the exception text.
- [x] Preserve successful JSON values, description-only results, and undefined results represented as null. General `eval` does not await Promises and preserves Promise descriptions.
- [x] Selector `wait` alone requests `awaitPromise: true`; complete only after resolution, and propagate rejection through the same exception handling as general `eval`. Do not report an element found before completion.

## How it works

- [CDP communication](../../CLAUDE.md#cdp-communication)

## Implementation inventory

- `src/cdp.rs` — WebSocket transport, command-specific awaited evaluation, and shared runtime evaluation result handling.
- `src/commands.rs::wait_for_selector` — selector-wait Promise evaluation.

## Tests asserting this spec

- `src/cdp.rs`, `eval_tests` — real loopback WebSocket CDP fixture; exception descriptions, exception text taking precedence over values, exceptions without values/descriptions, and successful result compatibility without Promise awaiting.
- `src/commands.rs`, `tests::selector_wait_*` — actual selector-wait path through a loopback WebSocket fixture; remain pending until delayed resolution, rejection (text or description), or protocol error. The fixture returns an immediate Promise description unless awaiting is requested. Tests cover the CDP boundary, not browser JavaScript execution or DOM polling.

## Verification evidence

`cargo test --locked --bin browser-cli cdp::eval_tests -- --nocapture` at `2d10d367a8ec47e431ffc65f3d2429989fd2a6b0`: four tests passed. Before the fix, the three exception regressions failed by returning description, value, and null; successful result compatibility passed.

## Out of scope

- General Promise awaiting, URL waiting, and load-state waiting.
- Browser/account integration, JavaScript changes, and network behavior changes.
