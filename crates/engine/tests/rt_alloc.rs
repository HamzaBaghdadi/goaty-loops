//! The renderer must not allocate during steady playback: allocation can
//! block the audio thread and cause dropouts. This binary counts heap
//! allocations with its own global allocator.
use goaty_engine::render::Renderer;
use goaty_engine::*;
use std::alloc::{GlobalAlloc, Layout, System};
use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};

struct Counting;
static COUNT: AtomicUsize = AtomicUsize::new(0);
static ON: AtomicBool = AtomicBool::new(false);

unsafe impl GlobalAlloc for Counting {
    unsafe fn alloc(&self, l: Layout) -> *mut u8 {
        if ON.load(Ordering::Relaxed) {
            COUNT.fetch_add(1, Ordering::Relaxed);
        }
        unsafe { System.alloc(l) }
    }
    unsafe fn dealloc(&self, p: *mut u8, l: Layout) {
        unsafe { System.dealloc(p, l) }
    }
    unsafe fn realloc(&self, p: *mut u8, l: Layout, n: usize) -> *mut u8 {
        if ON.load(Ordering::Relaxed) {
            COUNT.fetch_add(1, Ordering::Relaxed);
        }
        unsafe { System.realloc(p, l, n) }
    }
}

#[global_allocator]
static A: Counting = Counting;

#[test]
fn steady_playback_does_not_allocate() {
    // Drums plus an automated FL effect (decoded every block) on one insert.
    let mut p = Project::new_template();
    for s in 0..16 {
        p.patterns[0].toggle_step(s % 4, s as u32);
    }
    p.channels[0].mixer_track = 1;
    let filter = fl_effects_blob(&[1, 1024, 0, 1024, 0, 0]);
    let slot = EffectSlot {
        mix: 1.0,
        fl: Some(FlSlotInfo { internal: "Fruity Filter".into(), slot: 0, data: filter }),
        enabled: true,
        effect: Effect::MultiFilter { cutoff: 20_000.0, resonance: 0.0, low: 1.0, band: 0.0, high: 0.0 },
    };
    p.mixer[1].effects.push(slot);
    let auto = Channel::new("Cutoff", Instrument::Automation(AutomationParams {
        points: vec![AutoPoint { tick: 0, value: 0.2, tension: 0.0 }, AutoPoint { tick: PPQ * 4, value: 0.9, tension: 0.0 }],
        target: Some(AutoTarget::EffectParam { track: 1, slot: 0, param: 0 }),
    }));
    p.channels.push(auto);
    let ch = p.channels.len() - 1;
    p.playlist.clips.push(Clip { track: 0, start: 0, length: PPQ * 64, offset: 0, source: ClipSource::Pattern(0), muted: false, fade_in: 0, fade_out: 0, gain: 1.0 });
    p.playlist.clips.push(Clip { track: 1, start: 0, length: PPQ * 64, offset: 0, source: ClipSource::Audio(ch), muted: false, fade_in: 0, fade_out: 0, gain: 1.0 });

    let shared = Shared::new(p);
    shared.transport.song_mode.store(true, Ordering::Relaxed);
    shared.transport.playing.store(true, Ordering::Relaxed);
    let mut r = Renderer::new(48_000, None);
    let mut buf = vec![0f32; 512 * 2];
    for _ in 0..200 {
        r.process(&shared, &mut buf); // warm-up: first-time buffer growth is allowed
    }
    ON.store(true, Ordering::Relaxed);
    for _ in 0..1000 {
        r.process(&shared, &mut buf);
    }
    ON.store(false, Ordering::Relaxed);
    assert_eq!(COUNT.load(Ordering::Relaxed), 0, "allocations on the audio path");
}

fn fl_effects_blob(v: &[i32]) -> Vec<u8> {
    v.iter().flat_map(|x| x.to_le_bytes()).collect()
}
