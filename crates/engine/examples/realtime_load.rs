//! Plays an FLP through the real audio device (no UI) and reports how much CPU
//! the whole process uses versus how long the audio callback actually works.
//! A large gap means threads are busy-waiting rather than rendering.
//!
//! cargo run --release -p goaty-engine --example realtime_load -- song.flp [seconds] [--stopped]
//!     [--driver shared|low|exclusive] [--buffer frames] [--rate hz] [--device name] [--mute] [--switch-rate hz]
use goaty_engine::audio::{AudioConfig, AudioEngine, Driver};
use goaty_engine::import_flp::import_flp;
use goaty_engine::plugin::PluginDb;
use std::sync::atomic::Ordering;
use std::time::{Duration, Instant};

fn main() -> anyhow::Result<()> {
    let (mut positional, mut config, mut stopped, mut mute) = (Vec::new(), AudioConfig::default(), false, false);
    let mut switch_rate: Option<u32> = None;
    let mut args = std::env::args().skip(1);
    while let Some(a) = args.next() {
        match a.as_str() {
            "--stopped" => stopped = true,
            "--mute" => mute = true,
            "--driver" => {
                config.driver = match args.next().as_deref() {
                    Some("low") => Driver::LowLatency,
                    Some("exclusive") => Driver::Exclusive,
                    _ => Driver::Shared,
                }
            }
            "--buffer" => config.buffer_frames = args.next().and_then(|s| s.parse().ok()),
            "--rate" => config.sample_rate = args.next().and_then(|s| s.parse().ok()),
            "--device" => config.device = args.next(),
            "--switch-rate" => switch_rate = args.next().and_then(|s| s.parse().ok()),
            _ => positional.push(a),
        }
    }
    let path = positional.first().expect("usage: realtime_load song.flp [seconds] [options]");
    let secs: u64 = positional.get(1).and_then(|s| s.parse().ok()).unwrap_or(10);
    let mut project = import_flp(path.as_ref(), &PluginDb::load())?.project;
    if mute {
        project.mixer[0].muted = true; // everything still renders, nothing is heard
    }
    let mut engine = AudioEngine::new(project.clone(), &config);
    if let Some(e) = &engine.error {
        anyhow::bail!("audio device: {e}");
    }
    for e in project.instantiate_plugins(engine.sample_rate as f64) {
        eprintln!("plugin: {e}");
    }
    engine.publish(&project);
    let i = &engine.info;
    println!(
        "device: {} @ {} Hz via {}, buffer {}, output latency {}, {} mixer tracks",
        engine.device_name,
        engine.sample_rate,
        i.driver.label(),
        i.buffer_frames.map_or("system".into(), |b| format!("{b} frames")),
        engine.output_latency_ms().map_or("?".into(), |ms| format!("{ms:.1} ms")),
        project.mixer.len()
    );

    engine.transport().song_mode.store(true, Ordering::Relaxed);
    engine.transport().playing.store(!stopped, Ordering::Relaxed);
    std::thread::sleep(Duration::from_secs(2)); // settle
    if let Some(rate) = switch_rate {
        // What the app does when the sample rate changes: reopen, then reload
        // every plugin (with its settings) at the new rate.
        let t0 = Instant::now();
        engine.reopen(&AudioConfig { driver: Driver::Exclusive, sample_rate: Some(rate), ..config.clone() }).map_err(anyhow::Error::msg)?;
        project.unload_plugins();
        engine.publish(&project);
        let errors = project.instantiate_plugins(engine.sample_rate as f64);
        engine.publish(&project);
        println!("switched to {} Hz and reloaded plugins in {:.2}s ({} errors)", engine.sample_rate, t0.elapsed().as_secs_f32(), errors.len());
        std::thread::sleep(Duration::from_secs(1));
    }

    let (cpu0, wall0) = (process_cpu(), Instant::now());
    let (mut sum, mut max, mut n) = (0f32, 0f32, 0u32);
    while wall0.elapsed() < Duration::from_secs(secs) {
        std::thread::sleep(Duration::from_millis(20));
        engine.shared.collect_garbage(); // as the UI does every frame
        let l = engine.shared.cpu_load();
        sum += l;
        max = max.max(l);
        n += 1;
    }
    let cpu = (process_cpu() - cpu0).as_secs_f64() / wall0.elapsed().as_secs_f64();
    engine.transport().playing.store(false, Ordering::Relaxed);
    println!("audio callback load: avg {:.1}%, max {:.1}% of the buffer period", 100.0 * sum / n.max(1) as f32, 100.0 * max);
    println!("process CPU: {:.0}% of one core ({:.1} cores busy)", cpu * 100.0, cpu);
    Ok(())
}

/// Total user + kernel CPU time of this process.
#[cfg(windows)]
fn process_cpu() -> Duration {
    #[repr(C)]
    #[derive(Default)]
    struct FileTime(u32, u32);
    extern "system" {
        fn GetCurrentProcess() -> isize;
        fn GetProcessTimes(p: isize, c: *mut FileTime, e: *mut FileTime, k: *mut FileTime, u: *mut FileTime) -> i32;
    }
    let (mut c, mut e, mut k, mut u) = Default::default();
    unsafe { GetProcessTimes(GetCurrentProcess(), &mut c, &mut e, &mut k, &mut u) };
    let ticks = |f: &FileTime| (f.1 as u64) << 32 | f.0 as u64;
    Duration::from_nanos((ticks(&k) + ticks(&u)) * 100)
}

#[cfg(not(windows))]
fn process_cpu() -> Duration {
    Duration::ZERO
}
