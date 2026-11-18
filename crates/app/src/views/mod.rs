pub mod audio_settings;
pub mod automation;
pub mod browser;
pub mod channel_rack;
pub mod channel_settings;
pub mod mixer;
pub mod note_tools;
pub mod piano_roll;
pub mod playlist;

/// Snap choices shared by the piano roll and playlist (label, ticks).
pub const SNAPS: [(&str, u32); 8] = [
    ("None", 1),
    ("1/6 step", 4),
    ("1/4 step", 6),
    ("1/2 step", 12),
    ("Step", 24),
    ("1/2 beat", 48),
    ("Beat", 96),
    ("Bar", 384),
];

pub fn snap_name(ticks: u32) -> &'static str {
    SNAPS.iter().find(|s| s.1 == ticks).map(|s| s.0).unwrap_or("Custom")
}

pub fn snap_floor(t: f32, snap: u32) -> u32 {
    let s = snap.max(1) as f32;
    ((t.max(0.0) / s).floor() * s) as u32
}

pub fn snap_round(t: f32, snap: u32) -> u32 {
    let s = snap.max(1) as f32;
    ((t.max(0.0) / s).round() * s) as u32
}

pub fn snap_combo(ui: &mut egui::Ui, id: &str, snap: &mut u32) {
    egui::ComboBox::from_id_salt(id).width(80.0).selected_text(snap_name(*snap)).show_ui(ui, |ui| {
        for (name, t) in SNAPS {
            ui.selectable_value(snap, t, name);
        }
    });
}

/// Human-readable description of an automation target.
pub fn describe_target(p: &goaty_engine::Project, t: Option<goaty_engine::AutoTarget>) -> String {
    use goaty_engine::AutoTarget::*;
    let ch = |i: usize| p.channels.get(i).map(|c| c.name.clone()).unwrap_or_else(|| format!("channel {i}"));
    let tr = |i: usize| p.mixer.get(i).map(|m| m.name.clone()).unwrap_or_else(|| format!("insert {i}"));
    match t {
        None => "Not linked (target not supported yet)".into(),
        Some(ChannelVolume(c)) => format!("{} - Volume", ch(c)),
        Some(ChannelPan(c)) => format!("{} - Panning", ch(c)),
        Some(ChannelPitch(c)) => format!("{} - Pitch", ch(c)),
        Some(MixerVolume(m)) => format!("Mixer {} - Volume", tr(m)),
        Some(MixerPan(m)) => format!("Mixer {} - Panning", tr(m)),
        Some(Tempo) => "Tempo".into(),
        Some(MainVolume) => "Main volume".into(),
        Some(t @ EffectParam { track, slot, param }) => {
            let fx = p.mixer.get(track).and_then(|m| m.slot_by_id(slot));
            let fx_name = fx.map(|e| match &e.effect {
                goaty_engine::Effect::Plugin(pp) => pp.info.name.clone(),
                e => e.name().to_string(),
            });
            format!("{} - {} - {}", tr(track), fx_name.unwrap_or_else(|| format!("slot {}", slot + 1)), param_label(p, t, param))
        }
        Some(t @ PluginParam { channel, param }) => format!("{} - {}", ch(channel), param_label(p, t, param)),
        Some(SlotMix { track, slot }) => format!("{} - slot {} mix", tr(track), slot + 1),
        Some(SlotEnabled { track, slot }) => format!("{} - slot {} on/off", tr(track), slot + 1),
    }
}

/// A plugin parameter's own name when the plugin is loaded.
fn param_label(p: &goaty_engine::Project, t: goaty_engine::AutoTarget, param: u32) -> String {
    p.plugin_param(t).and_then(|(inst, i)| inst.param_name(i)).filter(|n| !n.is_empty()).unwrap_or_else(|| format!("parameter {param}"))
}

/// Paints an automation curve scaled into `rect` over `len` ticks.
pub fn paint_auto_curve(p: &egui::Painter, rect: egui::Rect, a: &goaty_engine::AutomationParams, start_tick: f32, len: f32, color: egui::Color32) {
    if a.points.is_empty() || rect.width() < 2.0 {
        return;
    }
    let n = (rect.width() as usize).clamp(2, 1200);
    let pts: Vec<egui::Pos2> = (0..=n)
        .map(|i| {
            let x = rect.left() + rect.width() * i as f32 / n as f32;
            let tick = start_tick + len * i as f32 / n as f32;
            let v = a.value_at(tick).unwrap_or(0.0);
            egui::pos2(x, rect.bottom() - v * rect.height())
        })
        .collect();
    p.add(egui::Shape::line(pts, egui::Stroke::new(1.5_f32, color)));
}
