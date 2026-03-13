//! Container-based step execution (Podman/Docker fallback).
//!
//! When kernel sandboxing (Landlock) is unavailable, steps can be executed
//! inside containers for isolation. Supports both Podman and Docker runtimes.

use std::collections::HashMap;
use std::path::Path;
use std::process::Stdio;
use tokio::process::Command;

/// Detect available container runtime. Prefers podman over docker.
pub fn detect_runtime() -> Option<String> {
    for runtime in &["podman", "docker"] {
        if std::process::Command::new(runtime)
            .arg("--version")
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .status()
            .is_ok_and(|s| s.success())
        {
            return Some(runtime.to_string());
        }
    }
    None
}

/// Build a `tokio::process::Command` that runs a step inside a container.
///
/// The work directory is bind-mounted at `/workspace` inside the container.
/// Environment variables are passed through via `-e` flags.
pub fn build_container_command(
    runtime: &str,
    image: &str,
    cmd: &str,
    work_dir: &Path,
    env_vars: &HashMap<String, String>,
) -> Command {
    let mut command = Command::new(runtime);
    command.arg("run").arg("--rm").arg("--network=host");

    // Mount work directory
    command
        .arg("-v")
        .arg(format!("{}:/workspace", work_dir.display()));
    command.arg("-w").arg("/workspace");

    // Pass environment variables
    for (k, v) in env_vars {
        command.arg("-e").arg(format!("{}={}", k, v));
    }

    command.arg(image).arg("sh").arg("-c").arg(cmd);

    command.stdout(Stdio::piped()).stderr(Stdio::piped());

    command
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_build_container_command_structure() {
        let mut env = HashMap::new();
        env.insert("FOO".into(), "bar".into());

        let cmd = build_container_command(
            "podman",
            "alpine:latest",
            "echo hello",
            Path::new("/tmp/work"),
            &env,
        );

        // Verify the command is constructed (we can't easily inspect tokio::Command internals,
        // but at least verify it doesn't panic)
        let _ = format!("{:?}", cmd);
    }

    #[test]
    fn test_detect_runtime_returns_option() {
        // Just verify it doesn't panic — actual result depends on system
        let _ = detect_runtime();
    }
}
