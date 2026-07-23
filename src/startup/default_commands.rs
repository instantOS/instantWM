use std::io::ErrorKind;
use std::os::unix::fs::PermissionsExt;
use std::path::Path;
use std::process::Command;

const REQUIRED_ALIASES: &[&str] = &[
    "terminal",
    "termfilemanager",
    "filemanager",
    "appmenu",
    "lockscreen",
    "browser",
    "editor",
    "systemmonitor",
];

pub(super) fn ensure_default_command_aliases() {
    let Some(home) = dirs::home_dir() else {
        eprintln!("instantwm: cannot determine home directory for default application aliases");
        return;
    };
    let default_dir = home.join(".config/instantos/default");
    if missing_aliases(&default_dir).is_empty() {
        return;
    }

    match Command::new("ins")
        .args(["settings", "ensure-defaults"])
        .status()
    {
        Ok(status) if !status.success() => {
            eprintln!("instantwm: 'ins settings ensure-defaults' exited with {status}");
        }
        Ok(_) => {}
        Err(error) if error.kind() == ErrorKind::NotFound => {
            eprintln!(
                "instantwm: default application aliases are missing and 'ins' is not installed"
            );
        }
        Err(error) => {
            eprintln!("instantwm: failed to ensure default application aliases: {error}");
        }
    }

    let missing = missing_aliases(&default_dir);
    if !missing.is_empty() {
        eprintln!(
            "instantwm: no installed default application could be found for: {}. Configure these in 'ins settings'.",
            missing.join(", ")
        );
    }
}

fn missing_aliases(default_dir: &Path) -> Vec<&'static str> {
    REQUIRED_ALIASES
        .iter()
        .copied()
        .filter(|alias| !is_executable(&default_dir.join(alias)))
        .collect()
}

fn is_executable(path: &Path) -> bool {
    match path.metadata() {
        Ok(metadata) if metadata.is_file() => metadata.permissions().mode() & 0o111 != 0,
        _ => false,
    }
}

#[cfg(test)]
mod tests {
    use std::fs;
    use std::os::unix::fs::{PermissionsExt, symlink};
    use std::path::{Path, PathBuf};
    use std::sync::atomic::{AtomicU64, Ordering};

    use super::*;

    struct TestDir(PathBuf);

    impl TestDir {
        fn new() -> Self {
            static NEXT_ID: AtomicU64 = AtomicU64::new(0);
            let path = std::env::temp_dir().join(format!(
                "instantwm-default-commands-{}-{}",
                std::process::id(),
                NEXT_ID.fetch_add(1, Ordering::Relaxed)
            ));
            fs::create_dir(&path).unwrap();
            Self(path)
        }

        fn path(&self) -> &Path {
            &self.0
        }
    }

    impl Drop for TestDir {
        fn drop(&mut self) {
            let _ = fs::remove_dir_all(&self.0);
        }
    }

    #[test]
    fn missing_aliases_rejects_absent_broken_and_non_executable_entries() {
        let temp = TestDir::new();
        let executable = temp.path().join("target");
        fs::write(&executable, "#!/bin/sh\n").unwrap();
        fs::set_permissions(&executable, fs::Permissions::from_mode(0o755)).unwrap();

        symlink(&executable, temp.path().join("terminal")).unwrap();
        symlink(
            temp.path().join("absent"),
            temp.path().join("termfilemanager"),
        )
        .unwrap();
        fs::write(temp.path().join("filemanager"), "").unwrap();

        let missing = missing_aliases(temp.path());

        assert!(!missing.contains(&"terminal"));
        assert!(missing.contains(&"termfilemanager"));
        assert!(missing.contains(&"filemanager"));
        assert!(missing.contains(&"browser"));
    }
}
