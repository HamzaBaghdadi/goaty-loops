//! Piano roll editing tools: quantize, chop, glue, legato, velocity.
//!
//! Each tool works on the notes at `idx` and returns the indices of the
//! resulting notes, so the selection can follow the edit.

use goaty_engine::Note;

fn round_to(t: u32, grid: u32) -> u32 {
    let g = grid.max(1);
    (t + g / 2) / g * g
}

/// Moves note starts toward the grid by `strength` (0..=1); with
/// `lengths`, note ends snap too (never shorter than one grid step).
pub fn quantize(notes: &mut [Note], idx: &[usize], grid: u32, strength: f32, lengths: bool) {
    for &i in idx {
        let n = &mut notes[i];
        let target = round_to(n.start, grid) as f32;
        n.start = (n.start as f32 + (target - n.start as f32) * strength.clamp(0.0, 1.0)).round() as u32;
        if lengths && n.length > 0 {
            n.length = round_to(n.length, grid).max(grid.max(1));
        }
    }
}

/// Splits each note into pieces of `grid` ticks.
pub fn chop(notes: &mut Vec<Note>, idx: &[usize], grid: u32) -> Vec<usize> {
    let grid = grid.max(1);
    let mut pieces = Vec::new();
    for &i in idx {
        let n = notes[i];
        let end = n.end();
        let mut t = n.start;
        while t < end {
            let len = grid.min(end - t);
            pieces.push(Note { start: t, length: len, ..n });
            t += len;
        }
    }
    replace(notes, idx, pieces)
}

/// Merges touching or overlapping notes of the same channel and key.
pub fn glue(notes: &mut Vec<Note>, idx: &[usize]) -> Vec<usize> {
    let mut sel: Vec<Note> = idx.iter().map(|&i| notes[i]).collect();
    sel.sort_by_key(|n| (n.channel, n.key, n.start));
    let mut merged: Vec<Note> = Vec::new();
    for n in sel {
        match merged.last_mut() {
            Some(m) if m.channel == n.channel && m.key == n.key && n.start <= m.end() => {
                m.length = m.end().max(n.end()) - m.start;
            }
            _ => merged.push(Note { length: n.end() - n.start, ..n }),
        }
    }
    replace(notes, idx, merged)
}

/// Extends each note to the start of the next later note (per channel).
pub fn legato(notes: &mut [Note], idx: &[usize]) {
    let mut starts: Vec<(usize, u32)> = idx.iter().map(|&i| (notes[i].channel, notes[i].start)).collect();
    starts.sort_unstable();
    starts.dedup();
    for &i in idx {
        let n = &mut notes[i];
        let next = starts.iter().find(|&&(c, s)| c == n.channel && s > n.start);
        if let Some(&(_, s)) = next {
            n.length = s - n.start;
        }
    }
}

/// Multiplies velocities by `factor`.
pub fn scale_velocity(notes: &mut [Note], idx: &[usize], factor: f32) {
    for &i in idx {
        notes[i].velocity = (notes[i].velocity * factor).clamp(0.0, 1.0);
    }
}

/// Removes the notes at `idx`, appends `new`, and returns the new indices.
fn replace(notes: &mut Vec<Note>, idx: &[usize], new: Vec<Note>) -> Vec<usize> {
    let mut drop = vec![false; notes.len()];
    for &i in idx {
        drop[i] = true;
    }
    let mut k = 0;
    notes.retain(|_| {
        k += 1;
        !drop[k - 1]
    });
    let first = notes.len();
    notes.extend(new);
    (first..notes.len()).collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn note(key: u8, start: u32, length: u32) -> Note {
        Note { channel: 0, key, start, length, velocity: 0.8, pan: 0.0 }
    }

    #[test]
    fn quantize_strength_and_lengths() {
        let mut n = vec![note(60, 10, 20), note(60, 90, 50)];
        quantize(&mut n, &[0, 1], 24, 1.0, true);
        assert_eq!((n[0].start, n[0].length), (0, 24));
        assert_eq!((n[1].start, n[1].length), (96, 48));
        let mut n = vec![note(60, 10, 20)];
        quantize(&mut n, &[0], 24, 0.5, false);
        assert_eq!((n[0].start, n[0].length), (5, 20));
    }

    #[test]
    fn chop_and_glue_round_trip() {
        let mut n = vec![note(60, 0, 50), note(64, 0, 10)];
        let pieces = chop(&mut n, &[0], 24);
        assert_eq!(pieces.len(), 3);
        assert_eq!(n.iter().filter(|x| x.key == 60).map(|x| x.length).collect::<Vec<_>>(), [24, 24, 2]);
        let glued = glue(&mut n, &pieces);
        assert_eq!(glued.len(), 1);
        assert_eq!((n[glued[0]].start, n[glued[0]].length), (0, 50));
        assert_eq!(n.len(), 2);
    }

    #[test]
    fn glue_keeps_gaps_and_keys_apart() {
        let mut n = vec![note(60, 0, 24), note(60, 48, 24), note(62, 24, 24)];
        assert_eq!(glue(&mut n, &[0, 1, 2]).len(), 3);
    }

    #[test]
    fn legato_fills_gaps() {
        let mut n = vec![note(60, 0, 10), note(64, 0, 10), note(67, 48, 10)];
        legato(&mut n, &[0, 1, 2]);
        assert_eq!([n[0].length, n[1].length, n[2].length], [48, 48, 10]);
    }
}
