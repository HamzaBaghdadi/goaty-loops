//! App icon: resizes `assets/icon.png` for the window and, on Windows, writes
//! it into a COFF resource object linked into the exe (Explorer, taskbar and
//! shortcuts read the icon from there). Written directly, so no `windres` or
//! `rc.exe` is needed.

use image::imageops::FilterType;
use image::RgbaImage;
use std::path::Path;

/// Icon sizes in the exe. Small ones are stored as bitmaps, large as PNG.
const SIZES: &[u32] = &[256, 128, 64, 48, 40, 32, 24, 20, 16];

pub fn generate(source: &Path, out_dir: &Path, windows_machine: Option<u16>) -> Result<(), String> {
    let img = image::open(source).map_err(|e| format!("{}: {e}", source.display()))?.to_rgba8();
    let resize = |s: u32| image::imageops::resize(&img, s, s, FilterType::Lanczos3);

    // Window/taskbar icon used at runtime.
    resize(256).save(out_dir.join("icon_256.png")).map_err(|e| e.to_string())?;

    if let Some(machine) = windows_machine {
        let images: Vec<Vec<u8>> = SIZES.iter().map(|&s| if s >= 128 { png(&resize(s)) } else { dib(&resize(s)) }).collect();
        let obj = resource_object(machine, &images);
        let path = out_dir.join("icon_resource.o");
        std::fs::write(&path, obj).map_err(|e| e.to_string())?;
        println!("cargo:rustc-link-arg-bins={}", path.display());
    }
    Ok(())
}

fn png(img: &RgbaImage) -> Vec<u8> {
    let mut v = Vec::new();
    image::DynamicImage::ImageRgba8(img.clone())
        .write_to(&mut std::io::Cursor::new(&mut v), image::ImageFormat::Png)
        .expect("PNG encode");
    v
}

/// An icon image as a DIB: BITMAPINFOHEADER (double height), bottom-up BGRA,
/// then an all-zero AND mask (transparency comes from alpha).
fn dib(img: &RgbaImage) -> Vec<u8> {
    let (w, h) = img.dimensions();
    let mask_row = w.div_ceil(32) * 4;
    let mut v = Vec::new();
    for x in [40u32, w, h * 2] {
        v.extend(x.to_le_bytes());
    }
    v.extend(1u16.to_le_bytes()); // planes
    v.extend(32u16.to_le_bytes()); // bits per pixel
    v.extend(0u32.to_le_bytes()); // BI_RGB
    v.extend((w * h * 4 + mask_row * h).to_le_bytes());
    v.extend([0u8; 16]); // resolution, palette
    for y in (0..h).rev() {
        for x in 0..w {
            let [r, g, b, a] = img.get_pixel(x, y).0;
            v.extend([b, g, r, a]);
        }
    }
    v.resize(v.len() + (mask_row * h) as usize, 0);
    v
}

/// GRPICONDIR for RT_GROUP_ICON: one entry per image, pointing at RT_ICON ids 1..
fn group(images: &[Vec<u8>]) -> Vec<u8> {
    let mut v = Vec::new();
    v.extend(0u16.to_le_bytes());
    v.extend(1u16.to_le_bytes()); // icon
    v.extend((images.len() as u16).to_le_bytes());
    for (i, (&s, data)) in SIZES.iter().zip(images).enumerate() {
        let b = if s >= 256 { 0 } else { s as u8 };
        v.extend([b, b, 0, 0]);
        v.extend(1u16.to_le_bytes()); // planes
        v.extend(32u16.to_le_bytes()); // bits per pixel
        v.extend((data.len() as u32).to_le_bytes());
        v.extend((i as u16 + 1).to_le_bytes());
    }
    v
}

const RT_ICON: u32 = 3;
const RT_GROUP_ICON: u32 = 14;
const LANG_NEUTRAL: u32 = 0;
const SUBDIR: u32 = 0x8000_0000;

