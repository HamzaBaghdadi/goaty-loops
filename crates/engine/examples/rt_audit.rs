//! Real-time audit: counts heap allocations made by the renderer during
//! steady playback (after a warm-up). Pass an FLP to audit a real song, or
//! nothing for the default template. Reads files only.
use goaty_engine::import_flp::import_flp;
use goaty_engine::plugin::PluginDb;
use goaty_engine::render::Renderer;
use goaty_engine::*;
use std::alloc::{GlobalAlloc, Layout, System};
use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};

struct Counting;
static COUNT: AtomicUsize = AtomicUsize::new(0);
static ON: AtomicBool = AtomicBool::new(false);
static TRACE: AtomicBool = AtomicBool::new(false);

thread_local! { static INSIDE: std::cell::Cell<bool> = const { std::cell::Cell::new(false) }; }

/// With RT_TRACE set, prints where the first few counted allocations come from.
fn trace() {
    if !TRACE.load(Ordering::Relaxed) || COUNT.load(Ordering::Relaxed) > 6 || INSIDE.with(|i| i.replace(true)) {
        return;
    }
    let bt = std::backtrace::Backtrace::force_capture().to_string();
    let ours: Vec<&str> = bt.lines().filter(|l| l.contains("goaty_") && !l.contains("rt_audit")).take(4).collect();
    eprintln!("--- allocation:
{}", ours.join("
"));
    INSIDE.with(|i| i.set(false));
}

unsafe impl GlobalAlloc for Counting {
    unsafe fn alloc(&self, l: Layout) -> *mut u8 {
        if ON.load(Ordering::Relaxed) {
            COUNT.fetch_add(1, Ordering::Relaxed);
            trace();
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

fn main() -> anyhow::Result<()> {
    TRACE.store(std::env::var_os("RT_TRACE").is_some(), Ordering::Relaxed);
    let project = match std::env::args().nth(1) {
        Some(path) => {
            let mut p = import_flp(path.as_ref(), &PluginDb::load())?.project;
            if std::env::var_os("RT_NO_PLUGINS").is_none() {
                p.instantiate_plugins(48000.0);
            }
            p
        }
        None => {
            let mut p = Project::new_template();
            for s in 0..16 {
                p.patterns[0].toggle_step(s % 4, s as u32);
            }
            p
        }
    };
    let shared = Shared::new(project);
    shared.transport.song_mode.store(!shared.project.load().playlist.clips.is_empty(), Ordering::Relaxed);
    shared.transport.playing.store(true, Ordering::Relaxed);
    let mut r = Renderer::new(48000, None);
    let mut buf = vec![0f32; 512 * 2];
    for _ in 0..400 {
        r.process(&shared, &mut buf); // warm-up: first-time buffer growth is allowed
    }
    let blocks = 2000;
    ON.store(true, Ordering::Relaxed);
    for _ in 0..blocks {
        r.process(&shared, &mut buf);
    }
    ON.store(false, Ordering::Relaxed);
    let n = COUNT.load(Ordering::Relaxed);
    println!("{n} allocations in {blocks} blocks ({:.2} per block)", n as f64 / blocks as f64);
    Ok(())
}
