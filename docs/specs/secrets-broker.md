# Secrets Broker integration

`browser-cli` provides credential-free browser filling through the Secrets Broker. The contract is implemented by `src/broker.rs` and command dispatch in `src/main.rs`; the current-state design is documented in [the system wiki](../wiki/systems/secrets-broker.md).

## What it must do

### Transport and secret boundary

- [x] Use published `secrets-broker-client` v0.3.0 pinned by exact git revision `9439cfa62098a6213daa466f69449bc8a99805fc` for socket and protocol transport.
- [x] Send only the requested broker scope and CDP target ID for browser fill; never send credential field values.
- [x] Return filled-count metadata rather than credential values.
- [ ] Rely on the broker to verify the exact registered browser origin before filling.
- [ ] Never submit the form; MFA and CAPTCHA remain user-driven.

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
- [ ] Keep `broker-unlock` available as an optional explicit approval command.

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

## Known gaps (current cycle)

- [ ] CI proof for the current integration.
- [ ] Push proof for the current integration.
- [ ] Deployment proof for the current integration.
- [ ] Live broker/browser proof for the current integration.

## Out of scope

- Credential storage, broker daemon authorization, authd approval UI, and exact-origin verification inside the broker.
- Automatic form submission, MFA completion, CAPTCHA handling, payments, transfers, or other browser mutations.
