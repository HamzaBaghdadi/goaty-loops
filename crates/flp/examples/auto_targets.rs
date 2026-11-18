//! Classifies automation link targets across FLPs (read-only).
use goaty_flp::{FlpControlTarget, FlpFile, FlpProject};
use std::collections::{BTreeMap, HashSet};
fn main() {
    let mut kinds: BTreeMap<String, usize> = BTreeMap::new();
    let (mut autos, mut linked) = (0, 0);
    let mut examples: BTreeMap<String, Vec<String>> = BTreeMap::new();
    for path in std::env::args().skip(1) {
        let Ok(f) = FlpFile::read(&path) else { continue };
        let p = FlpProject::from_file(&f);
        let auto_iids: HashSet<u16> = p.channels.iter().filter(|c| c.kind == 5).map(|c| c.iid).collect();
        autos += auto_iids.len();
        let mut seen = HashSet::new();
        for c in &p.controllers {
            if !auto_iids.contains(&c.source_channel) { continue; }
            seen.insert(c.source_channel);
            let key = match c.target {
                FlpControlTarget::Channel { param, .. } => format!("channel param {param}"),
                FlpControlTarget::ChannelPlugin { iid, .. } => {
                    let plug = p.channels.iter().find(|c| c.iid == iid).and_then(|c| c.plugin.clone()).unwrap_or_default();
                    format!("channel plugin: {plug}")
                }
                FlpControlTarget::Mixer { slot, param, .. } => format!("mixer param {param} (slot field {slot})"),
                FlpControlTarget::Global { param } => format!("global param {param}"),
                FlpControlTarget::MixerPlugin { insert, slot, .. } => {
                    let s = p.inserts.get(insert as usize).and_then(|i| i.slots.iter().find(|s| s.index as u16 == slot));
                    format!("mixer fx: {}", s.map(|s| s.internal.clone()).unwrap_or("?".into()))
                }
            };
            let name = p.channels.iter().find(|x| x.iid == c.source_channel).and_then(|x| x.name.clone()).unwrap_or_default();
            if key.starts_with("mixer param") || key.starts_with("channel param") { let e = examples.entry(key.clone()).or_default(); if e.len() < 4 { e.push(format!("{name} [{:?}]", c.target)); } }
            *kinds.entry(key).or_default() += 1;
        }
        linked += seen.len();
    }
    println!("automation channels {autos}, with a link {linked}");
    let mut v: Vec<_> = kinds.into_iter().collect();
    v.sort_by(|a, b| b.1.cmp(&a.1));
    for (k, n) in v.iter().take(30) { println!("{n:>5}  {k}"); }
}
