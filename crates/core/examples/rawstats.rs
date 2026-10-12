use darkslide_core::decode::*;
fn main() {
    let p = std::env::args().nth(1).unwrap();
    let s = load(std::path::Path::new(&p)).unwrap();
    println!("neutral {:?} xy {:?} tt {:?}", s.as_shot_neutral, s.as_shot_xy(), darkslide_core::color::xy_to_temp_tint(s.as_shot_xy()));
    if let SourceData::Cfa(c) = &s.data {
        // region args: x0 y0 x1 y1 as fractions
        let a: Vec<f32> = std::env::args().skip(2).map(|v| v.parse().unwrap()).collect();
        let (x0, y0, x1, y1) = ((a[0] * c.w as f32) as usize, (a[1] * c.h as f32) as usize, (a[2] * c.w as f32) as usize, (a[3] * c.h as f32) as usize);
        let mut sum = [0f64; 3];
        let mut n = [0f64; 3];
        let mut clip = [0f64; 3];
        let mut mx = [0f32; 3];
        for y in y0..y1 {
            for x in x0..x1 {
                let col = c.pattern.color_at(x, y);
                let v = c.data[y * c.w + x];
                sum[col] += v as f64;
                n[col] += 1.0;
                if v >= 0.995 {
                    clip[col] += 1.0
                }
                mx[col] = mx[col].max(v);
            }
        }
        // unclipped 2x2 blocks: ratios after WB
        let mul = [1.0 / s.as_shot_neutral[0], 1.0, 1.0 / s.as_shot_neutral[2]];
        let mut rs = vec![];
        let mut bs = vec![];
        for y in (y0..y1).step_by(2) {
            for x in (x0..x1).step_by(2) {
                let mut v = [0f32; 3];
                let mut cnt = [0f32; 3];
                let mut mxv = 0f32;
                for dy in 0..2 {
                    for dx in 0..2 {
                        let col = c.pattern.color_at(x + dx, y + dy);
                        let val = c.data[(y + dy) * c.w + x + dx];
                        v[col] += val;
                        cnt[col] += 1.0;
                        mxv = mxv.max(val);
                    }
                }
                if mxv < 0.9 && v[1] > 0.3 {
                    rs.push(v[0] / cnt[0] * mul[0] / (v[1] / cnt[1]));
                    bs.push(v[2] / cnt[2] * mul[2] / (v[1] / cnt[1]));
                }
            }
        }
        rs.sort_by(|a, b| a.partial_cmp(b).unwrap());
        bs.sort_by(|a, b| a.partial_cmp(b).unwrap());
        if !rs.is_empty() {
            println!("bright unclipped n={} R/G median {:.3} B/G median {:.3}", rs.len(), rs[rs.len() / 2], bs[bs.len() / 2]);
        }
        for k in 0..3 {
            println!("c{k}: mean {:.4} clipped {:.3} max {:.4}", sum[k] / n[k], clip[k] / n[k], mx[k]);
        }
    }
}
#[allow(dead_code)]
fn _x() {}
