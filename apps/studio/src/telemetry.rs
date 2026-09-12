//! CPU consumption measured from the Studio process's OS accounting.
use serde_json::{Value, json};
use std::sync::{Mutex, OnceLock};
use std::time::Instant;

#[cfg(windows)]
fn cpu_seconds() -> Option<f64> {
    #[repr(C)]
    #[derive(Default)]
    struct FileTime {
        low: u32,
        high: u32,
    }
    #[link(name = "kernel32")]
    unsafe extern "system" {
        fn GetCurrentProcess() -> *mut std::ffi::c_void;
        fn GetProcessTimes(
            process: *mut std::ffi::c_void,
            created: *mut FileTime,
            exited: *mut FileTime,
            kernel: *mut FileTime,
            user: *mut FileTime,
        ) -> i32;
    }
    let (mut created, mut exited, mut kernel, mut user) = (
        FileTime::default(),
        FileTime::default(),
        FileTime::default(),
        FileTime::default(),
    );
    // The pseudo-handle needs no close; all output pointers refer to initialized FILETIMEs.
    if unsafe {
        GetProcessTimes(
            GetCurrentProcess(),
            &mut created,
            &mut exited,
            &mut kernel,
            &mut user,
        )
    } == 0
    {
        return None;
    }
    let ticks = |v: FileTime| ((v.high as u64) << 32) | v.low as u64;
    Some((ticks(kernel) + ticks(user)) as f64 / 10_000_000.0)
}
#[cfg(not(windows))]
fn cpu_seconds() -> Option<f64> {
    None
}

struct Sample {
    at: Instant,
    seconds: f64,
    percent: Option<f64>,
}
fn percentage(cpu_delta: f64, elapsed: f64, cores: usize) -> f64 {
    (100.0 * cpu_delta.max(0.0) / elapsed.max(0.001) / cores.max(1) as f64).clamp(0.0, 100.0)
}
pub fn read() -> Value {
    static SAMPLE: OnceLock<Mutex<Option<Sample>>> = OnceLock::new();
    let mut sample = SAMPLE.get_or_init(|| Mutex::new(None)).lock().unwrap();
    let cores = std::thread::available_parallelism().map_or(1, usize::from);
    let Some(seconds) = cpu_seconds() else {
        return json!({"cpu_percent":null,"scope":"studio_process","available":false});
    };
    let now = Instant::now();
    let percent = match sample.as_ref() {
        Some(previous) if now.duration_since(previous.at).as_secs_f64() < 1.0 => previous.percent,
        Some(previous) => {
            let value = percentage(
                seconds - previous.seconds,
                now.duration_since(previous.at).as_secs_f64(),
                cores,
            );
            *sample = Some(Sample {
                at: now,
                seconds,
                percent: Some(value),
            });
            Some(value)
        }
        None => {
            *sample = Some(Sample {
                at: now,
                seconds,
                percent: None,
            });
            None
        }
    };
    json!({"cpu_percent":percent,"scope":"studio_process","available":true,"logical_cpus":cores})
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn cpu_is_normalized_by_elapsed_time_and_machine_capacity() {
        assert_eq!(percentage(2.0, 2.0, 8), 12.5);
        assert_eq!(percentage(16.0, 2.0, 8), 100.0);
        assert_eq!(percentage(-1.0, 2.0, 8), 0.0);
        assert_eq!(percentage(32.0, 2.0, 8), 100.0);
    }
}
