//! The app icon and the installer pictures, drawn in code.
//!
//! The icon is a lensless webcam's sensor with a laser line across it. Each size is drawn for its
//! own pixel grid, so the line stays sharp at 16 px.

use std::path::Path;

use tiny_skia::{
    Color, FillRule, GradientStop, LinearGradient, Mask, Paint, Path as SkPath, PathBuilder, Pixmap, PixmapPaint,
    Point, Rect, SpreadMode, Stroke, Transform,
};

/// Sizes in the `.ico` file. Windows picks the one that fits the place it shows the icon.
const ICO_SIZES: [u32; 10] = [16, 20, 24, 32, 40, 48, 64, 96, 128, 256];
const LASER: (u8, u8, u8) = (255, 46, 34);

fn rounded(x0: f32, y0: f32, x1: f32, y1: f32, r: f32) -> SkPath {
    let k = 0.552_284_8 * r;
    let mut pb = PathBuilder::new();
    pb.move_to(x0 + r, y0);
    pb.line_to(x1 - r, y0);
    pb.cubic_to(x1 - r + k, y0, x1, y0 + r - k, x1, y0 + r);
    pb.line_to(x1, y1 - r);
    pb.cubic_to(x1, y1 - r + k, x1 - r + k, y1, x1 - r, y1);
    pb.line_to(x0 + r, y1);
    pb.cubic_to(x0 + r - k, y1, x0, y1 - r + k, x0, y1 - r);
    pb.line_to(x0, y0 + r);
    pb.cubic_to(x0, y0 + r - k, x0 + r - k, y0, x0 + r, y0);
    pb.close();
    pb.finish().expect("a rounded rectangle")
}

fn rgba(r: u8, g: u8, b: u8, a: f32) -> Color {
    Color::from_rgba8(r, g, b, (a.clamp(0.0, 1.0) * 255.0).round() as u8)
}

fn vertical(y0: f32, y1: f32, stops: Vec<GradientStop>) -> Paint<'static> {
    let shader = LinearGradient::new(Point::from_xy(0.0, y0), Point::from_xy(0.0, y1), stops, SpreadMode::Pad, Transform::identity())
        .expect("a gradient");
    Paint { shader, anti_alias: true, ..Paint::default() }
}

fn diagonal(x0: f32, y0: f32, x1: f32, y1: f32, from: Color, to: Color) -> Paint<'static> {
    let stops = vec![GradientStop::new(0.0, from), GradientStop::new(1.0, to)];
    let shader = LinearGradient::new(Point::from_xy(x0, y0), Point::from_xy(x1, y1), stops, SpreadMode::Pad, Transform::identity())
        .expect("a gradient");
    Paint { shader, anti_alias: true, ..Paint::default() }
}

/// The laser line's glow: a Gaussian across the line, `half` px each side of `cy`.
fn glow(pm: &mut Pixmap, x0: f32, x1: f32, cy: f32, half: f32, peak: f32, mask: Option<&Mask>) {
    let (r, g, b) = LASER;
    let stops = (0..=16)
        .map(|i| {
            let t = i as f32 / 16.0;
            let z = (t - 0.5) / 0.16;
            GradientStop::new(t, rgba(r, g, b, peak * (-0.5 * z * z).exp()))
        })
        .collect();
    let paint = vertical(cy - half, cy + half, stops);
    if let Some(rect) = Rect::from_ltrb(x0, cy - half, x1, cy + half) {
        pm.fill_rect(rect, &paint, Transform::identity(), mask);
    }
}

