//! Frosted "hide zone": averages a rectangle into soft blocks and then blurs the blocks together.
//! The result keeps the colours and layout (a chat still looks like a chat) but text and faces cannot be recovered.

use std::sync::atomic::{AtomicU8, Ordering};

/// 1 light, 2 medium, 3 strong. Set from the settings before a recording starts.
static LEVEL: AtomicU8 = AtomicU8::new(1);
pub fn set_level(l: u8) {
    LEVEL.store(l.clamp(1, 3), Ordering::Relaxed);
}

pub type Rect4 = (i32, i32, i32, i32); // left, top, right, bottom

/// The parts of `target` that nothing in `above` (windows in front of it) covers. Up to four pieces per covering window.
pub fn visible_parts(target: Rect4, above: &[Rect4]) -> Vec<Rect4> {
    let mut parts = vec![target];
    for &(cl, ct, cr, cb) in above {
        let mut next = Vec::with_capacity(parts.len() + 3);
        for (l, t, r, b) in parts {
            if cr <= l || cl >= r || cb <= t || ct >= b {
                next.push((l, t, r, b)); // no overlap
                continue;
            }
            if ct > t { next.push((l, t, r, ct)); } // strip above the cover
            if cb < b { next.push((l, cb, r, b)); } // strip below
            let (mt, mb) = (t.max(ct), b.min(cb));
            if cl > l { next.push((l, mt, cl, mb)); } // left of the cover
            if cr < r { next.push((cr, mt, r, mb)); } // right of the cover
        }
        parts = next;
    }
    parts.retain(|&(l, t, r, b)| r - l > 3 && b - t > 3);
    parts
}

/// Blurs `x0,y0 .. x1,y1` of a BGRA picture (w x h) in place. Out-of-range rectangles are clamped.
pub fn soften(buf: &mut [u8], w: u32, h: u32, x0: i32, y0: i32, x1: i32, y1: i32) {
    let (w, h) = (w as usize, h as usize);
    if buf.len() < w * h * 4 {
        return;
    }
    let (x0, y0) = (x0.max(0) as usize, y0.max(0) as usize);
    let (x1, y1) = ((x1.max(0) as usize).min(w), (y1.max(0) as usize).min(h));
    if x1 <= x0 + 3 || y1 <= y0 + 3 {
        return;
    }
    let (rw, rh) = (x1 - x0, y1 - y0);
    // block size and how far the blocks are blended: light keeps the look of the window, strong hides it
    let level = LEVEL.load(Ordering::Relaxed);
    let (b, r) = match level {
        3 => { let b = (rw.min(rh) / 14).clamp(10, 26); (b, (b / 2).max(2)) }
        2 => { let b = (rw.min(rh) / 24).clamp(8, 18); (b, (b / 3).max(2)) }
        _ => { let b = (rw.min(rh) / 34).clamp(6, 12); (b, (b / 5).max(1)) }
    };

    // 1. average every b x b block
    let mut by = 0;
    while by < rh {
        let bh = b.min(rh - by);
        let mut bx = 0;
        while bx < rw {
            let bw = b.min(rw - bx);
            let mut s = [0u32; 3];
            for y in 0..bh {
                let row = ((y0 + by + y) * w + x0 + bx) * 4;
                for x in 0..bw {
                    let p = row + x * 4;
                    s[0] += buf[p] as u32;
                    s[1] += buf[p + 1] as u32;
                    s[2] += buf[p + 2] as u32;
                }
            }
            let n = (bw * bh) as u32;
            let avg = [(s[0] / n) as u8, (s[1] / n) as u8, (s[2] / n) as u8];
            for y in 0..bh {
                let row = ((y0 + by + y) * w + x0 + bx) * 4;
                for x in 0..bw {
                    let p = row + x * 4;
                    buf[p] = avg[0];
                    buf[p + 1] = avg[1];
                    buf[p + 2] = avg[2];
                    buf[p + 3] = 255;
                }
            }
            bx += b;
        }
        by += b;
    }

    // 2. blur the blocks into each other (horizontal, then vertical, running sums)
    let r = r as isize;
    let mut tmp = vec![0u8; rw * rh * 4];
    for y in 0..rh {
        for c in 0..3 {
            line(buf, ((y0 + y) * w + x0) * 4 + c, 4, rw, &mut tmp, (y * rw) * 4 + c, 4, r);
        }
    }
    for x in 0..rw {
        for c in 0..3 {
            let mut col = vec![0u8; rh];
            line(&tmp, x * 4 + c, rw * 4, rh, &mut col, 0, 1, r);
            for y in 0..rh {
                buf[((y0 + y) * w + x0 + x) * 4 + c] = col[y];
            }
        }
    }
}

