// Port of avatar_aura/ae_convert.py: the row-major 4x4/3x3 helpers, `_fmt`, `_safe_id`.
// Every operation keeps the Python evaluation order in f64 so the writers reproduce its output bit for bit.

/// 3-vector.
pub type V3 = [f64; 3];
/// 3x3 matrix as rows.
pub type M3 = [[f64; 3]; 3];
/// 4x4 matrix as rows, column vectors.
pub type M4 = [[f64; 4]; 4];

/// Avatar frame: right-handed, +Y up, faces +Z, left +X.
pub const FRAME_XBOX: M3 = [[1.0, 0.0, 0.0], [0.0, 1.0, 0.0], [0.0, 0.0, 1.0]];
/// Source engine frame, Z up, faces +X, left +Y: (x, y, z) -> (z, x, y).
pub const FRAME_SOURCE: M3 = [[0.0, 0.0, 1.0], [1.0, 0.0, 0.0], [0.0, 1.0, 0.0]];

/// The f64 the bake's decimal text denotes, recovered from its f32 parse.
/// The bake prints at most seven significant digits, so the shortest f32 repr is that decimal.
pub fn widen(v: f32) -> f64 {
    if !v.is_finite() {
        return f64::from(v);
    }
    v.to_string().parse().unwrap_or(f64::from(v))
}

pub fn widen3(v: [f32; 3]) -> V3 {
    [widen(v[0]), widen(v[1]), widen(v[2])]
}

pub fn widen4(v: [f32; 4]) -> [f64; 4] {
    [widen(v[0]), widen(v[1]), widen(v[2]), widen(v[3])]
}

/// CPython 3.12+ `sum()` over floats: int 0 start, then Neumaier-compensated accumulation.
pub fn psum(values: impl IntoIterator<Item = f64>) -> f64 {
    let mut it = values.into_iter();
    let Some(first) = it.next() else {
        return 0.0;
    };
    let mut f = 0.0 + first;
    let mut c = 0.0;
    for x in it {
        let t = f + x;
        if f.abs() >= x.abs() {
            c += (f - t) + x;
        } else {
            c += (x - t) + f;
        }
        f = t;
    }
    if c != 0.0 && c.is_finite() {
        f += c;
    }
    f
}

/// Python's `x or 1.0` on a float.
fn or_one(x: f64) -> f64 {
    if x == 0.0 {
        1.0
    } else {
        x
    }
}

pub fn mat_identity() -> M4 {
    [
        [1.0, 0.0, 0.0, 0.0],
        [0.0, 1.0, 0.0, 0.0],
        [0.0, 0.0, 1.0, 0.0],
        [0.0, 0.0, 0.0, 1.0],
    ]
}

pub fn mat_mul(a: &M4, b: &M4) -> M4 {
    let mut out = [[0.0; 4]; 4];
    for (i, row) in out.iter_mut().enumerate() {
        for (j, cell) in row.iter_mut().enumerate() {
            *cell = psum((0..4).map(|k| a[i][k] * b[k][j]));
        }
    }
    out
}

pub fn mat_vec(m: &M4, v: &V3, w: f64) -> V3 {
    let [x, y, z] = *v;
    [
        m[0][0] * x + m[0][1] * y + m[0][2] * z + m[0][3] * w,
        m[1][0] * x + m[1][1] * y + m[1][2] * z + m[1][3] * w,
        m[2][0] * x + m[2][1] * y + m[2][2] * z + m[2][3] * w,
    ]
}

pub fn mat_from_rt(r: &M3, t: &V3) -> M4 {
    [
        [r[0][0], r[0][1], r[0][2], t[0]],
        [r[1][0], r[1][1], r[1][2], t[1]],
        [r[2][0], r[2][1], r[2][2], t[2]],
        [0.0, 0.0, 0.0, 1.0],
    ]
}

pub fn mat_rot3(m: &M4) -> M3 {
    [
        [m[0][0], m[0][1], m[0][2]],
        [m[1][0], m[1][1], m[1][2]],
        [m[2][0], m[2][1], m[2][2]],
    ]
}

pub fn mat_trans(m: &M4) -> V3 {
    [m[0][3], m[1][3], m[2][3]]
}

