// SPDX-License-Identifier: GPL-3.0-or-later
// Copyright (C) 2026 dv-subsystem-set contributors
//! Embeds the manifest and the suite's icon, drawn here: a white download arrow into a tray on a rounded blue square.

/// how much of the pixel at (x, y) of a `size` square the icon covers, as (blue square, white arrow), 4x4 samples
fn cover(x: u32, y: u32, size: u32) -> (f32, f32) {
    let (mut square, mut arrow) = (0.0, 0.0);
    for sy in 0..4 {
        for sx in 0..4 {
            let u = (x as f32 + (sx as f32 + 0.5) / 4.0) / size as f32;
            let v = (y as f32 + (sy as f32 + 0.5) / 4.0) / size as f32;
            if in_square(u, v) {
                square += 1.0 / 16.0;
                if in_arrow(u, v) {
                    arrow += 1.0 / 16.0;
                }
            }
        }
    }
    (square, arrow)
}

fn in_square(u: f32, v: f32) -> bool {
    let (m, r) = (0.06, 0.2);
    let cx = u.clamp(m + r, 1.0 - m - r);
    let cy = v.clamp(m + r, 1.0 - m - r);
    (u - cx).powi(2) + (v - cy).powi(2) <= r * r
}

fn in_arrow(u: f32, v: f32) -> bool {
    let within = |x0: f32, x1: f32, y0: f32, y1: f32| u >= x0 && u <= x1 && v >= y0 && v <= y1;
    let stem = within(0.44, 0.56, 0.18, 0.48);
    // the head narrows from its top edge to the tip
    let head = v >= 0.42 && v <= 0.66 && (u - 0.5).abs() <= 0.24 * (0.66 - v) / 0.24;
    let tray = within(0.22, 0.78, 0.72, 0.80) || within(0.22, 0.30, 0.58, 0.80) || within(0.70, 0.78, 0.58, 0.80);
    stem || head || tray
}

/// premultiplied BGRA rows, bottom-up, as an icon's bitmap holds them
fn pixels(size: u32) -> Vec<u8> {
    let mut d = Vec::with_capacity((size * size * 4) as usize);
    for y in (0..size).rev() {
        for x in 0..size {
            let (square, arrow) = cover(x, y, size);
            // a little lighter at the top
            let t = y as f32 / size as f32;
            let blue = [0xF6 as f32 - 0x1E as f32 * t, 0x82 as f32 - 0x30 as f32 * t, 0x3B as f32 - 0x1A as f32 * t];
            let ch = |b: f32| ((b * (square - arrow) + 255.0 * arrow).round()).clamp(0.0, 255.0) as u8;
            d.extend_from_slice(&[ch(blue[0]), ch(blue[1]), ch(blue[2]), (square * 255.0).round() as u8]);
        }
    }
    d
}

fn ico(sizes: &[u32]) -> Vec<u8> {
    let images: Vec<Vec<u8>> = sizes
        .iter()
        .map(|&size| {
            let mut d = Vec::new();
            d.extend_from_slice(&40u32.to_le_bytes()); // BITMAPINFOHEADER
            d.extend_from_slice(&(size as i32).to_le_bytes());
            d.extend_from_slice(&(size as i32 * 2).to_le_bytes()); // height covers XOR + AND masks
            d.extend_from_slice(&1u16.to_le_bytes());
            d.extend_from_slice(&32u16.to_le_bytes());
            d.extend_from_slice(&[0; 24]);
            d.extend_from_slice(&pixels(size));
            d.resize(d.len() + (size.div_ceil(32) * 4 * size) as usize, 0); // AND mask: all opaque (alpha rules)
            d
        })
        .collect();
    let mut out = vec![0, 0, 1, 0];
    out.extend_from_slice(&(sizes.len() as u16).to_le_bytes());
    let mut offset = 6 + 16 * sizes.len() as u32;
    for (&size, img) in sizes.iter().zip(&images) {
        out.extend_from_slice(&[(size % 256) as u8, (size % 256) as u8, 0, 0, 1, 0, 32, 0]);
        out.extend_from_slice(&(img.len() as u32).to_le_bytes());
        out.extend_from_slice(&offset.to_le_bytes());
        offset += img.len() as u32;
    }
    for img in images {
        out.extend_from_slice(&img);
    }
    out
}

fn main() {
    println!("cargo:rerun-if-changed=build.rs");
    println!("cargo:rerun-if-changed=app.manifest");
    let out = std::path::PathBuf::from(std::env::var_os("OUT_DIR").unwrap());
    let manifest = std::fs::canonicalize("app.manifest").unwrap();
    std::fs::write(out.join("icon.ico"), ico(&[16, 20, 24, 32, 48, 64, 96, 256])).unwrap();
    // rc.exe wants forward slashes
    let fwd = |p: &std::path::Path| p.display().to_string().trim_start_matches(r"\\?\").replace('\\', "/");
    let rc = format!("1 ICON \"{}\"\n1 24 \"{}\"\n", fwd(&out.join("icon.ico")), fwd(&manifest));
    std::fs::write(out.join("app.rc"), rc).unwrap();
    embed_resource::compile(out.join("app.rc"), embed_resource::NONE).manifest_required().unwrap();
}
