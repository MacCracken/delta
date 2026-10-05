//! Sandbox execution via Landlock LSM + seccomp BPF.
//!
//! The sandbox is prepared in the parent process ([`PreparedSandbox::prepare`])
//! and applied in the child between `fork` and `exec` via
//! `Command::pre_exec()` ([`PreparedSandbox::apply`]). Only raw syscalls run in
//! the child: anything that allocates or takes locks there can deadlock when
//! the parent is a multi-threaded async runtime.

use landlock::{Access, AccessFs, BitFlags, PathBeneath, PathFd, RulesetCreated};
use std::path::Path;

/// Read-only system locations available to sandboxed steps.
///
/// `/etc` is deliberately not granted as a whole: it holds the server's own
/// configuration (e.g. `/etc/delta/config.toml` with `secrets_key`). Only
/// the entries that common tools need are listed. Missing paths are skipped.
const READ_ONLY_PATHS: &[&str] = &[
    "/usr",
    "/bin",
    "/sbin",
    "/lib",
    "/lib32",
    "/lib64",
    "/etc/alternatives",
    "/etc/ca-certificates",
    "/etc/gai.conf",
    "/etc/gitconfig",
    "/etc/group",
    "/etc/host.conf",
    "/etc/hosts",
    "/etc/ld.so.cache",
    "/etc/ld.so.conf",
    "/etc/ld.so.conf.d",
    "/etc/localtime",
    "/etc/mime.types",
    "/etc/nsswitch.conf",
    "/etc/os-release",
    "/etc/passwd",
    "/etc/pki",
    "/etc/protocols",
    "/etc/resolv.conf",
    "/etc/services",
    "/etc/ssl",
    "/etc/timezone",
];

/// Device nodes that scripts routinely read from or redirect to.
const DEVICE_PATHS: &[&str] = &["/dev/null", "/dev/zero", "/dev/random", "/dev/urandom"];

/// A sandbox built in the parent process, ready to be applied in a child.
pub struct PreparedSandbox {
    ruleset: Option<RulesetCreated>,
    seccomp_filter: Vec<libc::sock_filter>,
}

impl PreparedSandbox {
    /// Build the Landlock ruleset and seccomp program for a step that may
    /// write only to `work_dir` and `/tmp`.
    pub fn prepare(work_dir: &Path) -> Result<Self, String> {
        Ok(Self {
            ruleset: Some(build_landlock_ruleset(work_dir)?),
            seccomp_filter: build_seccomp_filter(),
        })
    }

    /// Restrict the calling process. Async-signal-safe: only syscalls, no
    /// allocation, so it may run in a `pre_exec` hook.
    pub fn apply(&mut self) -> std::io::Result<()> {
        if let Some(ruleset) = self.ruleset.take() {
            ruleset
                .restrict_self()
                .map_err(|_| std::io::Error::from_raw_os_error(libc::EPERM))?;
        }
        if self.seccomp_filter.is_empty() {
            return Ok(());
        }
        let prog = libc::sock_fprog {
            len: self.seccomp_filter.len() as u16,
            filter: self.seccomp_filter.as_mut_ptr(),
        };
        // SAFETY: plain prctl calls; `prog` points at a filter that outlives them.
        unsafe {
            // Required before installing a filter without CAP_SYS_ADMIN.
            if libc::prctl(libc::PR_SET_NO_NEW_PRIVS, 1, 0, 0, 0) != 0 {
                return Err(std::io::Error::last_os_error());
            }
            if libc::prctl(
                libc::PR_SET_SECCOMP,
                libc::SECCOMP_MODE_FILTER,
                &prog as *const libc::sock_fprog,
            ) != 0
            {
                return Err(std::io::Error::last_os_error());
            }
        }
        Ok(())
    }
}

