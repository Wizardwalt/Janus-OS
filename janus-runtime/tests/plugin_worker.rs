use serde_json::{json, Value};
use std::io::Write;
use std::process::{Command, ExitStatus, Stdio};

fn invoke_worker(request: &Value) -> (ExitStatus, Value) {
    let mut child = Command::new(env!("CARGO_BIN_EXE_janus-plugin-worker"))
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::null())
        .spawn()
        .expect("worker binary should start");
    let payload = serde_json::to_vec(request).unwrap();
    if let Some(mut stdin) = child.stdin.take() {
        let _ = stdin.write_all(&payload);
    }
    let output = child.wait_with_output().expect("worker should exit");
    let response = serde_json::from_slice(&output.stdout)
        .expect("worker should return one JSON response");
    (output.status, response)
}

#[test]
fn worker_applies_os_limits_and_runs_with_only_explicit_lua_capabilities() {
    let (status, response) = invoke_worker(&json!({
        "code": "return { answer = args.answer + 1, print_type = type(print), os_type = type(os), io_type = type(io), package_type = type(package), log_type = type(janus.log) }",
        "args": { "answer": 41 }
    }));
    assert!(status.success(), "worker failed: {response}");
    assert_eq!(response["status"], "success");
    assert_eq!(response["value"]["answer"], 42);
    assert_eq!(response["value"]["print_type"], "nil");
    assert_eq!(response["value"]["os_type"], "nil");
    assert_eq!(response["value"]["io_type"], "nil");
    assert_eq!(response["value"]["package_type"], "nil");
    assert_eq!(response["value"]["log_type"], "function");
}

#[test]
fn worker_bounds_cpu_instruction_loops_and_returns_a_failure_envelope() {
    let (status, response) = invoke_worker(&json!({
        "code": "while true do end",
        "args": {}
    }));
    assert!(!status.success());
    assert_eq!(response["status"], "failure");
    assert!(response["error"].as_str().unwrap().contains("instruction limit"));
}

#[test]
fn worker_rejects_non_object_arguments() {
    let (status, response) = invoke_worker(&json!({
        "code": "return 1",
        "args": [1, 2, 3]
    }));
    assert!(!status.success());
    assert_eq!(response["status"], "failure");
    assert!(response["error"].as_str().unwrap().contains("JSON object"));
}
