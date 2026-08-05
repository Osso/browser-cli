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
