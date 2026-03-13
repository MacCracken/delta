//! Sandbox execution via Landlock LSM + seccomp BPF.
//!
//! Applied in child processes via `Command::pre_exec()` to restrict
//! filesystem access and block dangerous syscalls.

use landlock::{Access, BitFlags};
use std::path::Path;

/// Apply Landlock filesystem restrictions to the current process.
///
/// Restricts filesystem access to:
/// - Read-only: /usr, /bin, /lib, /lib64, /etc (system binaries and libs)
/// - Read-write: work_dir and /tmp
///
/// Gracefully degrades if the kernel doesn't support Landlock.
pub fn apply_landlock(work_dir: &Path) -> Result<(), String> {
    use landlock::{
        ABI, AccessFs, PathBeneath, PathFd, Ruleset, RulesetAttr, RulesetCreatedAttr, RulesetStatus,
    };

    let abi = ABI::V3;
    let read_access = AccessFs::from_read(abi);
    let full_access = AccessFs::from_all(abi);

    let status = Ruleset::default()
        .handle_access(full_access)
        .map_err(|e| format!("landlock: failed to handle access: {e}"))?
        .create()
        .map_err(|e| format!("landlock: failed to create ruleset: {e}"))?
        // Read-only: system paths
        .add_rules(path_beneath_rules(
            &["/usr", "/bin", "/lib", "/lib64", "/etc"],
            read_access,
        ))
        .map_err(|e| format!("landlock: failed to add read-only rules: {e}"))?
        // Read-write: work directory
        .add_rule(PathBeneath::new(
            PathFd::new(work_dir).map_err(|e| format!("landlock: work_dir fd: {e}"))?,
            full_access,
        ))
        .map_err(|e| format!("landlock: failed to add work_dir rule: {e}"))?
        // Read-write: /tmp
        .add_rule(PathBeneath::new(
            PathFd::new("/tmp").map_err(|e| format!("landlock: /tmp fd: {e}"))?,
            full_access,
        ))
        .map_err(|e| format!("landlock: failed to add /tmp rule: {e}"))?
        .restrict_self()
        .map_err(|e| format!("landlock: restrict_self failed: {e}"))?;

    match status.ruleset {
        RulesetStatus::FullyEnforced => {
            tracing::debug!("landlock: fully enforced");
        }
        RulesetStatus::PartiallyEnforced => {
            tracing::warn!("landlock: partially enforced (kernel may not support all features)");
        }
        RulesetStatus::NotEnforced => {
            tracing::warn!("landlock: not enforced (kernel does not support landlock)");
        }
    }

    Ok(())
}

/// Helper to create PathBeneath rules for multiple paths, skipping paths that
/// don't exist (e.g. /lib64 on some systems).
fn path_beneath_rules(
    paths: &[&str],
    access: BitFlags<landlock::AccessFs>,
) -> Vec<Result<landlock::PathBeneath<landlock::PathFd>, landlock::RulesetError>> {
    paths
        .iter()
        .filter_map(|p| {
            let path = Path::new(p);
            if !path.exists() {
                return None;
            }
            landlock::PathFd::new(p)
                .ok()
                .map(|fd| Ok(landlock::PathBeneath::new(fd, access)))
        })
        .collect()
}

