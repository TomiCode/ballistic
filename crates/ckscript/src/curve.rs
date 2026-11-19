//! 2D curves (`CK2dCurve`) as saved in parameter chunks (the progression
//! curves of *Bezier Progression*).
//!
//! The layout, as found in Ballance's files (research findings 14):
//! - the words `0x100, 0, N`, with N the number of control points;
//! - N + 1 words (zero in every curve seen);
//! - N control points of 12 words: flags (bit 28 set), x, y, tension,
//!   continuity, bias, two words (zero), in tangent (x, y), out tangent (x, y);
//! - `0xFFFF_FFFF`.
//!
//! The curve's own chunk is nested inside the parameter's; the words are
//! searched for that layout.

/// One control point.
#[derive(Debug, Clone, Copy, PartialEq)]
#[cfg_attr(feature = "serde", derive(serde::Serialize))]
pub struct CurvePoint {
    pub x: f32,
    pub y: f32,
    pub in_tangent: [f32; 2],
    pub out_tangent: [f32; 2],
}

const POINT_WORDS: usize = 12;
const POINT_FLAG: u32 = 0x1000_0000;

/// The control points of a curve saved in `words`, if the layout is found.
pub fn decode(words: &[u32]) -> Option<Vec<CurvePoint>> {
    let f = |w: u32| f32::from_bits(w);
    (0..words.len().saturating_sub(3)).find_map(|i| {
        if words[i] != 0x100 || words[i + 1] != 0 {
            return None;
        }
        let n = words[i + 2] as usize;
        if n == 0 || n > 256 {
            return None;
        }
        let start = i + 3 + n + 1;
        let end = start + n * POINT_WORDS;
        if words.get(end) != Some(&0xFFFF_FFFF) {
            return None;
        }
        let points: Vec<CurvePoint> = words[start..end]
            .as_chunks::<POINT_WORDS>()
            .0
            .iter()
            .map(|p| CurvePoint {
                x: f(p[1]),
                y: f(p[2]),
                in_tangent: [f(p[8]), f(p[9])],
                out_tangent: [f(p[10]), f(p[11])],
            })
            .collect();
        let flags_ok = words[start..end]
            .as_chunks::<POINT_WORDS>()
            .0
            .iter()
            .all(|p| p[0] & POINT_FLAG != 0);
        let sorted = points.windows(2).all(|w| w[0].x <= w[1].x);
        (flags_ok && sorted).then_some(points)
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn decodes_a_two_point_curve() {
        let mut w = vec![0x27, 0xe, 0x7_000a, 0x1e, 0, 2, 0, 0, 0x100, 0, 2, 0, 0, 0];
        let p = |x: f32, y: f32, i: [f32; 2], o: [f32; 2]| {
            vec![
                0x1000_0001,
                x.to_bits(),
                y.to_bits(),
                0,
                0,
                0,
                0,
                0,
                i[0].to_bits(),
                i[1].to_bits(),
                o[0].to_bits(),
                o[1].to_bits(),
            ]
        };
        w.extend(p(0.0, 1.0, [0.0001, 0.0], [1.0, 0.05]));
        w.extend(p(1.0, 0.0, [1.0, -2.48], [0.001, -0.002_48]));
        w.push(0xFFFF_FFFF);
        let c = decode(&w).unwrap();
        assert_eq!(c.len(), 2);
        assert_eq!((c[0].x, c[0].y, c[0].out_tangent), (0.0, 1.0, [1.0, 0.05]));
        assert_eq!((c[1].x, c[1].y, c[1].in_tangent), (1.0, 0.0, [1.0, -2.48]));
    }
}