/// Build a Landlock ruleset allowing read access to system paths and full
/// access beneath `work_dir` and `/tmp`. Gracefully degrades (best effort)
/// if the kernel doesn't support every access right.
fn build_landlock_ruleset(work_dir: &Path) -> Result<RulesetCreated, String> {
    use landlock::{ABI, Ruleset, RulesetAttr, RulesetCreatedAttr};

    let abi = ABI::V3;
    let read_access = AccessFs::from_read(abi);
    let full_access = AccessFs::from_all(abi);
    let device_access = AccessFs::ReadFile | AccessFs::WriteFile;

    Ruleset::default()
        .handle_access(full_access)
        .map_err(|e| format!("landlock: failed to handle access: {e}"))?
        .create()
        .map_err(|e| format!("landlock: failed to create ruleset: {e}"))?
        .add_rules(path_beneath_rules(READ_ONLY_PATHS, read_access))
        .map_err(|e| format!("landlock: failed to add read-only rules: {e}"))?
        .add_rules(path_beneath_rules(DEVICE_PATHS, device_access))
        .map_err(|e| format!("landlock: failed to add device rules: {e}"))?
        .add_rule(PathBeneath::new(
            PathFd::new(work_dir).map_err(|e| format!("landlock: work_dir fd: {e}"))?,
            full_access,
        ))
        .map_err(|e| format!("landlock: failed to add work_dir rule: {e}"))?
        .add_rule(PathBeneath::new(
            PathFd::new("/tmp").map_err(|e| format!("landlock: /tmp fd: {e}"))?,
            full_access,
        ))
        .map_err(|e| format!("landlock: failed to add /tmp rule: {e}"))
}

/// Helper to create PathBeneath rules for multiple paths, skipping paths that
/// don't exist (e.g. /lib64 on some systems).
fn path_beneath_rules(
    paths: &[&str],
    access: BitFlags<AccessFs>,
) -> Vec<Result<PathBeneath<PathFd>, landlock::RulesetError>> {
    paths
        .iter()
        .filter_map(|p| {
            let path = Path::new(p);
            if !path.exists() {
                return None;
            }
            PathFd::new(p)
                .ok()
                .map(|fd| Ok(PathBeneath::new(fd, access)))
        })
        .collect()
}

