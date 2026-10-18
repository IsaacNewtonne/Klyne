//! One local speech worker, started on demand, with bounded private stdio.
use crate::err;
use harness_core::process_job::ProcessJob;
use serde_json::{Value, json};
use std::{
    io::{self, BufRead, BufReader, Read, Write},
    path::PathBuf,
    process::{Child, ChildStdin, ChildStdout, Command, Stdio},
    sync::{Arc, Mutex, OnceLock, mpsc},
    time::Duration,
};

struct Worker {
    child: Mutex<Child>,
    pipes: Mutex<(ChildStdin, BufReader<ChildStdout>)>,
    _lifetime: mpsc::Sender<()>,
}
impl Worker {
    fn stop(&self) {
        let mut child = self.child.lock().unwrap();
        let _ = child.kill();
        let _ = child.wait();
    }
}
impl Drop for Worker {
    fn drop(&mut self) {
        self.stop();
    }
}
#[derive(Default)]
struct State {
    worker: Option<Arc<Worker>>,
    active: Option<String>,
}
fn state() -> &'static Mutex<State> {
    static STATE: OnceLock<Mutex<State>> = OnceLock::new();
    STATE.get_or_init(|| Mutex::new(State::default()))
}
fn home() -> PathBuf {
    std::env::var_os("KLYNE_VOICE_HOME")
        .map(PathBuf::from)
        .unwrap_or_else(|| PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../.voice"))
}
fn python() -> PathBuf {
    home().join(if cfg!(windows) {
        "runtime/Scripts/python.exe"
    } else {
        "runtime/bin/python"
    })
}
pub fn status() -> Value {
    let state = state().lock().unwrap();
    json!({"installed":installed(),"busy":state.active.is_some(),"loaded":state.worker.is_some(),"local":true,"asr":"Moonshine Tiny Streaming 34M (English; Vietnamese preview)","tts":"Kokoro 82M / George (English)","setup":"powershell -ExecutionPolicy Bypass -File scripts/setup-voice.ps1"})
}
fn installed() -> bool {
    python().is_file()
        && std::fs::read(home().join("ready.json"))
            .ok()
            .and_then(|data| serde_json::from_slice::<Value>(&data).ok())
            .is_some_and(|value| value["version"] == 3)
}
fn start() -> io::Result<Arc<Worker>> {
    if !installed() {
        return Err(err(
            "Local voice is not installed. Run scripts/setup-voice.ps1 in the Klyne folder.",
        ));
    }
    crate::safe_dir(&home())?;
    let mut command = Command::new(python());
    // Stage the embedded worker, so source updates and runtime upgrades agree.
    let script = home().join("worker.py");
    if script.exists() && std::fs::symlink_metadata(&script)?.file_type().is_symlink() {
        return Err(err("Invalid voice worker path"));
    }
    std::fs::write(&script, include_bytes!("../../../scripts/voice-worker.py"))?;
    command
        .arg("-I")
        .arg("-u")
        .arg(&script)
        .arg("--home")
        .arg(home())
        .env_clear()
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::null());
    for key in [
        "SystemRoot",
        "WINDIR",
        "PATH",
        "TEMP",
        "TMP",
        "USERPROFILE",
        "LOCALAPPDATA",
    ] {
        if let Some(value) = std::env::var_os(key) {
            command.env(key, value);
        }
    }
    command
        .env("HF_HUB_OFFLINE", "1")
        .env("TRANSFORMERS_OFFLINE", "1")
        .env("HF_HUB_DISABLE_TELEMETRY", "1");
    #[cfg(windows)]
    {
        use std::os::windows::process::CommandExt;
        command.creation_flags(0x08000000);
    }
    let (ready_tx, ready_rx) = mpsc::channel();
    let (life_tx, life_rx) = mpsc::channel();
    // The job object stays on its owning thread; no unsafe Send/Sync handle wrapper.
    std::thread::spawn(move || {
        let started = (|| -> io::Result<(Child, ProcessJob)> {
            let mut child = command.spawn()?;
            match ProcessJob::attach(&child) {
                Ok(job) => Ok((child, job)),
                Err(error) => {
                    let _ = child.kill();
                    let _ = child.wait();
                    Err(error)
                }
            }
        })();
        match started {
            Ok((mut child, job)) => {
                let input = child.stdin.take().unwrap();
                let output = child.stdout.take().unwrap();
                let _ = ready_tx.send(Ok((child, input, output)));
                let _ = life_rx.recv();
                job.terminate();
            }
            Err(error) => {
                let _ = ready_tx.send(Err(error));
            }
        }
    });
    let (child, input, output) = ready_rx
        .recv()
        .map_err(|_| err("Could not start voice worker"))??;
    Ok(Arc::new(Worker {
        child: Mutex::new(child),
        pipes: Mutex::new((input, BufReader::new(output))),
        _lifetime: life_tx,
    }))
}
fn id(body: &Value) -> io::Result<&str> {
    body["request_id"]
        .as_str()
        .filter(|s| {
            !s.is_empty()
                && s.len() <= 80
                && s.bytes().all(|c| c.is_ascii_alphanumeric() || c == b'-')
        })
        .ok_or_else(|| err("Invalid voice request ID"))
}
pub fn validate(body: &Value) -> io::Result<()> {
    id(body)?;
    match body["operation"].as_str() {
        Some("health") => Ok(()),
        Some("warmup")
            if matches!(body["language"].as_str(), Some("en" | "vi"))
                && body["tts"].is_boolean() =>
        {
            Ok(())
        }
        Some("transcribe")
            if body["audio"]
                .as_str()
                .is_some_and(|s| !s.is_empty() && s.len() <= 1_280_060)
                && matches!(body["language"].as_str(), Some("en" | "vi")) =>
        {
            Ok(())
        }
        Some("speak")
            if body["text"]
                .as_str()
                .is_some_and(|s| !s.trim().is_empty() && s.chars().count() <= 450) =>
        {
            Ok(())
        }
        _ => Err(err("Invalid voice operation, recording or speech length")),
    }
}
pub fn cancel(body: &Value) -> io::Result<Value> {
    let id = id(body)?;
    let state = state().lock().unwrap();
    let cancelled = state.active.as_deref() == Some(id);
    if cancelled && let Some(worker) = &state.worker {
        worker.stop();
    }
    Ok(json!({"cancelled":cancelled}))
}
pub fn request(body: &Value) -> io::Result<Value> {
    validate(body)?;
    let (worker, id) = {
        let mut state = state().lock().unwrap();
        if state.active.is_some() {
            return Err(err(
                "Voice is busy. Wait for the current audio operation or stop it.",
            ));
        }
        if let Some(worker) = &state.worker {
            let dead = worker.child.lock().unwrap().try_wait()?.is_some();
            if dead {
                state.worker = None;
            }
        }
        if state.worker.is_none() {
            state.worker = Some(start()?);
        }
        let id = id(body)?.to_string();
        state.active = Some(id.clone());
        (state.worker.as_ref().unwrap().clone(), id)
    };
    let input = body.to_string();
    let (sender, receiver) = mpsc::channel();
    let task_worker = worker.clone();
    std::thread::spawn(move || {
        let result = (|| -> io::Result<Value> {
            let mut pipes = task_worker.pipes.lock().unwrap();
            writeln!(pipes.0, "{input}")?;
            pipes.0.flush()?;
            let mut line = String::new();
            (&mut pipes.1).take(4_500_001).read_line(&mut line)?;
            if line.len() > 4_500_000 || !line.ends_with('\n') {
                return Err(err("Voice worker stopped or exceeded its output limit"));
            }
            let value: Value =
                serde_json::from_str(&line).map_err(|_| err("Invalid voice worker response"))?;
            if let Some(message) = value["error"].as_str() {
                return Err(err(message));
            }
            Ok(value)
        })();
        let _ = sender.send(result);
    });
    let result = receiver
        .recv_timeout(Duration::from_secs(120))
        .unwrap_or_else(|_| {
            Err(err(
                "Voice operation exceeded two minutes; stopped. Retry to restart the local worker.",
            ))
        });
    let mut state = state().lock().unwrap();
    if state.active.as_deref() == Some(&id) {
        if result.is_err() {
            worker.stop();
            state.worker = None;
        }
        state.active = None;
    }
    result
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn voice_protocol_is_bounded_and_never_accepts_paths_or_commands() {
        assert!(
            validate(&json!({"request_id":"abc-1","operation":"speak","text":"Hello"})).is_ok()
        );
        assert!(
            validate(&json!({"request_id":"abc-1","operation":"speak","text":"x".repeat(451)}))
                .is_err()
        );
        assert!(
            validate(&json!({"request_id":"abc-1","operation":"shell","command":"echo hello"}))
                .is_err()
        );
        assert!(validate(&json!({"request_id":"../x","operation":"health"})).is_err());
        assert_eq!(
            cancel(&json!({"request_id":"unrelated"})).unwrap()["cancelled"],
            false
        );
    }
}
