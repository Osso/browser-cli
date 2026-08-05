# Secrets Broker integration

`browser-cli` provides credential-free browser filling through the Secrets Broker. The contract is implemented by `src/broker.rs` and command dispatch in `src/main.rs`; the current-state design is documented in [the system wiki](../wiki/systems/secrets-broker.md).

## What it must do

### Transport and secret boundary

- [x] Use published `secrets-broker-client` v0.3.0 pinned by exact git revision `9439cfa62098a6213daa466f69449bc8a99805fc` for socket and protocol transport.
- [x] Send only the requested broker scope and CDP target ID for browser fill; never send credential field values.
- [x] Return filled-count metadata rather than credential values.
- [x] Rely on the broker to verify the exact registered browser origin before filling.
- [x] Never submit the form; MFA and CAPTCHA remain user-driven.

### Scope resolution

- [x] Resolve `--current-origin` from the active tab's exact lowercase HTTPS hostname.
- [x] Require an explicit `--scope` to exactly match the hostname mapping when both options are supplied.
- [x] Reject unmapped hostnames, insecure URLs, suffix matches, malformed mappings, missing files, symlinks, non-regular files, foreign ownership, and modes other than `0600`.
- [x] Prefer `$XDG_CONFIG_HOME/browser-cli/credential-scopes.json`, otherwise use `~/.config/browser-cli/credential-scopes.json`.

### Fill approval lifecycle

- [x] If the broker authorizes the request, perform one fill request without unlocking.
- [x] If the first fill returns `Locked`, request approval once for the resolved exact scope and retry the fill exactly once.
- [x] On approval denial, fail closed before retrying the fill and do not expose broker error contents.
- [x] On approval unavailability, fail closed before retrying the fill and do not expose broker error contents.
- [x] If the retry returns `Locked`, fail closed without another approval or fill attempt.
- [x] Keep `broker-unlock` available as an optional explicit approval command.

## How it works

- [Secrets Broker system design](../wiki/systems/secrets-broker.md)

## Implementation inventory

- `src/broker.rs` — shared-client broker operations, exact-host scope mapping, and approval/retry state machine.
- `src/main.rs` — `broker-unlock`/`broker-fill` command dispatch, active-target discovery, and broker arguments.
- `src/cdp.rs` — active CDP target discovery and target identifiers.
- `Cargo.toml` — exact git revision for `secrets-broker-client`.
- `Cargo.lock` — locked dependency version and exact git revision.

## Tests asserting this spec

- `src/broker.rs` — unit tests for authorized fill, exact-scope approval and one retry, denial, unavailable approval, second `Locked`, exact HTTPS mapping, mapping validation, and XDG/home path selection.
- `tests/cli.rs` — external CLI help test proving `broker-unlock` remains an optional explicit command.

## Verification evidence

- Feature/source commit `34b850125ba037d411f2e4abde330cda3706e6d2` was pushed to remote `master`; later evidence/test-only commits do not change executable source.
- Local feature `./run-tests.sh`: `cargo fmt`, `cargo clippy -- -D warnings`, and 62 tests passed.
- Test-only commit `f9a177bc1494cd022d2def573c186daf4c60f56e` adds external proof that optional `broker-unlock` remains available; the current runner passes 63 tests.
- GitHub CI run `31033510969`: passed.
- Deployment: `./deploy.sh`; installed/release SHA-256 `a2bad6e754d3703e15f7a510f6309b1ae6e5b2e05d84d55133d421994361b0d3`; no `browser-cli` process remained.
- Fresh terminal-principal smoke: PID `1478667`, start time `151617732`, TTY `34816`; exact `browser:citi` fill returned `Locked`, exact leaf approval/unlock was requested once and recorded, one retry succeeded with only `Filled 2 credential fields`; no sign-in or submit occurred; the browser was navigated to `example.com` afterward. Broker PID `3373670`, `NRestarts 0`.

## Out of scope

- Credential storage, broker daemon authorization, authd approval UI, and exact-origin verification inside the broker.
- Automatic form submission, MFA completion, CAPTCHA handling, payments, transfers, or other browser mutations.
