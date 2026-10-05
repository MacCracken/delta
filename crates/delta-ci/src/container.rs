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

/// Validate a container image reference of the form `name[:tag]`.
///
/// Only single-component names (official images such as `alpine:3.20` or
/// `rust:1.99`) are accepted. The name must start with a lowercase letter or
/// digit, so the image can never be parsed as a `docker run`/`podman run`
/// option (e.g. `--privileged`). Invalid images fall back to a safe default.
fn validate_image(image: &str) -> String {
    let (name, tag) = match image.split_once(':') {
        Some((name, tag)) => (name, Some(tag)),
        None => (image, None),
    };
    let name_ok = name.len() <= 128
        && name
            .chars()
            .next()
            .is_some_and(|c| c.is_ascii_lowercase() || c.is_ascii_digit())
        && name
            .chars()
            .all(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || matches!(c, '.' | '_' | '-'))
        && !name.contains("..");
    // Docker tag grammar: [A-Za-z0-9_][A-Za-z0-9_.-]{0,127}
    let tag_ok = tag.is_none_or(|t| {
        t.len() <= 128
            && t.chars()
                .next()
                .is_some_and(|c| c.is_ascii_alphanumeric() || c == '_')
            && t.chars()
                .all(|c| c.is_ascii_alphanumeric() || matches!(c, '_' | '.' | '-'))
    });
    if !(name_ok && tag_ok) {
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
    fn test_validate_image_accepts_official_images() {
        assert_eq!(validate_image("alpine"), "alpine");
        assert_eq!(validate_image("alpine:3.20"), "alpine:3.20");
        assert_eq!(validate_image("rust:1.99-bookworm"), "rust:1.99-bookworm");
    }

    #[test]
    fn test_validate_image_rejects_option_injection() {
        for image in [
            "--privileged",
            "-v",
            "--pid=host",
            "--cap-add=SYS_ADMIN",
            "--security-opt=seccomp=unconfined",
            "alpine:-x",
            ":latest",
            "",
            "Alpine",
            "evil/image",
            "a..b",
            "alpine:1:2",
            "alpine latest",
        ] {
            assert_eq!(validate_image(image), "alpine:latest", "accepted {image:?}");
        }
    }

    #[test]
    fn test_detect_runtime_returns_option() {
        // Just verify it doesn't panic — actual result depends on system
        let _ = detect_runtime();
    }

    #[test]
    fn test_validate_image_valid() {
        assert_eq!(validate_image("alpine:latest"), "alpine:latest");
        assert_eq!(validate_image("alpine"), "alpine");
        assert_eq!(validate_image("myimage:v1.2.3"), "myimage:v1.2.3");
    }

    #[test]
    fn test_validate_image_path_traversal() {
        assert_eq!(validate_image("../../../etc/passwd"), "alpine:latest");
        assert_eq!(validate_image("foo/bar"), "alpine:latest");
    }

    #[test]
    fn test_validate_image_multiple_colons() {
        assert_eq!(validate_image("image:tag:extra"), "alpine:latest");
    }

    #[test]
    fn test_validate_image_empty() {
        assert_eq!(validate_image(""), "alpine:latest");
    }

    #[test]
    fn test_validate_work_dir_normal() {
        let result = validate_work_dir(Path::new("/tmp/work"));
        assert_eq!(result, std::path::PathBuf::from("/tmp/work"));
    }

    #[test]
    fn test_validate_work_dir_with_colon() {
        let result = validate_work_dir(Path::new("/tmp:evil/work"));
        assert_eq!(result, std::path::PathBuf::from("/tmp"));
    }

    #[test]
    fn test_env_var_key_filtering() {
        let mut env = HashMap::new();
        env.insert("GOOD_KEY".into(), "value".into());
        env.insert("BAD=KEY".into(), "value".into());
        env.insert("BAD\nKEY".into(), "value".into());
        env.insert("BAD\0KEY".into(), "value".into());

        // Build a command — it should skip the bad keys.
        // We can't easily inspect tokio::Command args, but we verify it doesn't panic.
        let cmd = build_container_command(
            "podman",
            "alpine:latest",
            "echo test",
            Path::new("/tmp/work"),
            &env,
        );
        let debug = format!("{:?}", cmd);
        // The good key should appear in the debug output
        assert!(debug.contains("GOOD_KEY"));
    }
}
