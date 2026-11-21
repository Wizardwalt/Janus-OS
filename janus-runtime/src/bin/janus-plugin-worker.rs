//! One-request Lua worker. The parent supplies source/arguments over stdin
//! and receives one JSON response over stdout. No host capabilities are inherited.

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

/// Linux/Android process limits are applied before any plugin source is parsed
/// or executed. This is process/resource isolation, not a complete namespace,
/// filesystem, network, seccomp, or separate-UID sandbox.
#[cfg(any(target_os = "linux", target_os = "android"))]
fn apply_resource_limits() -> Result<()> {
    fn lower_limit(resource: libc::c_int, desired: libc::rlim_t, name: &str) -> Result<()> {
        let mut current = std::mem::MaybeUninit::<libc::rlimit>::uninit();
        let read_status = unsafe { libc::getrlimit(resource, current.as_mut_ptr()) };
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
        let write_status = unsafe { libc::setrlimit(resource, &limit) };
        if write_status != 0 {
            return Err(std::io::Error::last_os_error())
                .with_context(|| format!("setting {name} resource limit"));
        }
        Ok(())
    }

    lower_limit(libc::RLIMIT_CPU, 6, "CPU time")?;
    lower_limit(libc::RLIMIT_AS, 512 * 1024 * 1024, "address space")?;
    lower_limit(libc::RLIMIT_NOFILE, 16, "open file descriptors")?;
    lower_limit(libc::RLIMIT_FSIZE, 1024 * 1024, "file size")?;
    lower_limit(libc::RLIMIT_CORE, 0, "core dump")?;

    let status = unsafe { libc::prctl(libc::PR_SET_NO_NEW_PRIVS, 1, 0, 0, 0) };
    anyhow::ensure!(
        status == 0,
        "PR_SET_NO_NEW_PRIVS failed: {}",
        std::io::Error::last_os_error()
    );
    Ok(())
}

#[cfg(not(any(target_os = "linux", target_os = "android")))]
fn apply_resource_limits() -> Result<()> {
    anyhow::bail!("OS-limited plugin workers are supported only on Linux and Android; execution denied")
}
