use std::os::unix::fs::PermissionsExt;
use std::process::Command;

#[test]
fn installed_layouts_are_local_and_report_helper_failures() {
    let directory = tempfile::tempdir().unwrap();
    let helper = directory.path().join("localectl");
    std::fs::write(
        &helper,
        "#!/bin/sh\n[ \"$1\" = list-x11-keymap-layouts ] || exit 2\nprintf ' us\\n\\nde \\n'\n",
    )
    .unwrap();
    std::fs::set_permissions(&helper, std::fs::Permissions::from_mode(0o755)).unwrap();
    let run = |args: &[&str]| {
        Command::new(env!("CARGO_BIN_EXE_instantwmctl"))
            .args(args)
            .env("PATH", directory.path())
            .env("INSTANTWM_SOCKET", directory.path().join("absent.sock"))
            .output()
            .unwrap()
    };
    let text = run(&["keyboard", "available"]);
    assert!(
        text.status.success(),
        "{}",
        String::from_utf8_lossy(&text.stderr)
    );
    assert_eq!(text.stdout, b"us\nde\n");
    let json = run(&["--json", "keyboard", "available"]);
    assert!(json.status.success());
    assert_eq!(
        serde_json::from_slice::<Vec<String>>(&json.stdout).unwrap(),
        ["us", "de"]
    );

    std::fs::write(
        &helper,
        "#!/bin/sh\nprintf 'helper unavailable\\n' >&2\nexit 7\n",
    )
    .unwrap();
    let failed = run(&["keyboard", "available"]);
    assert!(!failed.status.success());
    let error = String::from_utf8_lossy(&failed.stderr);
    assert!(error.contains("helper unavailable") && error.contains('7'));
    let remote = run(&["keyboard", "list"]);
    assert!(!remote.status.success());
    assert!(String::from_utf8_lossy(&remote.stderr).contains("not running"));
}
