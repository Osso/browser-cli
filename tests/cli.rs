use std::process::Command;

#[test]
fn broker_unlock_remains_available_as_an_optional_command() {
    let output = Command::new(env!("CARGO_BIN_EXE_browser-cli"))
        .args(["broker-unlock", "--help"])
        .output()
        .unwrap();

    assert!(output.status.success());
    let stdout = String::from_utf8(output.stdout).unwrap();
    assert!(stdout.contains("Request approval to unlock a Secrets Broker scope"));
    assert!(stdout.contains("--current-origin"));
}

fn browser_cli(args: &[&str], env: &[(&str, &str)]) -> std::process::Output {
    let mut command = Command::new(env!("CARGO_BIN_EXE_browser-cli"));
    command
        .args(args)
        .env_remove("BROWSER_CLI_TAB")
        .env_remove("BROWSER_CLI_SESSION");
    for (key, value) in env {
        command.env(key, value);
    }
    command.output().unwrap()
}

#[test]
fn page_commands_without_a_tab_or_session_fail_instead_of_using_the_active_tab() {
    // Port 1 has no browser: the command must fail on selection before touching Chrome.
    for args in [
        vec!["--port", "1", "eval", "1"],
        vec!["--port", "1", "open", "https://example.com"],
        vec!["--port", "1", "tabs", "close"],
    ] {
        let output = browser_cli(&args, &[]);
        assert!(!output.status.success(), "{args:?} succeeded");
        let stderr = String::from_utf8(output.stderr).unwrap();
        assert!(stderr.contains("No tab selected"), "{args:?}: {stderr}");
    }
}

#[test]
fn tab_and_session_together_are_rejected() {
    let output = browser_cli(&["--tab", "A", "--session", "s", "eval", "1"], &[]);
    assert!(!output.status.success());
    let stderr = String::from_utf8(output.stderr).unwrap();
    assert!(stderr.contains("either --tab or --session"), "{stderr}");
}

#[test]
fn explicit_tab_flag_overrides_the_environment_session() {
    // With the env session ignored, --tab is the selector, so `tabs switch` rejects it
    // before contacting Chrome (a session would have been rebound instead).
    let output = browser_cli(
        &["--port", "1", "--tab", "A", "tabs", "switch", "0"],
        &[("BROWSER_CLI_SESSION", "agent-session")],
    );
    assert!(!output.status.success());
    let stderr = String::from_utf8(output.stderr).unwrap();
    assert!(stderr.contains("--tab already names the tab"), "{stderr}");
}
