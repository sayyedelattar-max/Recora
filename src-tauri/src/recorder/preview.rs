//! A small picture of what is about to be recorded, shown on the Record page.
//! Pixels come in as BGRA, go out as a 24-bit BMP in a data URL (the web view shows BMP natively, so no image library is needed).

/// Box-filters a BGRA picture down to `tw` pixels wide. Returns (width, height, BGR rows, top to bottom).
pub fn downscale_bgra(src: &[u8], sw: u32, sh: u32, tw: u32) -> (u32, u32, Vec<u8>) {
    let tw = tw.min(sw).max(1);
    let th = ((sh as u64 * tw as u64) / sw.max(1) as u64).max(1) as u32;
    let mut out = Vec::with_capacity((tw * th * 3) as usize);
    let n = ((sw as f64 / tw as f64).ceil() as u32).clamp(1, 4); // samples per axis inside each output pixel
    for y in 0..th {
        for x in 0..tw {
            let (mut b, mut g, mut r) = (0u32, 0u32, 0u32);
            for j in 0..n {
                for i in 0..n {
                    let sx = (((x * n + i) as u64 * sw as u64) / (tw * n) as u64).min(sw as u64 - 1) as usize;
                    let sy = (((y * n + j) as u64 * sh as u64) / (th * n) as u64).min(sh as u64 - 1) as usize;
                    let p = (sy * sw as usize + sx) * 4;
                    if p + 2 < src.len() {
                        b += src[p] as u32;
                        g += src[p + 1] as u32;
                        r += src[p + 2] as u32;
                    }
                }
            }
            let k = n * n;
            out.extend_from_slice(&[(b / k) as u8, (g / k) as u8, (r / k) as u8]);
        }
    }
    (tw, th, out)
}

pub fn bmp_data_url(w: u32, h: u32, bgr: &[u8]) -> String {
    let row = ((w * 3 + 3) / 4 * 4) as usize;
    let data = row * h as usize;
    let mut f: Vec<u8> = Vec::with_capacity(54 + data);
    f.extend_from_slice(b"BM");
    f.extend_from_slice(&((54 + data) as u32).to_le_bytes());
    f.extend_from_slice(&[0, 0, 0, 0]);
    f.extend_from_slice(&54u32.to_le_bytes());
    f.extend_from_slice(&40u32.to_le_bytes());
    f.extend_from_slice(&w.to_le_bytes());
    f.extend_from_slice(&(-(h as i32)).to_le_bytes()); // negative height: rows run top to bottom
    f.extend_from_slice(&1u16.to_le_bytes());
    f.extend_from_slice(&24u16.to_le_bytes());
    f.extend_from_slice(&[0; 24]);
    for y in 0..h as usize {
        let s = y * w as usize * 3;
        f.extend_from_slice(&bgr[s..s + w as usize * 3]);
        f.resize(f.len() + (row - w as usize * 3), 0);
    }
    format!("data:image/bmp;base64,{}", base64(&f))
}

fn base64(d: &[u8]) -> String {
    const T: &[u8; 64] = b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789+/";
    let mut o = String::with_capacity(d.len() / 3 * 4 + 4);
    for c in d.chunks(3) {
        let n = (c[0] as u32) << 16 | (*c.get(1).unwrap_or(&0) as u32) << 8 | *c.get(2).unwrap_or(&0) as u32;
        o.push(T[(n >> 18) as usize & 63] as char);
        o.push(T[(n >> 12) as usize & 63] as char);
        o.push(if c.len() > 1 { T[(n >> 6) as usize & 63] as char } else { '=' });
        o.push(if c.len() > 2 { T[n as usize & 63] as char } else { '=' });
    }
    o
}

/// A calm placeholder used by the test backend.
pub fn sample(w: u32, h: u32) -> String {
    let mut px = Vec::with_capacity((w * h * 4) as usize);
    for y in 0..h {
        for x in 0..w {
            let t = (x as f32 / w as f32 * 0.5 + y as f32 / h as f32 * 0.5) * 60.0;
            let win = (x > w / 8 && x < w * 5 / 8 && y > h / 6 && y < h * 4 / 6) || (x > w * 3 / 8 && x < w * 7 / 8 && y > h * 3 / 8 && y < h * 5 / 6);
            let v = if win { 238 } else { 150 + t as u8 };
            px.extend_from_slice(&[v, v - 4, v - 14, 255]);
        }
    }
    let (tw, th, bgr) = downscale_bgra(&px, w, h, w);
    bmp_data_url(tw, th, &bgr)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn downscale_keeps_the_aspect_ratio_and_the_colour() {
        let (w, h) = (400u32, 250u32);
        let src: Vec<u8> = (0..w * h).flat_map(|_| [10u8, 120, 240, 255]).collect();
        let (tw, th, bgr) = downscale_bgra(&src, w, h, 100);
        assert_eq!((tw, th), (100, 62));
        assert_eq!(bgr.len(), (tw * th * 3) as usize);
        assert!(bgr.chunks(3).all(|p| p == [10, 120, 240]));
    }

    #[test]
    fn bmp_is_well_formed() {
        let url = bmp_data_url(3, 2, &[0; 18]);
        assert!(url.starts_with("data:image/bmp;base64,Qk")); // "BM"
        // 3 px * 3 bytes = 9, padded to 12 per row; 54 + 24 = 78 bytes -> 104 base64 characters
        assert_eq!(url.trim_start_matches("data:image/bmp;base64,").len(), 104);
    }

    #[test]
    fn base64_matches_the_standard_examples() {
        assert_eq!(base64(b"M"), "TQ==");
        assert_eq!(base64(b"Ma"), "TWE=");
        assert_eq!(base64(b"Man"), "TWFu");
    }
}
