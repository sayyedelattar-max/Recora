//! Draws the shapes the editor places on a video (rounded boxes, ellipses, arrows, spotlights, masks) as PNG pictures,
//! so ffmpeg only has to lay them over the frame. Edges are anti-aliased.
use anyhow::Result;

pub struct Spec<'a> {
    pub shape: &'a str,
    pub w: u32,
    pub h: u32,
    pub rgb: [u8; 3],
    /// opacity of the inside, 0 to 1
    pub fill: f64,
    /// draw the outline
    pub outline: bool,
    /// outline thickness in pixels
    pub thick: f64,
    pub flipx: bool,
    pub flipy: bool,
    /// a mask: white inside the shape, black outside, no outline (used for blur shapes)
    pub mask: bool,
}

fn clamp01(v: f64) -> f64 {
    v.clamp(0.0, 1.0)
}

/// Signed distance to the shape (negative inside) for a box of w x h centred at the origin.
fn sd(shape: &str, px: f64, py: f64, w: f64, h: f64) -> f64 {
    let (hx, hy) = (w / 2.0, h / 2.0);
    match shape {
        "ellipse" => {
            let (rx, ry) = (hx.max(1.0), hy.max(1.0));
            (((px / rx).powi(2) + (py / ry).powi(2)).sqrt() - 1.0) * rx.min(ry)
        }
        _ => {
            let r = if shape == "round" { (w.min(h) * 0.2).max(2.0) } else { 0.0 };
            let (qx, qy) = (px.abs() - hx + r, py.abs() - hy + r);
            (qx.max(0.0).powi(2) + qy.max(0.0).powi(2)).sqrt() + qx.max(qy).min(0.0) - r
        }
    }
}

fn dist_seg(px: f64, py: f64, ax: f64, ay: f64, bx: f64, by: f64) -> f64 {
    let (dx, dy) = (bx - ax, by - ay);
    let t = if dx == 0.0 && dy == 0.0 { 0.0 } else { clamp01(((px - ax) * dx + (py - ay) * dy) / (dx * dx + dy * dy)) };
    ((px - ax - t * dx).powi(2) + (py - ay - t * dy).powi(2)).sqrt()
}

fn in_tri(px: f64, py: f64, a: (f64, f64), b: (f64, f64), c: (f64, f64)) -> bool {
    let s = |p: (f64, f64), q: (f64, f64)| (px - q.0) * (p.1 - q.1) - (p.0 - q.0) * (py - q.1);
    let (d1, d2, d3) = (s(a, b), s(b, c), s(c, a));
    !((d1 < 0.0 || d2 < 0.0 || d3 < 0.0) && (d1 > 0.0 || d2 > 0.0 || d3 > 0.0))
}

/// Coverage of an arrow or line at a pixel, 0 or 1 (the caller supersamples).
fn arrow_hit(px: f64, py: f64, w: f64, h: f64, t: f64, head: bool, fx: bool, fy: bool) -> bool {
    let m = (t * 1.2).min(w.min(h) / 3.0);
    let (mut x, mut y) = (px, py);
    if fx { x = w - x; }
    if fy { y = h - y; }
    let (ax, ay, bx, by) = (m, m, w - m, h - m);
    let len = ((bx - ax).powi(2) + (by - ay).powi(2)).sqrt().max(1.0);
    let (ux, uy) = ((bx - ax) / len, (by - ay) / len);
    let hl = if head { (t * 5.0).max(len * 0.22).min(len * 0.6) } else { 0.0 };
    let (sx, sy) = (bx - ux * hl * 0.8, by - uy * hl * 0.8);
    if dist_seg(x, y, ax, ay, sx, sy) <= t / 2.0 {
        return true;
    }
    if head {
        let (nx, ny) = (-uy, ux);
        let (cx, cy) = (bx - ux * hl, by - uy * hl);
        let hw = hl * 0.55;
        return in_tri(x, y, (bx, by), (cx + nx * hw, cy + ny * hw), (cx - nx * hw, cy - ny * hw));
    }
    false
}

