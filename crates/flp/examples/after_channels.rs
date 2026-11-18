//! For each file: does the last channel block look like an earlier block of
//! the same type, and which event id follows it? Read-only.
use goaty_flp::{ids, FlpFile};
fn main() {
    for path in std::env::args().skip(1) {
        let Ok(f) = FlpFile::read(&path) else { continue };
        let ev = &f.events;
        let starts: Vec<usize> = ev.iter().enumerate().filter(|(_, e)| e.id == ids::CH_NEW).map(|(i, _)| i).collect();
        let Some(&last) = starts.last() else { continue };
        let ty = |s: usize, e: usize| ev[s..e].iter().find(|x| x.id == ids::CH_TYPE).and_then(|x| x.as_u32());
        // Template: an earlier block of the same type.
        let last_ty = ty(last, (last + 60).min(ev.len()));
        let tmpl = starts.windows(2).find(|w| ty(w[0], w[1]) == last_ty).map(|w| w[1] - w[0]);
        let name = path.rsplit(['/', '\\']).next().unwrap_or(&path).to_string();
        match tmpl {
            Some(len) => {
                let same = (0..len).all(|k| ev.get(last + k).map(|e| e.id) == ev.get(starts[starts.iter().position(|&s| ty(s, s + 60) == last_ty).unwrap()] + k).map(|e| e.id));
                let next = ev.get(last + len).map(|e| e.id);
                println!("same_ids={same} next={next:?} type={last_ty:?} [{name}]");
            }
            None => println!("no template for type {last_ty:?} [{name}]"),
        }
    }
}