/// One box-blurred line: reads n samples from `src` (start, step), writes them to `dst` (start, step); edges repeat.
fn line(src: &[u8], s0: usize, sstep: usize, n: usize, dst: &mut [u8], d0: usize, dstep: usize, r: isize) {
    let at = |i: isize| -> i32 { src[s0 + (i.clamp(0, n as isize - 1) as usize) * sstep] as i32 };
    let win = (2 * r + 1) as i32;
    let mut sum: i32 = (-r..=r).map(at).sum();
    for i in 0..n as isize {
        dst[d0 + (i as usize) * dstep] = (sum / win) as u8;
        sum += at(i + r + 1) - at(i - r);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn area(v: &[Rect4]) -> i64 {
        v.iter().map(|&(l, t, r, b)| ((r - l) as i64) * ((b - t) as i64)).sum()
    }

    #[test]
    fn nothing_in_front_keeps_the_whole_window() {
        assert_eq!(visible_parts((0, 0, 100, 80), &[]), vec![(0, 0, 100, 80)]);
        assert_eq!(visible_parts((0, 0, 100, 80), &[(200, 0, 300, 80)]), vec![(0, 0, 100, 80)]);
    }

    #[test]
    fn a_window_in_front_is_cut_out() {
        let parts = visible_parts((0, 0, 100, 100), &[(40, 40, 60, 60)]);
        assert_eq!(area(&parts), 10_000 - 400);
        // no piece may touch the covering window
        assert!(parts.iter().all(|&(l, t, r, b)| r <= 40 || l >= 60 || b <= 40 || t >= 60));
    }

    #[test]
    fn fully_covered_leaves_nothing_and_partial_covers_stack() {
        assert!(visible_parts((10, 10, 50, 50), &[(0, 0, 200, 200)]).is_empty());
        let parts = visible_parts((0, 0, 100, 100), &[(0, 0, 50, 100), (50, 0, 100, 50)]);
        assert_eq!(area(&parts), 50 * 50);
    }


    #[test]
    fn text_like_pattern_becomes_flat_and_outside_is_untouched() {
        let (w, h) = (200u32, 120u32);
        let mut buf = vec![0u8; (w * h * 4) as usize];
        for y in 0..h as usize {
            for x in 0..w as usize {
                let v = if x % 2 == 0 { 255 } else { 0 }; // finest possible "text"
                let p = (y * w as usize + x) * 4;
                buf[p..p + 4].copy_from_slice(&[v, v, v, 255]);
            }
        }
        let before = buf.clone();
        soften(&mut buf, w, h, 40, 20, 160, 100);
        // inside: every value close to the mean, so nothing readable is left
        for y in 20..100usize {
            for x in 40..160usize {
                let v = buf[(y * w as usize + x) * 4] as i32;
                assert!((v - 127).abs() <= 6, "pixel {x},{y} = {v}");
            }
        }
        // outside: identical to before
        for y in 0..h as usize {
            for x in 0..w as usize {
                let inside = (40..160).contains(&x) && (20..100).contains(&y);
                if !inside {
                    let p = (y * w as usize + x) * 4;
                    assert_eq!(buf[p..p + 4], before[p..p + 4]);
                }
            }
        }
    }

    #[test]
    fn tiny_and_out_of_range_rectangles_do_not_panic() {
        let mut buf = vec![9u8; 50 * 50 * 4];
        soften(&mut buf, 50, 50, -30, -30, 3, 3);
        soften(&mut buf, 50, 50, 40, 40, 500, 500);
        soften(&mut buf, 50, 50, 10, 10, 11, 11);
        soften(&mut buf, 50, 50, 20, 20, 10, 10);
    }
}