pub fn png(s: &Spec) -> Result<Vec<u8>> {
    let (w, h) = (s.w.max(2), s.h.max(2));
    let mut buf = vec![0u8; (w * h * 4) as usize];
    let t = s.thick.max(1.0);
    let (wf, hf) = (w as f64, h as f64);
    let line = s.shape == "arrow" || s.shape == "line";
    for y in 0..h {
        for x in 0..w {
            let (px, py) = (x as f64 + 0.5, y as f64 + 0.5);
            let a = if line {
                let mut hit = 0.0;
                for (ox, oy) in [(0.25, 0.25), (0.75, 0.25), (0.25, 0.75), (0.75, 0.75)] {
                    if arrow_hit(x as f64 + ox, y as f64 + oy, wf, hf, t, s.shape == "arrow", s.flipx, s.flipy) { hit += 0.25; }
                }
                hit
            } else {
                let d = sd(s.shape, px - wf / 2.0, py - hf / 2.0, wf, hf);
                let inside = clamp01(0.5 - d);
                if s.mask {
                    inside
                } else {
                    let ring = if s.outline { clamp01(t / 2.0 + 0.5 - (d + t / 2.0).abs()) } else { 0.0 };
                    (inside * s.fill).max(ring)
                }
            };
            let i = ((y * w + x) * 4) as usize;
            let c = if s.mask { [255, 255, 255] } else { s.rgb };
            buf[i] = c[0];
            buf[i + 1] = c[1];
            buf[i + 2] = c[2];
            buf[i + 3] = (a * 255.0).round() as u8;
        }
    }
    encode(&buf, w, h)
}

/// A frame-sized picture that dims everything except a shaped hole.
pub fn spotlight(vw: u32, vh: u32, hx: f64, hy: f64, hw: f64, hh: f64, shape: &str, dim: f64) -> Result<Vec<u8>> {
    let (w, h) = (vw.max(2), vh.max(2));
    let mut buf = vec![0u8; (w * h * 4) as usize];
    let (cx, cy) = (hx + hw / 2.0, hy + hh / 2.0);
    let feather = (h as f64 * 0.012).max(3.0);
    for y in 0..h {
        for x in 0..w {
            let d = sd(shape, x as f64 + 0.5 - cx, y as f64 + 0.5 - cy, hw, hh);
            let a = dim * clamp01(0.5 + d / feather);
            let i = ((y * w + x) * 4) as usize;
            buf[i + 3] = (a * 255.0).round() as u8;
        }
    }
    encode(&buf, w, h)
}

fn encode(buf: &[u8], w: u32, h: u32) -> Result<Vec<u8>> {
    let mut out = vec![];
    {
        let mut e = png::Encoder::new(&mut out, w, h);
        e.set_color(png::ColorType::Rgba);
        e.set_depth(png::BitDepth::Eight);
        e.set_compression(png::Compression::Fast);
        let mut wr = e.write_header()?;
        wr.write_image_data(buf)?;
    }
    Ok(out)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn alpha(p: &[u8], w: u32, x: u32, y: u32) -> u8 {
        let d = png::Decoder::new(p);
        let mut r = d.read_info().unwrap();
        let mut b = vec![0; r.output_buffer_size()];
        r.next_frame(&mut b).unwrap();
        b[((y * w + x) * 4 + 3) as usize]
    }
    fn spec(shape: &str) -> Spec<'_> {
        Spec { shape, w: 100, h: 60, rgb: [255, 0, 0], fill: 0.0, outline: true, thick: 6.0, flipx: false, flipy: false, mask: false }
    }

    #[test]
    fn ellipse_has_a_ring_and_a_clear_corner() {
        let p = png(&spec("ellipse")).unwrap();
        assert!(alpha(&p, 100, 0, 0) == 0, "corner must be clear");
        assert!(alpha(&p, 100, 50, 2) > 200, "top of the ring");
        assert!(alpha(&p, 100, 50, 30) == 0, "centre is empty when not filled");
    }
    #[test]
    fn masks_are_solid_inside() {
        let mut s = spec("round");
        s.mask = true;
        let p = png(&s).unwrap();
        assert!(alpha(&p, 100, 50, 30) == 255);
        assert!(alpha(&p, 100, 0, 0) < 30, "rounded corner");
    }
    #[test]
    fn arrow_draws_a_head_and_flips() {
        let mut s = spec("arrow");
        s.w = 80; s.h = 80;
        let p = png(&s).unwrap();
        assert!(alpha(&p, 80, 70, 70) > 100, "tip near the corner");
        assert_eq!(alpha(&p, 80, 70, 5), 0);
        s.flipx = true;
        let q = png(&s).unwrap();
        assert!(alpha(&q, 80, 9, 70) > 100);
    }
    #[test]
    fn spotlight_dims_outside_only() {
        let p = spotlight(200, 100, 60.0, 30.0, 80.0, 40.0, "ellipse", 0.7).unwrap();
        assert!(alpha(&p, 200, 100, 50) < 10, "hole");
        assert!(alpha(&p, 200, 2, 2) > 150, "dimmed");
    }
}
