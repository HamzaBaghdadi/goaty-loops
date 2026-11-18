//! Lists output devices and what each driver offers, then switches drivers
//! at runtime the way the Audio settings window does (silent: nothing plays).
//!
//! cargo run --release -p goaty-engine --example audio_devices
use goaty_engine::audio::{driver_caps, output_devices, AudioConfig, AudioEngine, Driver};
use goaty_engine::Project;

fn main() {
    for &d in Driver::all() {
        println!("{}:", d.label());
        for name in output_devices(d) {
            println!("  device: {name}");
        }
        match driver_caps(d, None, None) {
            Ok(c) => println!("  default device: rates {:?}, buffers {:?}, default buffer {:?}", c.sample_rates, c.buffer_frames, c.default_buffer),
            Err(e) => println!("  default device: {e}"),
        }
    }

    let mut engine = AudioEngine::new(Project::new_template(), &AudioConfig::default());
    let show = |e: &AudioEngine, what: &str| {
        println!(
            "{what}: {} @ {} Hz via {}, buffer {:?}, latency {:?} ms, error {:?}",
            e.device_name,
            e.sample_rate,
            e.info.driver.label(),
            e.info.buffer_frames,
            e.output_latency_ms(),
            e.error
        )
    };
    show(&engine, "start");
    for config in [
        AudioConfig { driver: Driver::Exclusive, buffer_frames: Some(128), ..Default::default() },
        AudioConfig { driver: Driver::Exclusive, sample_rate: Some(96000), buffer_frames: Some(256), ..Default::default() },
        AudioConfig { driver: Driver::LowLatency, ..Default::default() },
        AudioConfig { driver: Driver::Exclusive, device: Some("No such device".into()), ..Default::default() },
        AudioConfig::default(),
    ] {
        let r = engine.reopen(&config);
        std::thread::sleep(std::time::Duration::from_millis(300));
        show(&engine, &format!("{:?} {:?} {:?} -> {}", config.driver, config.sample_rate, config.device, if r.is_ok() { "ok" } else { "failed" }));
        if engine.info.driver == Driver::Exclusive {
            // The settings window queries the device while we hold it.
            match driver_caps(Driver::Exclusive, None, Some(engine.sample_rate)) {
                Ok(c) => println!("   caps while exclusive: rates {:?}, buffers {:?}", c.sample_rates, c.buffer_frames),
                Err(e) => println!("   caps while exclusive: {e}"),
            }
        }
    }
}