/// Apply seccomp BPF filter to block dangerous syscalls.
///
/// Blocks: mount, umount2, pivot_root, chroot, ptrace, process_vm_readv,
/// process_vm_writev, reboot, kexec_load, init_module, finit_module,
/// delete_module, swapon, swapoff, acct, settimeofday, clock_settime.
pub fn apply_seccomp() -> Result<(), String> {
    use libc::{
        BPF_ABS, BPF_JEQ, BPF_JMP, BPF_K, BPF_LD, BPF_RET, BPF_W, PR_SET_NO_NEW_PRIVS,
        PR_SET_SECCOMP, SECCOMP_MODE_FILTER, SECCOMP_RET_ALLOW, SECCOMP_RET_ERRNO, sock_filter,
        sock_fprog,
    };

    // AUDIT_ARCH_X86_64 constant
    const AUDIT_ARCH_X86_64: u32 = 0xC000003E;

    // Syscall numbers (x86_64)
    const SYS_MOUNT: u32 = 165;
    const SYS_UMOUNT2: u32 = 166;
    const SYS_PIVOT_ROOT: u32 = 155;
    const SYS_CHROOT: u32 = 161;
    const SYS_PTRACE: u32 = 101;
    const SYS_PROCESS_VM_READV: u32 = 310;
    const SYS_PROCESS_VM_WRITEV: u32 = 311;
    const SYS_REBOOT: u32 = 169;
    const SYS_KEXEC_LOAD: u32 = 246;
    const SYS_INIT_MODULE: u32 = 175;
    const SYS_FINIT_MODULE: u32 = 313;
    const SYS_DELETE_MODULE: u32 = 176;
    const SYS_SWAPON: u32 = 167;
    const SYS_SWAPOFF: u32 = 168;
    const SYS_ACCT: u32 = 163;
    const SYS_SETTIMEOFDAY: u32 = 164;
    const SYS_CLOCK_SETTIME: u32 = 227;

    let blocked_syscalls: &[u32] = &[
        SYS_MOUNT,
        SYS_UMOUNT2,
        SYS_PIVOT_ROOT,
        SYS_CHROOT,
        SYS_PTRACE,
        SYS_PROCESS_VM_READV,
        SYS_PROCESS_VM_WRITEV,
        SYS_REBOOT,
        SYS_KEXEC_LOAD,
        SYS_INIT_MODULE,
        SYS_FINIT_MODULE,
        SYS_DELETE_MODULE,
        SYS_SWAPON,
        SYS_SWAPOFF,
        SYS_ACCT,
        SYS_SETTIMEOFDAY,
        SYS_CLOCK_SETTIME,
    ];

    // Build BPF program:
    // 1. Validate architecture (x86_64 only)
    // 2. Load syscall number
    // 3. For each blocked syscall, jump to ERRNO return
    // 4. Default: allow
    let mut filter: Vec<sock_filter> = vec![
        // Load architecture
        sock_filter {
            code: (BPF_LD | BPF_W | BPF_ABS) as u16,
            jt: 0,
            jf: 0,
            k: 4, // offsetof(struct seccomp_data, arch)
        },
        // Verify x86_64 — if not, allow everything
        sock_filter {
            code: (BPF_JMP | BPF_JEQ | BPF_K) as u16,
            jt: 1,
            jf: 0,
            k: AUDIT_ARCH_X86_64,
        },
        // Not x86_64: allow
        sock_filter {
            code: (BPF_RET | BPF_K) as u16,
            jt: 0,
            jf: 0,
            k: SECCOMP_RET_ALLOW,
        },
        // Load syscall number
        sock_filter {
            code: (BPF_LD | BPF_W | BPF_ABS) as u16,
            jt: 0,
            jf: 0,
            k: 0, // offsetof(struct seccomp_data, nr)
        },
    ];

    // For each blocked syscall: if match, jump to deny
    for (i, &syscall) in blocked_syscalls.iter().enumerate() {
        let remaining = (blocked_syscalls.len() - i - 1) as u8;
        filter.push(sock_filter {
            code: (BPF_JMP | BPF_JEQ | BPF_K) as u16,
            jt: remaining + 1, // Jump over remaining checks + allow to reach deny
            jf: 0,             // Continue checking
            k: syscall,
        });
    }

    // Default: allow
    filter.push(sock_filter {
        code: (BPF_RET | BPF_K) as u16,
        jt: 0,
        jf: 0,
        k: SECCOMP_RET_ALLOW,
    });

    // Deny: return EPERM
    filter.push(sock_filter {
        code: (BPF_RET | BPF_K) as u16,
        jt: 0,
        jf: 0,
        k: SECCOMP_RET_ERRNO | 1, // EPERM = 1
    });

    let prog = sock_fprog {
        len: filter.len() as u16,
        filter: filter.as_ptr() as *mut sock_filter,
    };

    unsafe {
        // Must set no_new_privs before installing seccomp filter
        if libc::prctl(PR_SET_NO_NEW_PRIVS, 1, 0, 0, 0) != 0 {
            return Err(format!(
                "prctl(PR_SET_NO_NEW_PRIVS) failed: {}",
                std::io::Error::last_os_error()
            ));
        }

        if libc::prctl(
            PR_SET_SECCOMP,
            SECCOMP_MODE_FILTER,
            &prog as *const sock_fprog,
        ) != 0
        {
            return Err(format!(
                "prctl(PR_SET_SECCOMP) failed: {}",
                std::io::Error::last_os_error()
            ));
        }
    }

    Ok(())
}

/// Apply both Landlock and seccomp sandboxing.
/// Called from `Command::pre_exec()` in the child process.
pub fn apply_sandbox(work_dir: &Path) -> Result<(), String> {
    apply_landlock(work_dir)?;
    apply_seccomp()?;
    Ok(())
}

/// Check if the current kernel supports Landlock.
pub fn landlock_supported() -> bool {
    use landlock::{ABI, Access, AccessFs, Ruleset, RulesetAttr};
    Ruleset::default()
        .handle_access(AccessFs::from_all(ABI::V3))
        .and_then(|r| r.create())
        .is_ok()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_path_beneath_rules_skips_missing() {
        let access = landlock::AccessFs::from_read(landlock::ABI::V3);
        let rules = path_beneath_rules(&["/nonexistent_path_xyz"], access);
        assert!(rules.is_empty());
    }

    #[test]
    fn test_path_beneath_rules_includes_existing() {
        let access = landlock::AccessFs::from_read(landlock::ABI::V3);
        let rules = path_beneath_rules(&["/tmp"], access);
        // /tmp should exist on any Linux system
        assert_eq!(rules.len(), 1);
    }
}