/// The bright core of the line, on whole pixel rows.
fn core(pm: &mut Pixmap, x0: f32, x1: f32, cy: f32, height: f32, mask: Option<&Mask>) {
    let h = height.round().max(1.0);
    let top = (cy - h / 2.0).round();
    let (r, g, b) = LASER;
    let paint = if h <= 2.0 {
        let mut p = Paint { anti_alias: true, ..Paint::default() };
        p.set_color_rgba8(255, 84, 70, 255);
        p
    } else {
        vertical(top, top + h, vec![
            GradientStop::new(0.0, rgba(r, g, b, 1.0)),
            GradientStop::new(0.5, rgba(255, 238, 232, 1.0)),
            GradientStop::new(1.0, rgba(r, g, b, 1.0)),
        ])
    };
    if let Some(rect) = Rect::from_ltrb(x0, top, x1, top + h) {
        pm.fill_rect(rect, &paint, Transform::identity(), mask);
    }
}

/// The app icon at `size` × `size` px.
pub fn icon(size: u32) -> Pixmap {
    let s = size as f32;
    let mut pm = Pixmap::new(size, size).expect("an icon size above 0");
    let pad = if size <= 24 { 0.0 } else { (s / 32.0).round() };
    let (t0, t1) = (pad, s - pad);
    let tile = rounded(t0, t0, t1, t1, (t1 - t0) * 0.21);

    // The camera body: a slate tile.
    let body = vertical(t0, t1, vec![
        GradientStop::new(0.0, rgba(66, 74, 87, 1.0)),
        GradientStop::new(1.0, rgba(28, 32, 39, 1.0)),
    ]);
    pm.fill_path(&tile, &body, FillRule::Winding, Transform::identity(), None);

    // The sensor: a dark die with the faint violet sheen of a bare image sensor.
    let m = t0 + (t1 - t0) * 0.2;
    let die = rounded(m, m, s - m, s - m, (s - 2.0 * m) * 0.08);
    let sheen = diagonal(m, m, s - m, s - m, rgba(44, 36, 66, 1.0), rgba(10, 10, 16, 1.0));
    pm.fill_path(&die, &sheen, FillRule::Winding, Transform::identity(), None);
    if size >= 64 {
        // Pixel rows and columns.
        let mut grid = PathBuilder::new();
        let n = 8;
        for i in 1..n {
            let p = m + (s - 2.0 * m) * i as f32 / n as f32;
            grid.move_to(p, m);
            grid.line_to(p, s - m);
            grid.move_to(m, p);
            grid.line_to(s - m, p);
        }
        if let Some(grid) = grid.finish() {
            let mut paint = Paint { anti_alias: true, ..Paint::default() };
            paint.set_color(rgba(255, 255, 255, 0.05));
            let stroke = Stroke { width: (s / 256.0).max(1.0), ..Stroke::default() };
            pm.stroke_path(&grid, &paint, &stroke, Transform::identity(), None);
        }
    }

    // The laser line, across the whole tile and clipped to it.
    let mut clip = Mask::new(size, size).expect("a mask");
    clip.fill_path(&tile, FillRule::Winding, true, Transform::identity());
    let cy = s / 2.0;
    glow(&mut pm, 0.0, s, cy, s * 0.22, 0.62, Some(&clip));
    core(&mut pm, 0.0, s, cy, s * 0.06, Some(&clip));

    // A thin light edge so the tile shows on a dark taskbar.
    if size >= 32 {
        let mut paint = Paint { anti_alias: true, ..Paint::default() };
        paint.set_color(rgba(255, 255, 255, 0.14));
        let w = (s / 128.0).max(1.0);
        let edge = rounded(t0 + w / 2.0, t0 + w / 2.0, t1 - w / 2.0, t1 - w / 2.0, (t1 - t0) * 0.21 - w / 2.0);
        pm.stroke_path(&edge, &paint, &Stroke { width: w, ..Stroke::default() }, Transform::identity(), None);
    }
    pm
}

/// RGBA bytes, not premultiplied.
fn straight(pm: &Pixmap) -> Vec<u8> {
    pm.pixels()
        .iter()
        .flat_map(|p| {
            let c = p.demultiply();
            [c.red(), c.green(), c.blue(), c.alpha()]
        })
        .collect()
}