/// Inverse of a rotation plus translation (orthonormal 3x3).
pub fn mat_inverse_rigid(m: &M4) -> M4 {
    let rt = r3_t(&mat_rot3(m));
    let t = mat_trans(m);
    let nt = [0, 1, 2].map(|i| -(rt[i][0] * t[0] + rt[i][1] * t[1] + rt[i][2] * t[2]));
    mat_from_rt(&rt, &nt)
}

/// General 4x4 inverse by Gauss-Jordan with partial pivoting; identity when singular.
pub fn mat_inverse(m: &M4) -> M4 {
    let mut a = [[0.0f64; 8]; 4];
    for i in 0..4 {
        a[i][..4].copy_from_slice(&m[i]);
        a[i][4 + i] = 1.0;
    }
    for c in 0..4 {
        let mut piv = c;
        for r in c + 1..4 {
            if a[r][c].abs() > a[piv][c].abs() {
                piv = r;
            }
        }
        if a[piv][c].abs() < 1e-12 {
            return mat_identity();
        }
        a.swap(c, piv);
        let p = a[c][c];
        for v in a[c].iter_mut() {
            *v /= p;
        }
        let pivot_row = a[c];
        for (r, row) in a.iter_mut().enumerate() {
            if r != c && row[c] != 0.0 {
                let f = row[c];
                for (rv, cv) in row.iter_mut().zip(pivot_row.iter()) {
                    *rv -= f * cv;
                }
            }
        }
    }
    let mut out = [[0.0; 4]; 4];
    for i in 0..4 {
        out[i].copy_from_slice(&a[i][4..]);
    }
    out
}

pub fn r3_mul(a: &M3, b: &M3) -> M3 {
    let mut out = [[0.0; 3]; 3];
    for (i, row) in out.iter_mut().enumerate() {
        for (j, cell) in row.iter_mut().enumerate() {
            *cell = psum((0..3).map(|k| a[i][k] * b[k][j]));
        }
    }
    out
}

pub fn r3_t(a: &M3) -> M3 {
    [
        [a[0][0], a[1][0], a[2][0]],
        [a[0][1], a[1][1], a[2][1]],
        [a[0][2], a[1][2], a[2][2]],
    ]
}

pub fn r3_vec(r: &M3, v: &V3) -> V3 {
    [0, 1, 2].map(|i| r[i][0] * v[0] + r[i][1] * v[1] + r[i][2] * v[2])
}

pub fn r3_normalize_columns(r: &M3) -> M3 {
    let mut out = [[0.0; 3]; 3];
    for j in 0..3 {
        let c = [r[0][j], r[1][j], r[2][j]];
        let l = or_one(sum_squares(&c).sqrt());
        for i in 0..3 {
            out[i][j] = c[i] / l;
        }
    }
    out
}

fn sum_squares(c: &[f64]) -> f64 {
    psum(c.iter().map(|x| x * x))
}

/// Rotation matrix of a quaternion (x, y, z, w), normalised first.
pub fn quat_to_r3(q: &[f64; 4]) -> M3 {
    let [x, y, z, w] = *q;
    let n = or_one((x * x + y * y + z * z + w * w).sqrt());
    let (x, y, z, w) = (x / n, y / n, z / n, w / n);
    [
        [
            1.0 - 2.0 * (y * y + z * z),
            2.0 * (x * y - z * w),
            2.0 * (x * z + y * w),
        ],
        [
            2.0 * (x * y + z * w),
            1.0 - 2.0 * (x * x + z * z),
            2.0 * (y * z - x * w),
        ],
        [
            2.0 * (x * z - y * w),
            2.0 * (y * z + x * w),
            1.0 - 2.0 * (x * x + y * y),
        ],
    ]
}

