# Secrets Broker integration

`browser-cli` is a credential-free Secrets Broker consumer. It uses published `secrets-broker-client` v0.3.0 pinned to exact git revision `9439cfa62098a6213daa466f69449bc8a99805fc` for Unix-socket framing and protocol transport. Browser-cli retains browser-specific policy: exact-host scope mapping and CDP target discovery.

## Commands

```text
browser-cli broker-unlock --scope SCOPE
browser-cli broker-fill --scope SCOPE
browser-cli broker-unlock --current-origin
browser-cli broker-fill --current-origin
```

`broker-unlock` is optional. It requests approval for the selected scope and reports the broker's unlocked scope list. Normal fill does not require a separate unlock command.

The default socket is `/run/secrets-broker/broker.sock`; `--socket PATH` overrides it. `--scope-config PATH` overrides the automatic mapping path and is valid only with `--current-origin`.

## Exact-host scope mapping

With `--current-origin`, browser-cli discovers the active CDP target, parses its URL, requires `https`, lowercases the exact hostname, and looks it up in a JSON object mapping hostnames to broker scopes. It does not use wildcard or suffix matching.

The default file is `$XDG_CONFIG_HOME/browser-cli/credential-scopes.json` when `XDG_CONFIG_HOME` is non-empty, otherwise `~/.config/browser-cli/credential-scopes.json`. The file must be a current-user-owned regular file with mode `0600`. Missing, malformed, symlinked, permissive-mode, foreign-owned, insecure, or unmapped state fails closed. If `--scope` is supplied with `--current-origin`, it must exactly equal the mapped scope.

Example:

```json
{
  "online.citi.com": "browser:citi"
}
```

The mapping contains no credentials.

## Fill state machine

`broker-fill` first sends a browser-fill request containing only the resolved scope and active CDP target ID through `secrets-broker-client`.

1. **Authorized:** return the broker's filled-count metadata. No unlock request is sent.
2. **Locked:** send one unlock request for the same exact resolved scope.
3. **Approval succeeds:** retry browser-fill exactly once.
4. **Approval denied or unavailable:** fail closed before retrying browser-fill.
5. **Retry is `Locked`:** fail closed; do not request approval or fill again.
6. **Other broker error:** fail closed without exposing broker error contents that could contain secrets.

The command prints only the number of credential fields filled. Browser-cli never receives or prints credential values.

## Browser boundary

The broker verifies the exact registered browser origin before filling its registered selectors through CDP. Browser-cli retains active-target discovery and sends the target ID; it does not independently implement credential storage or selector filling.

Broker fill does not submit the form. The browser operator identifies and clicks the sign-in control. MFA and CAPTCHA remain user-driven.

## Verification status

The `src/broker.rs` tests cover the request shape, authorized-fill fast path, exact-scope approval and single retry, denial/unavailable fail-closed behavior, second-`Locked` termination, exact HTTPS hostname mapping, mapping-file validation, and XDG/home path precedence.

Feature/source commit `34b850125ba037d411f2e4abde330cda3706e6d2` was pushed to remote `master`; later evidence/test-only commits do not change executable source. Local `./run-tests.sh` passed `cargo fmt`, `cargo clippy -- -D warnings`, and 62 feature tests; GitHub CI run `31033510969` passed. Test-only commit `f9a177bc1494cd022d2def573c186daf4c60f56e` added external proof that optional `broker-unlock` remains available; the current runner passes 63 tests. Deployment used `./deploy.sh`; installed/release SHA-256 is `a2bad6e754d3703e15f7a510f6309b1ae6e5b2e05d84d55133d421994361b0d3`, and no `browser-cli` process remained.

Fresh terminal-principal smoke used PID `1478667`, start time `151617732`, and TTY `34816`. The first exact `browser:citi` fill returned `Locked`; browser-cli requested exact leaf unlock once, approval/unlock was recorded, and one retry succeeded with only `Filled 2 credential fields` printed. No sign-in or submit occurred; the browser was navigated to `example.com` afterward to clear the form. The broker remained active as PID `3373670` with `NRestarts 0`.
