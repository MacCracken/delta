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

/// Validate a container image name. Must have at most one `:` (for tag separator),
/// no path traversal sequences, and no other dangerous characters.
/// Returns the validated image or falls back to a safe default.
fn validate_image(image: &str) -> String {
    let colon_count = image.chars().filter(|&c| c == ':').count();
    let has_traversal = image.contains("..") || image.contains('/');
    if colon_count > 1 || has_traversal || image.is_empty() {
        tracing::warn!(
            image = image,
            "invalid container image name, falling back to alpine:latest"
        );
        return "alpine:latest".to_string();
    }
    image.to_string()
}

/// Validate a work_dir path: must not contain `:` to prevent mount manipulation.
/// Returns the validated path or falls back to a safe default.
fn validate_work_dir(work_dir: &Path) -> std::path::PathBuf {
    let work_dir_str = work_dir.display().to_string();
    if work_dir_str.contains(':') {
        tracing::warn!(
            work_dir = %work_dir_str,
            "work_dir contains ':', falling back to /tmp"
        );
        return std::path::PathBuf::from("/tmp");
    }
    work_dir.to_path_buf()
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
    let safe_image = validate_image(image);
    let safe_work_dir = validate_work_dir(work_dir);

    let mut command = Command::new(runtime);
    command
        .arg("run")
        .arg("--rm")
        .arg("--network=none")
        .arg("--cap-drop=ALL")
        .arg("--security-opt=no-new-privileges")
        .arg("--read-only")
        .arg("--pids-limit=256")
        .arg("--memory=512m")
        .arg("--user=65534:65534"); // nobody:nogroup

    // Mount work directory and a writable /tmp
    command
        .arg("-v")
        .arg(format!("{}:/workspace", safe_work_dir.display()));
    command.arg("--tmpfs").arg("/tmp:rw,noexec,nosuid,size=64m");
    command.arg("-w").arg("/workspace");

    // Pass environment variables, skipping keys with invalid characters
    for (k, v) in env_vars {
        // H3: Skip env var keys containing `=`, newline, or null bytes
        if k.contains('=') || k.contains('\n') || k.contains('\0') {
            tracing::warn!(key = k, "skipping env var with invalid key");
            continue;
        }
        command.arg("-e").arg(format!("{}={}", k, v));
    }

    command.arg(&safe_image).arg("sh").arg("-c").arg(cmd);

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