/// Quaternion (x, y, z, w) of a rotation matrix, normalised.
pub fn r3_to_quat(m: &M3) -> [f64; 4] {
    let t = m[0][0] + m[1][1] + m[2][2];
    let (x, y, z, w);
    if t > 0.0 {
        let s = (t + 1.0).sqrt() * 2.0;
        w = 0.25 * s;
        x = (m[2][1] - m[1][2]) / s;
        y = (m[0][2] - m[2][0]) / s;
        z = (m[1][0] - m[0][1]) / s;
    } else if m[0][0] > m[1][1] && m[0][0] > m[2][2] {
        let s = (1.0 + m[0][0] - m[1][1] - m[2][2]).sqrt() * 2.0;
        w = (m[2][1] - m[1][2]) / s;
        x = 0.25 * s;
        y = (m[0][1] + m[1][0]) / s;
        z = (m[0][2] + m[2][0]) / s;
    } else if m[1][1] > m[2][2] {
        let s = (1.0 + m[1][1] - m[0][0] - m[2][2]).sqrt() * 2.0;
        w = (m[0][2] - m[2][0]) / s;
        x = (m[0][1] + m[1][0]) / s;
        y = 0.25 * s;
        z = (m[1][2] + m[2][1]) / s;
    } else {
        let s = (1.0 + m[2][2] - m[0][0] - m[1][1]).sqrt() * 2.0;
        w = (m[1][0] - m[0][1]) / s;
        x = (m[0][2] + m[2][0]) / s;
        y = (m[1][2] + m[2][1]) / s;
        z = 0.25 * s;
    }
    let n = or_one((x * x + y * y + z * z + w * w).sqrt());
    [x / n, y / n, z / n, w / n]
}

/// Euler XYZ in radians with R = Rz * Ry * Rx, the Source SMD order.
pub fn r3_to_euler_xyz(m: &M3) -> V3 {
    let sy = -m[2][0];
    let sy = if sy < 1.0 { sy } else { 1.0 };
    let sy = if sy > -1.0 { sy } else { -1.0 };
    let y = sy.asin();
    if sy.abs() < 0.9999999 {
        [m[2][1].atan2(m[2][2]), y, m[1][0].atan2(m[0][0])]
    } else {
        [(-m[1][2]).atan2(m[1][1]), y, 0.0]
    }
}

pub fn vsub(a: &V3, b: &V3) -> V3 {
    [a[0] - b[0], a[1] - b[1], a[2] - b[2]]
}

pub fn vadd(a: &V3, b: &V3) -> V3 {
    [a[0] + b[0], a[1] + b[1], a[2] + b[2]]
}

pub fn vscale(a: &V3, s: f64) -> V3 {
    [a[0] * s, a[1] * s, a[2] * s]
}

pub fn vlen(a: &V3) -> f64 {
    sum_squares(a).sqrt()
}

pub fn vnorm(a: &V3) -> V3 {
    let l = vlen(a);
    if l > 1e-12 {
        [a[0] / l, a[1] / l, a[2] / l]
    } else {
        [0.0, 0.0, 1.0]
    }
}

pub fn vdot(a: &V3, b: &V3) -> f64 {
    psum((0..3).map(|i| a[i] * b[i]))
}

/// Fixed-point text with trailing zeros and a bare point removed; negative zero prints as "0".
pub fn fmt(v: f64, nd: usize) -> String {
    let mut s = format!("{v:.nd$}");
    if s.contains('.') {
        let trimmed = s.trim_end_matches('0').trim_end_matches('.').len();
        s.truncate(trimmed);
    }
    if s == "-0" || s.is_empty() {
        "0".to_string()
    } else {
        s
    }
}

/// The sixteen entries of a matrix, row by row, through [`fmt`].
pub fn mat_text(m: &M4, nd: usize) -> String {
    let mut parts = Vec::with_capacity(16);
    for row in m {
        for v in row {
            parts.push(fmt(*v, nd));
        }
    }
    parts.join(" ")
}

/// Identifier-safe name: alphanumerics and `_-.` kept, everything else `_`, never empty.
pub fn safe_id(s: &str) -> String {
    let out: String = s
        .chars()
        .map(|c| {
            if c.is_alphanumeric() || matches!(c, '_' | '-' | '.') {
                c
            } else {
                '_'
            }
        })
        .collect();
    if out.is_empty() {
        "x".to_string()
    } else {
        out
    }
}

/// Python's `str.title()`: first cased letter of each run upper, the rest lower.
pub fn title_case(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    let mut prev_cased = false;
    for c in s.chars() {
        if prev_cased {
            out.extend(c.to_lowercase());
        } else {
            out.extend(c.to_uppercase());
        }
        prev_cased = c.is_lowercase() || c.is_uppercase();
    }
    out
}