pub fn png(pm: &Pixmap) -> Vec<u8> {
    let mut out = Vec::new();
    let mut enc = png::Encoder::new(&mut out, pm.width(), pm.height());
    enc.set_color(png::ColorType::Rgba);
    enc.set_depth(png::BitDepth::Eight);
    let mut w = enc.write_header().expect("a PNG header");
    w.write_image_data(&straight(pm)).expect("PNG data");
    w.finish().expect("a PNG file");
    out
}

/// An `.ico` file with a PNG for each size.
fn ico() -> Vec<u8> {
    let images: Vec<(u32, Vec<u8>)> = ICO_SIZES.iter().map(|&s| (s, png(&icon(s)))).collect();
    let mut out = vec![0, 0, 1, 0];
    out.extend((images.len() as u16).to_le_bytes());
    let mut offset = 6 + 16 * images.len() as u32;
    for (size, data) in &images {
        let side = if *size >= 256 { 0 } else { *size as u8 };
        out.extend([side, side, 0, 0]);
        out.extend(1u16.to_le_bytes());
        out.extend(32u16.to_le_bytes());
        out.extend((data.len() as u32).to_le_bytes());
        out.extend(offset.to_le_bytes());
        offset += data.len() as u32;
    }
    for (_, data) in images {
        out.extend(data);
    }
    out
}

/// The icon as macOS app icons sit on their canvas: the tile at 824/1024 of the side, centred, so
/// it lines up with the other apps in the Dock.
fn mac_icon(size: u32) -> Pixmap {
    let inner = (size as f32 * 824.0 / 1024.0).round() as u32;
    let mut pm = Pixmap::new(size, size).expect("an icon size above 0");
    let at = ((size - inner) / 2) as i32;
    pm.draw_pixmap(at, at, icon(inner).as_ref(), &PixmapPaint::default(), Transform::identity(), None);
    pm
}

/// A macOS `.icns` file with a PNG for each size and scale that Finder and the Dock ask for.
fn icns() -> Vec<u8> {
    const ENTRIES: [(&[u8; 4], u32); 10] = [
        (b"icp4", 16),
        (b"icp5", 32),
        (b"ic11", 32),
        (b"ic12", 64),
        (b"ic07", 128),
        (b"ic13", 256),
        (b"ic08", 256),
        (b"ic14", 512),
        (b"ic09", 512),
        (b"ic10", 1024),
    ];
    let mut body = Vec::new();
    for (kind, size) in ENTRIES {
        let data = png(&mac_icon(size));
        body.extend(kind);
        body.extend((data.len() as u32 + 8).to_be_bytes());
        body.extend(data);
    }
    let mut out = b"icns".to_vec();
    out.extend((body.len() as u32 + 8).to_be_bytes());
    out.extend(body);
    out
}

/// A 24-bit `.bmp` file of an opaque picture, as the installer needs.
fn bmp(pm: &Pixmap) -> Vec<u8> {
    let (w, h) = (pm.width(), pm.height());
    let row = (w * 3).div_ceil(4) * 4;
    let size = 54 + row * h;
    let mut out = Vec::with_capacity(size as usize);
    out.extend(b"BM");
    out.extend(size.to_le_bytes());
    out.extend(0u32.to_le_bytes());
    out.extend(54u32.to_le_bytes());
    out.extend(40u32.to_le_bytes());
    out.extend((w as i32).to_le_bytes());
    out.extend((h as i32).to_le_bytes());
    out.extend(1u16.to_le_bytes());
    out.extend(24u16.to_le_bytes());
    out.extend(0u32.to_le_bytes());
    out.extend((row * h).to_le_bytes());
    out.extend(2835i32.to_le_bytes());
    out.extend(2835i32.to_le_bytes());
    out.extend(0u32.to_le_bytes());
    out.extend(0u32.to_le_bytes());
    let px = straight(pm);
    for y in (0..h).rev() {
        let start = out.len();
        for x in 0..w {
            let i = ((y * w + x) * 4) as usize;
            out.extend([px[i + 2], px[i + 1], px[i]]);
        }
        out.resize(start + row as usize, 0);
    }
    out
}