/// A COFF object with a single `.rsrc` section holding the icon images
/// (RT_ICON 1..n) and their group (RT_GROUP_ICON 1), as `cvtres`/`windres` make.
fn resource_object(machine: u16, images: &[Vec<u8>]) -> Vec<u8> {
    let group = group(images);
    // Every resource: (type, id, data).
    let mut res: Vec<(u32, u32, &[u8])> = images.iter().enumerate().map(|(i, d)| (RT_ICON, i as u32 + 1, d.as_slice())).collect();
    res.push((RT_GROUP_ICON, 1, &group));

    let dir = |entries: usize| 16 + 8 * entries as u32;
    let n_icons = images.len();
    // Directory tree layout: root, one type directory per type, then one
    // language directory per resource; then data entries; then the data.
    let root = 0u32;
    let type_icon = root + dir(2);
    let type_group = type_icon + dir(n_icons);
    let langs = type_group + dir(1);
    let entries = langs + dir(1) * res.len() as u32;
    let mut data_at = entries + 16 * res.len() as u32;

    let mut sec = Vec::new();
    let header = |sec: &mut Vec<u8>, ids: usize| {
        sec.extend([0u8; 12]); // characteristics, timestamp, version
        sec.extend(0u16.to_le_bytes()); // named entries
        sec.extend((ids as u16).to_le_bytes());
    };
    let entry = |sec: &mut Vec<u8>, id: u32, target: u32| {
        sec.extend(id.to_le_bytes());
        sec.extend(target.to_le_bytes());
    };
    header(&mut sec, 2);
    entry(&mut sec, RT_ICON, SUBDIR | type_icon);
    entry(&mut sec, RT_GROUP_ICON, SUBDIR | type_group);
    header(&mut sec, n_icons);
    for (k, &(_, id, _)) in res.iter().enumerate().take(n_icons) {
        entry(&mut sec, id, SUBDIR | (langs + dir(1) * k as u32));
    }
    header(&mut sec, 1);
    entry(&mut sec, 1, SUBDIR | (langs + dir(1) * n_icons as u32));
    for k in 0..res.len() {
        header(&mut sec, 1);
        entry(&mut sec, LANG_NEUTRAL, entries + 16 * k as u32);
    }
    // Data entries: the data's address is an RVA, filled in by a relocation.
    let mut relocs = Vec::new();
    for &(_, _, data) in &res {
        relocs.push(sec.len() as u32);
        sec.extend(data_at.to_le_bytes());
        sec.extend((data.len() as u32).to_le_bytes());
        sec.extend([0u8; 8]); // code page, reserved
        data_at = (data_at + data.len() as u32).next_multiple_of(8);
    }
    for &(_, _, data) in &res {
        sec.extend(data);
        sec.resize(sec.len().next_multiple_of(8), 0);
    }

    let reloc_type: u16 = match machine {
        0x14c => 0x0007,  // IMAGE_REL_I386_DIR32NB
        0xaa64 => 0x0002, // IMAGE_REL_ARM64_ADDR32NB
        _ => 0x0003,      // IMAGE_REL_AMD64_ADDR32NB
    };
    let raw_at = 20 + 40u32;
    let relocs_at = raw_at + sec.len() as u32;
    let symbols_at = relocs_at + 10 * relocs.len() as u32;

    let mut o = Vec::new();
    // File header.
    o.extend(machine.to_le_bytes());
    o.extend(1u16.to_le_bytes()); // sections
    o.extend(0u32.to_le_bytes()); // timestamp
    o.extend(symbols_at.to_le_bytes());
    o.extend(1u32.to_le_bytes()); // symbols
    o.extend(0u16.to_le_bytes()); // optional header size
    o.extend(0u16.to_le_bytes()); // characteristics
    // Section header.
    o.extend(*b".rsrc\0\0\0");
    o.extend(0u32.to_le_bytes()); // virtual size
    o.extend(0u32.to_le_bytes()); // virtual address
    o.extend((sec.len() as u32).to_le_bytes());
    o.extend(raw_at.to_le_bytes());
    o.extend(relocs_at.to_le_bytes());
    o.extend(0u32.to_le_bytes()); // line numbers
    o.extend((relocs.len() as u16).to_le_bytes());
    o.extend(0u16.to_le_bytes());
    o.extend(0x4030_0040u32.to_le_bytes()); // initialized data, read, 4-byte aligned
    o.extend(&sec);
    for at in relocs {
        o.extend(at.to_le_bytes());
        o.extend(0u32.to_le_bytes()); // symbol 0: the section
        o.extend(reloc_type.to_le_bytes());
    }
    // Symbol table: the section symbol, then an empty string table.
    o.extend(*b".rsrc\0\0\0");
    o.extend(0u32.to_le_bytes()); // value
    o.extend(1i16.to_le_bytes()); // section number
    o.extend(0u16.to_le_bytes()); // type
    o.push(3); // IMAGE_SYM_CLASS_STATIC
    o.push(0); // aux symbols
    o.extend(4u32.to_le_bytes());
    o
}
