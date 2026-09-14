//! Run Studio with health-checked binary activation and automatic rollback.
#[path = "../activation.rs"]
mod activation;
use std::{
    fs, io,
    path::{Path, PathBuf},
    process::{Child, Command},
    time::{Duration, Instant},
};
fn start(binary: &Path, root: &Path, port: u16) -> io::Result<Child> {
    let mut command = Command::new(binary);
    command
        .args(["--root"])
        .arg(root)
        .args(["--port", &port.to_string()])
        .env("KLYNE_SUPERVISED", "1");
    #[cfg(windows)]
    {
        use std::os::windows::process::CommandExt;
        command.creation_flags(0x08000000);
    }
    command.spawn()
}
fn health(port: u16) -> Option<serde_json::Value> {
    reqwest::blocking::Client::builder()
        .no_proxy()
        .timeout(Duration::from_secs(1))
        .build()
        .ok()?
        .get(format!("http://127.0.0.1:{port}/api/runtime/ready"))
        .send()
        .ok()?
        .json()
        .ok()
}
fn healthy(child: &mut Child, port: u16) -> bool {
    let deadline = Instant::now() + Duration::from_secs(15);
    while Instant::now() < deadline {
        if child.try_wait().ok().flatten().is_some() {
            return false;
        }
        if health(port).is_some_and(|v| v["pid"] == child.id()) {
            return true;
        }
        std::thread::sleep(Duration::from_millis(100));
    }
    false
}
fn main() -> io::Result<()> {
    let mut args = std::env::args().skip(1);
    let mut root = PathBuf::from("workspace/studio");
    let mut port = 4317;
    let mut app_browser: Option<PathBuf> = None;
    let mut binary = std::env::current_exe()?
        .with_file_name(format!("klyne-studio{}", std::env::consts::EXE_SUFFIX));
    // Execute the supplied test argv under supervision, then record a receipt
    // only if it succeeds and the candidate digest remains unchanged.
    // Staging and activation both require a fresh runner receipt.
    let mut attest_digest: Option<String> = None;
    let mut attest_command: Option<String> = None;
    while let Some(arg) = args.next() {
        match arg.as_str() {
            "--app-browser" => {
                app_browser = Some(
                    args.next()
                        .ok_or_else(|| io::Error::other("Missing browser"))?
                        .into(),
                );
            }
            "--root" => {
                root = args
                    .next()
                    .ok_or_else(|| io::Error::other("Missing root"))?
                    .into()
            }
            "--attest-digest" => {
                attest_digest = Some(
                    args.next()
                        .ok_or_else(|| io::Error::other("Missing digest"))?,
                )
            }
            "--attest-command" => {
                attest_command = Some(
                    args.next()
                        .ok_or_else(|| io::Error::other("Missing command"))?,
                )
            }
            "--port" => {
                port = args
                    .next()
                    .ok_or_else(|| io::Error::other("Missing port"))?
                    .parse()
                    .map_err(io::Error::other)?
            }
            "--binary" => {
                binary = args
                    .next()
                    .ok_or_else(|| io::Error::other("Missing binary"))?
                    .into()
            }
            _ => {
                return Err(io::Error::other(
                    "Use --root, --port, --binary, --app-browser, or --attest-digest with --attest-command",
                ));
            }
        }
    }
    if let (Some(digest), Some(command)) = (attest_digest.as_ref(), attest_command.as_ref()) {
        let command: Vec<String> = serde_json::from_str(command).map_err(|_| {
            io::Error::other("--attest-command must be a JSON argv array; it will be executed")
        })?;
        let attestation = activation::run_tests(
            &root,
            &binary,
            digest,
            &command,
            &std::env::current_dir()?,
            &std::sync::atomic::AtomicBool::new(false),
        )?;
        println!(
            "{}",
            serde_json::to_string(&attestation).map_err(io::Error::other)?
        );
        return Ok(());
    }
    if attest_digest.is_some() || attest_command.is_some() {
        return Err(io::Error::other(
            "Attestation needs both --attest-digest and --attest-command",
        ));
    }
    fs::create_dir_all(&root)?;
    root = fs::canonicalize(root)?;
    let owner = fs::OpenOptions::new()
        .create(true)
        .truncate(false)
        .read(true)
        .write(true)
        .open(root.join("supervisor.lock"))?;
    owner
        .try_lock()
        .map_err(|_| io::Error::other("Another supervisor owns this root"))?;
    fs::create_dir_all(root.join("runtime"))?;
    if let Ok(bytes) = fs::read(root.join("runtime/current.json"))
        && let Ok(current) = serde_json::from_slice::<activation::Candidate>(&bytes)
        && activation::digest(&current.binary).ok().as_deref() == Some(&current.sha256)
    {
        binary = current.binary;
    }
    let mut child = start(&binary, &root, port)?;
    let mut job = harness_core::process_job::ProcessJob::attach(&child)?;
    if !healthy(&mut child, port) {
        job.terminate();
        let _ = child.kill();
        return Err(io::Error::other("Studio failed its startup health check"));
    }
    // A dedicated browser profile prevents attaching to the user's personal
    // browser. Its last window owns the lifetime of this entire app session.
    let mut app_window = if let Some(browser) = app_browser {
        let mut window = Command::new(browser)
            .arg(format!("--app=http://127.0.0.1:{port}"))
            .arg(format!(
                "--user-data-dir={}",
                root.join("app-browser").display()
            ))
            .args([
                "--no-first-run",
                "--no-default-browser-check",
                "--disable-background-mode",
            ])
            .spawn()?;
        let window_job = match harness_core::process_job::ProcessJob::attach(&window) {
            Ok(job) => job,
            Err(error) => {
                let _ = window.kill();
                let _ = window.wait();
                return Err(error);
            }
        };
        Some((window, window_job))
    } else {
        None
    };
    let mut crashes = 0u32;
    let mut healthy_since = Instant::now();
    loop {
        if let Some((window, _)) = app_window.as_mut()
            && window.try_wait()?.is_some()
        {
            job.terminate();
            let _ = child.kill();
            let _ = child.wait();
            return Ok(());
        }
        if let Some(status) = child.try_wait()? {
            job.terminate();
            let _ = child.wait();
            if healthy_since.elapsed() >= Duration::from_secs(60) {
                crashes = 0;
            }
            crashes += 1;
            fs::write(
                root.join("runtime/restart-status.json"),
                serde_json::to_vec(
                    &serde_json::json!({"attempt":crashes,"exhausted":crashes>3,"exit":status.to_string(),"message":if crashes>3 {"Repeated crashes; checkpoints retained. Inspect the runtime before restarting."}else{"Restarting Studio from saved checkpoints"}}),
                )?,
            )?;
            if crashes > 3 {
                return Err(io::Error::other(
                    "Studio crash loop; stopped after three restart attempts",
                ));
            }
            std::thread::sleep(Duration::from_millis(250 * (1u64 << (crashes - 1))));
            child = start(&binary, &root, port)?;
            job = harness_core::process_job::ProcessJob::attach(&child)?;
            healthy_since = Instant::now();
            if !healthy(&mut child, port) {
                job.terminate();
                let _ = child.kill();
                let _ = child.wait();
            }
            continue;
        }
        let pending = root.join("runtime/pending.json");
        if let Ok(bytes) = fs::read(&pending)
            && let Ok(candidate) = serde_json::from_slice::<activation::Candidate>(&bytes)
            && health(port).is_some_and(|v| v["ready"] == true && v["pid"] == child.id())
        {
            let result = activation::preflight(&root, &candidate);
            let rejected = result.as_ref().err().map(ToString::to_string);
            if result.is_ok() {
                let client = reqwest::blocking::Client::builder()
                    .no_proxy()
                    .timeout(Duration::from_secs(3))
                    .build()
                    .map_err(io::Error::other)?;
                let response = client
                    .post(format!("http://127.0.0.1:{port}/api/runtime/quiesce"))
                    .header("X-Klyne-Request", "1")
                    .json(&serde_json::json!({}))
                    .send()
                    .map_err(io::Error::other)?;
                if !response.status().is_success() {
                    return Err(io::Error::other("Could not quiesce runtime"));
                }
                while !health(port).is_some_and(|v| v["ready"] == true && v["pid"] == child.id()) {
                    if child.try_wait()?.is_some() {
                        return Err(io::Error::other("Runtime exited while checkpointing"));
                    }
                    std::thread::sleep(Duration::from_millis(100));
                }
                let previous = activation::Candidate {
                    binary: binary.clone(),
                    sha256: activation::digest(&binary)?,
                };
                fs::write(
                    root.join("runtime/previous.json"),
                    serde_json::to_vec(&previous)?,
                )?;
                job.terminate();
                let _ = child.kill();
                let _ = child.wait();
                let launched = start(&candidate.binary, &root, port);
                let launch_failed = launched.is_err();
                child = match launched {
                    Ok(child) => child,
                    Err(_) => start(&previous.binary, &root, port)?,
                };
                job = harness_core::process_job::ProcessJob::attach(&child)?;
                if !launch_failed && healthy(&mut child, port) {
                    binary = candidate.binary.clone();
                    fs::write(
                        root.join("runtime/current.json"),
                        serde_json::to_vec(&candidate)?,
                    )?;
                    fs::write(
                        root.join("runtime/last-result.json"),
                        serde_json::to_vec(
                            &serde_json::json!({"activated":candidate,"rollback":false}),
                        )?,
                    )?;
                } else {
                    job.terminate();
                    let _ = child.kill();
                    let _ = child.wait();
                    child = start(&previous.binary, &root, port)?;
                    job = harness_core::process_job::ProcessJob::attach(&child)?;
                    fs::write(
                        root.join("runtime/last-result.json"),
                        b"{\"rollback\":true,\"reason\":\"Candidate startup failed\"}",
                    )?;
                    if !healthy(&mut child, port) {
                        return Err(io::Error::other(
                            "Both candidate and rollback failed health checks",
                        ));
                    }
                }
            } else {
                fs::write(
                    root.join("runtime/last-result.json"),
                    serde_json::to_vec(&serde_json::json!({"rejected":rejected}))?,
                )?;
            }
            fs::rename(&pending, root.join("runtime/last-request.json"))?;
        }
        std::thread::sleep(Duration::from_millis(250));
    }
}