/// Build a seccomp BPF program that blocks dangerous syscalls with EPERM.
///
/// Blocks: mount, umount2, pivot_root, chroot, ptrace, process_vm_readv,
/// process_vm_writev, reboot, kexec_load, init_module, finit_module,
/// delete_module, swapon, swapoff, acct, settimeofday, clock_settime.
///
/// Syscalls made through another ABI (i386 via `int 0x80`, or x32) use
/// different numbers and would slip past a deny-list, so they are refused
/// outright. Returns an empty program on architectures other than x86_64.
fn build_seccomp_filter() -> Vec<libc::sock_filter> {
    #[cfg(not(target_arch = "x86_64"))]
    {
        tracing::warn!(
            "seccomp filter skipped: syscall numbers are x86_64-only (current arch is not x86_64)"
        );
        Vec::new()
    }

    #[cfg(target_arch = "x86_64")]
    {
        use libc::{
            BPF_ABS, BPF_JEQ, BPF_JGE, BPF_JMP, BPF_K, BPF_LD, BPF_RET, BPF_W, SECCOMP_RET_ALLOW,
            SECCOMP_RET_ERRNO, sock_filter,
        };

        const AUDIT_ARCH_X86_64: u32 = 0xC000_003E;
        /// Bit set in syscall numbers of the x32 ABI.
        const X32_SYSCALL_BIT: u32 = 0x4000_0000;
        const BLOCKED_SYSCALLS: &[u32] = &[
            165, // mount
            166, // umount2
            155, // pivot_root
            161, // chroot
            101, // ptrace
            310, // process_vm_readv
            311, // process_vm_writev
            169, // reboot
            246, // kexec_load
            175, // init_module
            313, // finit_module
            176, // delete_module
            167, // swapon
            168, // swapoff
            163, // acct
            164, // settimeofday
            227, // clock_settime
        ];

        let insn = |code: u32, jt: usize, jf: usize, k: u32| sock_filter {
            code: code as u16,
            jt: jt as u8,
            jf: jf as u8,
            k,
        };
        // Layout: [0] load arch, [1] arch check, [2] load nr, [3] x32 check,
        // [4..4+n) blocked checks, [4+n] allow, [5+n] deny.
        let deny = 5 + BLOCKED_SYSCALLS.len();
        let jump_to_deny = |from: usize| deny - from - 1;

        let mut filter = vec![
            // offsetof(struct seccomp_data, arch)
            insn(BPF_LD | BPF_W | BPF_ABS, 0, 0, 4),
            insn(
                BPF_JMP | BPF_JEQ | BPF_K,
                0,
                jump_to_deny(1),
                AUDIT_ARCH_X86_64,
            ),
            // offsetof(struct seccomp_data, nr)
            insn(BPF_LD | BPF_W | BPF_ABS, 0, 0, 0),
            insn(
                BPF_JMP | BPF_JGE | BPF_K,
                jump_to_deny(3),
                0,
                X32_SYSCALL_BIT,
            ),
        ];
        for (i, &nr) in BLOCKED_SYSCALLS.iter().enumerate() {
            filter.push(insn(BPF_JMP | BPF_JEQ | BPF_K, jump_to_deny(4 + i), 0, nr));
        }
        filter.push(insn(BPF_RET | BPF_K, 0, 0, SECCOMP_RET_ALLOW));
        filter.push(insn(
            BPF_RET | BPF_K,
            0,
            0,
            SECCOMP_RET_ERRNO | libc::EPERM as u32,
        ));
        debug_assert_eq!(filter.len(), deny + 1);
        filter
    }
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

    #[cfg(target_arch = "x86_64")]
    #[test]
    fn test_seccomp_jumps_land_on_deny() {
        let filter = build_seccomp_filter();
        let deny = filter.len() - 1;
        assert_eq!(filter[deny].k, libc::SECCOMP_RET_ERRNO | libc::EPERM as u32);
        assert_eq!(filter[deny - 1].k, libc::SECCOMP_RET_ALLOW);
        // Wrong-arch (jf of the arch check) and every syscall check (jt)
        // must target the deny instruction.
        assert_eq!(1 + 1 + filter[1].jf as usize, deny);
        for (i, insn) in filter.iter().enumerate().take(deny - 1).skip(3) {
            assert_eq!(i + 1 + insn.jt as usize, deny, "instruction {i}");
            assert_eq!(insn.jf, 0);
        }
    }

    /// Run `script` in the Landlock sandbox; returns whether it succeeded.
    async fn run_sandboxed(work_dir: &Path, script: &str) -> bool {
        let mut prepared = PreparedSandbox::prepare(work_dir).unwrap();
        let mut command = tokio::process::Command::new("sh");
        command.arg("-c").arg(script).current_dir(work_dir);
        // SAFETY: `apply` is async-signal-safe.
        unsafe {
            command.pre_exec(move || prepared.apply());
        }
        command.status().await.unwrap().success()
    }

    #[tokio::test]
    async fn test_sandbox_restricts_filesystem() {
        if !landlock_supported() {
            eprintln!("skipping: kernel lacks Landlock");
            return;
        }
        let work = tempfile::tempdir().unwrap();
        let wd = work.path();

        // Ordinary script plumbing keeps working.
        assert!(run_sandboxed(wd, "echo hi > /dev/null && cat /etc/passwd > out.txt").await);
        assert!(wd.join("out.txt").exists());

        // Writes outside the work dir and /tmp are refused.
        let outside = tempfile::tempdir_in(std::env::current_dir().unwrap()).unwrap();
        let target = outside.path().join("x");
        assert!(!run_sandboxed(wd, &format!("echo x > '{}'", target.display())).await);
        assert!(!target.exists());

        // /etc is not readable wholesale (server config lives there).
        if let Some(denied) = ["/etc/hostname", "/etc/fstab", "/etc/shells"]
            .into_iter()
            .find(|p| Path::new(p).exists())
        {
            assert!(!run_sandboxed(wd, &format!("cat {denied}")).await);
        }
    }

    #[test]
    fn test_path_beneath_rules_includes_existing() {
        let access = landlock::AccessFs::from_read(landlock::ABI::V3);
        let rules = path_beneath_rules(&["/tmp"], access);
        // /tmp should exist on any Linux system
        assert_eq!(rules.len(), 1);
    }
}
