//! Embeds the Lanes icon as icon resource #1, which GPUI's Windows backend uses for the window
//! and Explorer uses for lanes.exe.

#[allow(dead_code)]
#[path = "src/icon_art.rs"]
mod icon_art;

use std::path::{Path, PathBuf};

const ICON_SIZES: [u32; 5] = [16, 24, 32, 48, 256];

fn main() {
    println!("cargo:rerun-if-changed=src/icon_art.rs");
    if std::env::var("CARGO_CFG_TARGET_OS").as_deref() != Ok("windows") {
        return;
    }
    let out_dir = PathBuf::from(std::env::var("OUT_DIR").expect("OUT_DIR is set by cargo"));
    let icon = out_dir.join("lanes.ico");
    std::fs::write(&icon, ico_file(&ICON_SIZES)).expect("writing lanes.ico");
    let rc = out_dir.join("lanes.rc");
    std::fs::write(&rc, format!("1 ICON \"{}\"\n", forward_slashes(&icon))).expect("writing lanes.rc");
    embed_resource::compile(&rc, embed_resource::NONE).manifest_optional().expect("compiling the icon resource");
}

/// A .ico holding one 32-bit image per size (alpha carries transparency, so the AND mask stays empty).
fn ico_file(sizes: &[u32]) -> Vec<u8> {
    let images: Vec<Vec<u8>> = sizes.iter().map(|&size| dib_image(size)).collect();
    let mut file = Vec::new();
    file.extend(u16_le(0));
    file.extend(u16_le(1));
    file.extend(u16_le(sizes.len() as u16));
    let mut offset = 6 + 16 * sizes.len() as u32;
    for (size, image) in sizes.iter().zip(&images) {
        let edge = if *size >= 256 { 0 } else { *size as u8 };
        file.extend([edge, edge, 0, 0]);
        file.extend(u16_le(1));
        file.extend(u16_le(32));
        file.extend(u32_le(image.len() as u32));
        file.extend(u32_le(offset));
        offset += image.len() as u32;
    }
    images.into_iter().for_each(|image| file.extend(image));
    file
}

/// BITMAPINFOHEADER, then BGRA rows bottom-up, then a blank 1-bit AND mask.
fn dib_image(size: u32) -> Vec<u8> {
    let rgba = icon_art::rgba(size, false);
    let mask_row = size.div_ceil(32) * 4;
    let mut dib = Vec::new();
    dib.extend(u32_le(40));
    dib.extend(i32_le(size as i32));
    dib.extend(i32_le(size as i32 * 2));
    dib.extend(u16_le(1));
    dib.extend(u16_le(32));
    dib.extend([0u8; 24]);
    for row in (0..size).rev() {
        for column in 0..size {
            let i = ((row * size + column) * 4) as usize;
            dib.extend([rgba[i + 2], rgba[i + 1], rgba[i], rgba[i + 3]]);
        }
    }
    dib.extend(vec![0u8; (mask_row * size) as usize]);
    dib
}

fn forward_slashes(path: &Path) -> String {
    path.to_string_lossy().replace('\\', "/")
}

fn u16_le(value: u16) -> [u8; 2] {
    value.to_le_bytes()
}

fn u32_le(value: u32) -> [u8; 4] {
    value.to_le_bytes()
}

fn i32_le(value: i32) -> [u8; 4] {
    value.to_le_bytes()
}