fn white(w: u32, h: u32) -> Pixmap {
    let mut pm = Pixmap::new(w, h).expect("a picture size above 0");
    pm.fill(Color::WHITE);
    pm
}

/// The strip at the top of the installer's inner pages: white, so the page title stays legible,
/// with the icon at the right.
fn banner() -> Pixmap {
    let mut pm = white(493, 58);
    let i = icon(44);
    pm.draw_pixmap(493 - 44 - 10, 7, i.as_ref(), &PixmapPaint::default(), Transform::identity(), None);
    pm
}

/// The first and last installer pages: a dark panel at the left with the laser line running through
/// the icon, and white at the right under the text.
fn dialog() -> Pixmap {
    let mut pm = white(493, 312);
    let panel_w = 164.0;
    let panel = Rect::from_ltrb(0.0, 0.0, panel_w, 312.0).expect("the panel");
    let shade = vertical(0.0, 312.0, vec![
        GradientStop::new(0.0, rgba(46, 52, 62, 1.0)),
        GradientStop::new(1.0, rgba(20, 22, 27, 1.0)),
    ]);
    pm.fill_rect(panel, &shade, Transform::identity(), None);
    let cy = 120.0;
    glow(&mut pm, 0.0, panel_w, cy, 30.0, 0.5, None);
    core(&mut pm, 0.0, panel_w, cy, 3.0, None);
    let size = 96;
    let i = icon(size);
    let x = ((panel_w - size as f32) / 2.0).round() as i32;
    pm.draw_pixmap(x, (cy - size as f32 / 2.0) as i32, i.as_ref(), &PixmapPaint::default(), Transform::identity(), None);
    pm
}

fn write(path: &Path, bytes: &[u8]) -> Result<(), String> {
    if let Some(dir) = path.parent() {
        std::fs::create_dir_all(dir).map_err(|e| format!("{}: {e}", dir.display()))?;
    }
    std::fs::write(path, bytes).map_err(|e| format!("{}: {e}", path.display()))?;
    println!("wrote {} ({} bytes)", path.display(), bytes.len());
    Ok(())
}

/// Writes the icon files and the installer pictures into the repository.
pub fn write_all(root: &Path) -> Result<(), String> {
    write(&root.join("assets/icon.ico"), &ico())?;
    write(&root.join("assets/icon.png"), &png(&icon(256)))?;
    write(&root.join("assets/icon.icns"), &icns())?;
    write(&root.join("packaging/windows/banner.bmp"), &bmp(&banner()))?;
    write(&root.join("packaging/windows/dialog.bmp"), &bmp(&dialog()))?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The line is the brightest thing on the icon, and it is red, at every size.
    #[test]
    fn the_laser_line_is_bright_and_red_at_every_size() {
        for size in ICO_SIZES {
            let px = straight(&icon(size));
            let row = (size / 2) as usize;
            let mid = (row * size as usize + size as usize / 2) * 4;
            let corner_row = (size as usize / 4 * size as usize + size as usize / 4) * 4;
            assert!(px[mid] > 240 && px[mid] as i32 - px[mid + 2] as i32 >= 0, "size {size}: centre {:?}", &px[mid..mid + 4]);
            assert!(px[corner_row] < 120, "size {size}: the sensor away from the line is dark");
        }
    }

    #[test]
    fn ico_and_bmp_have_the_right_headers() {
        let ico = ico();
        assert_eq!(&ico[..4], &[0, 0, 1, 0]);
        assert_eq!(u16::from_le_bytes([ico[4], ico[5]]) as usize, ICO_SIZES.len());
        let b = bmp(&banner());
        assert_eq!(&b[..2], b"BM");
        assert_eq!(u32::from_le_bytes([b[2], b[3], b[4], b[5]]) as usize, b.len());
    }
}
