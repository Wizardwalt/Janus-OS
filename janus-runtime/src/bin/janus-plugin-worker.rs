//! One-request Lua worker. The parent supplies source/arguments over stdin
//! and receives one JSON response over stdout. No Janus capability IPC is exposed;
//! the worker still runs under the service account and is not a full OS sandbox.

#[path = "../lua.rs"]
mod lua;

use anyhow::{Context, Result};
use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::io::{Read, Write};

const MAX_REQUEST_BYTES: usize = 512 * 1024;

#[derive(Deserialize)]
struct WorkerRequest {
    code: String,
    args: Value,
}

#[derive(Serialize)]
#[serde(tag = "status", rename_all = "snake_case")]
enum WorkerResponse {
    Success { value: Value },
    Failure { error: String },
}

fn main() {
    let response = run_worker();
    let succeeded = matches!(&response, WorkerResponse::Success { .. });
    let encoded = serde_json::to_vec(&response)
        .unwrap_or_else(|_| br#"{"status":"failure","error":"response encoding failed"}"#.to_vec());
    let mut stdout = std::io::stdout().lock();
    if stdout.write_all(&encoded).is_err() || stdout.flush().is_err() {
        std::process::exit(2);
    }
    if !succeeded {
        std::process::exit(1);
    }
}

fn run_worker() -> WorkerResponse {
    if let Err(error) = apply_resource_limits() {
        return WorkerResponse::Failure {
            error: format!("OS worker limits could not be applied; execution denied: {error}"),
        };
    }

    let mut input = Vec::new();
    let stdin = std::io::stdin();
    let read_result = stdin
        .lock()
        .take((MAX_REQUEST_BYTES + 1) as u64)
        .read_to_end(&mut input);
    if let Err(error) = read_result {
        return WorkerResponse::Failure {
            error: format!("could not read worker request: {error}"),
        };
    }
    if input.len() > MAX_REQUEST_BYTES {
        return WorkerResponse::Failure {
            error: "worker request exceeds the 512 KiB limit".to_owned(),
        };
    }

    let request: WorkerRequest = match serde_json::from_slice(&input) {
        Ok(request) => request,
        Err(error) => {
            return WorkerResponse::Failure {
                error: format!("invalid worker request: {error}"),
            };
        }
    };
    if !request.args.is_object() {
        return WorkerResponse::Failure {
            error: "plugin arguments must be a JSON object".to_owned(),
        };
    }

    let sandbox = match lua::PluginSandbox::new() {
        Ok(sandbox) => sandbox,
        Err(error) => {
            return WorkerResponse::Failure {
                error: format!("could not initialize plugin sandbox: {error}"),
            };
        }
    };
    match sandbox.execute(&request.code, request.args) {
        Ok(value) => WorkerResponse::Success { value },
        Err(error) => WorkerResponse::Failure {
            error: error.to_string(),
        },
    }
}

#[cfg(any(target_os = "linux", target_os = "android"))]
fn install_worker_seccomp_filter() -> Result<()> {
    #[repr(C)]
    struct SockFilter { code: u16, jt: u8, jf: u8, k: u32 }
    #[repr(C)]
    struct SockFprog { len: u16, filter: *mut SockFilter }

    // AUDIT_ARCH values from linux/audit.h. Unknown architectures fail closed.
    #[cfg(target_arch = "x86_64")]
    let audit_arch = 0xc000_003e_u32;
    #[cfg(target_arch = "aarch64")]
    let audit_arch = 0xc000_00b7_u32;
    #[cfg(target_arch = "x86")]
    let audit_arch = 0x4000_0003_u32;
    #[cfg(target_arch = "arm")]
    let audit_arch = 0x4000_0028_u32;
    #[cfg(not(any(target_arch = "x86_64", target_arch = "aarch64", target_arch = "x86", target_arch = "arm")))]
    anyhow::bail!("seccomp worker policy is not defined for this CPU architecture");

    const BPF_LD_W_ABS: u16 = 0x20;
    const BPF_JMP_JEQ_K: u16 = 0x15;
    const BPF_RET_K: u16 = 0x06;
    const SECCOMP_RET_KILL_PROCESS: u32 = 0x8000_0000;
    const SECCOMP_RET_ERRNO: u32 = 0x0005_0000;
    const SECCOMP_RET_ALLOW: u32 = 0x7fff_0000;

    #[allow(unused_mut)]
    let mut denied_syscalls = vec![
        libc::SYS_socket,
        libc::SYS_socketpair,
        libc::SYS_connect,
        libc::SYS_accept,
        libc::SYS_accept4,
        libc::SYS_bind,
        libc::SYS_listen,
        libc::SYS_sendto,
        libc::SYS_recvfrom,
        libc::SYS_sendmsg,
        libc::SYS_recvmsg,
        libc::SYS_shutdown,
        libc::SYS_execve,
        libc::SYS_execveat,
        libc::SYS_ptrace,
        libc::SYS_clone,
        435, // clone3 is syscall 435 on the supported Linux ABIs.
    ];
    // fork/vfork have dedicated syscall numbers on x86; ARM ABIs implement
    // them through clone, which is denied above.
    #[cfg(target_arch = "x86_64")]
    denied_syscalls.extend([57, 58]);
    #[cfg(target_arch = "x86")]
    denied_syscalls.extend([2, 190]);
    // Legacy 32-bit x86 uses socketcall; ARM EABI exposes direct socket syscalls.
    #[cfg(target_arch = "x86")]
    denied_syscalls.push(libc::SYS_socketcall);

    // Validate seccomp_data.arch before syscall numbers; ABI mismatches are killed.
    let mut filter = Vec::with_capacity(5 + denied_syscalls.len() * 2);
    filter.push(SockFilter { code: BPF_LD_W_ABS, jt: 0, jf: 0, k: 4 });
    filter.push(SockFilter { code: BPF_JMP_JEQ_K, jt: 1, jf: 0, k: audit_arch });
    filter.push(SockFilter { code: BPF_RET_K, jt: 0, jf: 0, k: SECCOMP_RET_KILL_PROCESS });
    filter.push(SockFilter { code: BPF_LD_W_ABS, jt: 0, jf: 0, k: 0 });
    for syscall in denied_syscalls {
        filter.push(SockFilter { code: BPF_JMP_JEQ_K, jt: 0, jf: 1, k: syscall as u32 });
        filter.push(SockFilter {
            code: BPF_RET_K, jt: 0, jf: 0,
            k: SECCOMP_RET_ERRNO | libc::EPERM as u32,
        });
    }
    filter.push(SockFilter { code: BPF_RET_K, jt: 0, jf: 0, k: SECCOMP_RET_ALLOW });

    anyhow::ensure!(filter.len() <= u16::MAX as usize, "seccomp filter is too large");
    let program = SockFprog { len: filter.len() as u16, filter: filter.as_mut_ptr() };
    // PR_SET_SECCOMP = 22; SECCOMP_MODE_FILTER = 2.
    let status = unsafe { libc::prctl(22, 2, &program as *const SockFprog as libc::c_ulong, 0, 0) };
    anyhow::ensure!(
        status == 0,
        "installing worker seccomp filter failed: {}",
        std::io::Error::last_os_error()
    );
    Ok(())
}

#[cfg(not(any(target_os = "linux", target_os = "android")))]
fn install_worker_seccomp_filter() -> Result<()> {
    anyhow::bail!("seccomp worker policy is supported only on Linux and Android")
}

/// Linux/Android resource limits and a narrow seccomp deny-list are applied
/// before plugin source is parsed or executed. Network socket creation/use,
/// process creation/execution, and ptrace are denied. File access is not yet isolated,
/// and this is not a complete namespace, filesystem, or separate-UID sandbox.
#[cfg(any(target_os = "linux", target_os = "android"))]
fn apply_resource_limits() -> Result<()> {
    fn lower_limit(resource: libc::c_int, desired: libc::rlim_t, name: &str) -> Result<()> {
        let mut current = std::mem::MaybeUninit::<libc::rlimit>::uninit();
        let read_status = unsafe { libc::getrlimit(resource as _, current.as_mut_ptr()) };
        if read_status != 0 {
            return Err(std::io::Error::last_os_error())
                .with_context(|| format!("reading {name} resource limit"));
        }
        let current = unsafe { current.assume_init() };
        let cap = desired.min(current.rlim_cur).min(current.rlim_max);
        let limit = libc::rlimit {
            rlim_cur: cap,
            rlim_max: cap,
        };
        let write_status = unsafe { libc::setrlimit(resource as _, &limit) };
        if write_status != 0 {
            return Err(std::io::Error::last_os_error())
                .with_context(|| format!("setting {name} resource limit"));
        }
        Ok(())
    }

    lower_limit(libc::RLIMIT_CPU as libc::c_int, 6, "CPU time")?;
    lower_limit(libc::RLIMIT_AS as libc::c_int, 512 * 1024 * 1024, "address space")?;
    lower_limit(libc::RLIMIT_NOFILE as libc::c_int, 16, "open file descriptors")?;
    lower_limit(libc::RLIMIT_FSIZE as libc::c_int, 1024 * 1024, "file size")?;
    lower_limit(libc::RLIMIT_CORE as libc::c_int, 0, "core dump")?;

    let status = unsafe { libc::prctl(libc::PR_SET_DUMPABLE, 0, 0, 0, 0) };
    anyhow::ensure!(
        status == 0,
        "PR_SET_DUMPABLE failed: {}",
        std::io::Error::last_os_error()
    );

    let status = unsafe { libc::prctl(libc::PR_SET_NO_NEW_PRIVS, 1, 0, 0, 0) };
    anyhow::ensure!(
        status == 0,
        "PR_SET_NO_NEW_PRIVS failed: {}",
        std::io::Error::last_os_error()
    );
    install_worker_seccomp_filter()?;
    Ok(())
}

#[cfg(not(any(target_os = "linux", target_os = "android")))]
fn apply_resource_limits() -> Result<()> {
    anyhow::bail!("OS-limited plugin workers are supported only on Linux and Android; execution denied")
}


#[cfg(test)]
mod tests {
    use super::*;
    use std::process::Command;

    const SECCOMP_CHILD_ENV: &str = "JANUS_SECCOMP_PROBE_CHILD";

    #[test]
    fn seccomp_probe_child() {
        if std::env::var_os(SECCOMP_CHILD_ENV).is_none() { return; }
        apply_resource_limits().expect("worker resource and seccomp policy must install");
        unsafe {
            let socket = libc::socket(libc::AF_INET, libc::SOCK_STREAM, 0);
            assert_eq!(socket, -1, "socket creation must be denied");
            assert_eq!(std::io::Error::last_os_error().raw_os_error(), Some(libc::EPERM));

            let path = std::ffi::CString::new("/bin/true").unwrap();
            let argv = [path.as_ptr(), std::ptr::null()];
            let envp = [std::ptr::null()];
            let exec_status = libc::execve(path.as_ptr(), argv.as_ptr(), envp.as_ptr());
            assert_eq!(exec_status, -1, "process execution must be denied");
            assert_eq!(std::io::Error::last_os_error().raw_os_error(), Some(libc::EPERM));

            let execveat_status = libc::syscall(
                libc::SYS_execveat,
                libc::AT_FDCWD,
                path.as_ptr(),
                argv.as_ptr(),
                envp.as_ptr(),
                0,
            );
            assert_eq!(execveat_status, -1, "execveat must be denied");
            assert_eq!(std::io::Error::last_os_error().raw_os_error(), Some(libc::EPERM));

            let fork_status = libc::fork();
            assert_eq!(fork_status, -1, "process creation through fork must be denied");
            assert_eq!(std::io::Error::last_os_error().raw_os_error(), Some(libc::EPERM));

            let clone3_status = libc::syscall(
                435,
                std::ptr::null::<libc::c_void>(),
                0,
            );
            assert_eq!(clone3_status, -1, "clone3 must be denied or unavailable");
            assert!(
                matches!(
                    std::io::Error::last_os_error().raw_os_error(),
                    Some(libc::EPERM) | Some(libc::ENOSYS)
                ),
                "clone3 must not be allowed when the kernel implements it"
            );

            let ptrace_status = libc::ptrace(
                libc::PTRACE_TRACEME,
                0,
                std::ptr::null_mut::<libc::c_void>(),
                std::ptr::null_mut::<libc::c_void>(),
            );
            assert_eq!(ptrace_status, -1, "ptrace must be denied");
            assert_eq!(std::io::Error::last_os_error().raw_os_error(), Some(libc::EPERM));
        }
    }

    #[test]
    fn worker_seccomp_denies_socket_exec_and_process_creation() {
        let executable = std::env::current_exe().expect("test executable path");
        let output = Command::new(executable)
            .args(["--exact", "tests::seccomp_probe_child", "--nocapture"])
            .env(SECCOMP_CHILD_ENV, "1")
            .output()
            .expect("launch isolated seccomp probe process");
        assert!(
            output.status.success(),
            "seccomp probe failed: stdout={} stderr={}",
            String::from_utf8_lossy(&output.stdout),
            String::from_utf8_lossy(&output.stderr)
        );
    }
}
