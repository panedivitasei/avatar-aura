//! Ports the glm 1.0 scalar paths avatar_export.cpp leans on: mat3/mat4 products, inverses and quaternion casts.
//! Every operation keeps glm's evaluation order so baked floats match the C++ bit for bit.

use glam::{Vec2, Vec3, Vec4};

/// glm::dot for vec2.
pub fn dot2(a: Vec2, b: Vec2) -> f32 {
    a.x * b.x + a.y * b.y
}

/// glm::dot for vec3: `(x + y) + z`.
pub fn dot3(a: Vec3, b: Vec3) -> f32 {
    a.x * b.x + a.y * b.y + a.z * b.z
}

pub fn length2(v: Vec2) -> f32 {
    dot2(v, v).sqrt()
}

pub fn length3(v: Vec3) -> f32 {
    dot3(v, v).sqrt()
}

/// glm::normalize: multiplies by `1 / sqrt(dot)`.
pub fn normalize3(v: Vec3) -> Vec3 {
    v * (1.0 / dot3(v, v).sqrt())
}

pub fn cross3(x: Vec3, y: Vec3) -> Vec3 {
    Vec3::new(
        x.y * y.z - y.y * x.z,
        x.z * y.x - y.z * x.x,
        x.x * y.y - y.x * x.y,
    )
}

/// glm::mix: `x * (1 - a) + y * a`.
pub fn mix3(x: Vec3, y: Vec3, a: f32) -> Vec3 {
    x * (1.0 - a) + y * a
}

pub fn mix4(x: Vec4, y: Vec4, a: f32) -> Vec4 {
    x * (1.0 - a) + y * a
}

/// std::max / glm::max: `(a < b) ? b : a`.
pub fn fmax(a: f32, b: f32) -> f32 {
    if a < b {
        b
    } else {
        a
    }
}

/// std::min / glm::min: `(b < a) ? b : a`.
pub fn fmin(a: f32, b: f32) -> f32 {
    if b < a {
        b
    } else {
        a
    }
}

pub fn clampf(x: f32, lo: f32, hi: f32) -> f32 {
    fmin(fmax(x, lo), hi)
}

pub fn clamp3(v: Vec3, lo: f32, hi: f32) -> Vec3 {
    Vec3::new(clampf(v.x, lo, hi), clampf(v.y, lo, hi), clampf(v.z, lo, hi))
}

pub fn min2(a: Vec2, b: Vec2) -> Vec2 {
    Vec2::new(fmin(a.x, b.x), fmin(a.y, b.y))
}

pub fn max2(a: Vec2, b: Vec2) -> Vec2 {
    Vec2::new(fmax(a.x, b.x), fmax(a.y, b.y))
}

pub fn min3(a: Vec3, b: Vec3) -> Vec3 {
    Vec3::new(fmin(a.x, b.x), fmin(a.y, b.y), fmin(a.z, b.z))
}

pub fn max3(a: Vec3, b: Vec3) -> Vec3 {
    Vec3::new(fmax(a.x, b.x), fmax(a.y, b.y), fmax(a.z, b.z))
}

/// std::min({a, b, c}).
pub fn fmin3(a: f32, b: f32, c: f32) -> f32 {
    fmin(fmin(a, b), c)
}

/// std::max({a, b, c}).
pub fn fmax3(a: f32, b: f32, c: f32) -> f32 {
    fmax(fmax(a, b), c)
}

/// glm::radians: the double constant narrowed to float, then one multiply.
pub fn radians(deg: f32) -> f32 {
    const DEG_TO_RAD: f64 = 0.017_453_292_519_943_295;
    deg * DEG_TO_RAD as f32
}

/// Quaternion with glm's component meaning, stored by name.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Quat {
    pub x: f32,
    pub y: f32,
    pub z: f32,
    pub w: f32,
}

impl Default for Quat {
    fn default() -> Self {
        Quat::IDENTITY
    }
}

impl Quat {
    pub const IDENTITY: Quat = Quat {
        x: 0.0,
        y: 0.0,
        z: 0.0,
        w: 1.0,
    };

    pub fn wxyz(w: f32, x: f32, y: f32, z: f32) -> Self {
        Quat { x, y, z, w }
    }

    pub fn from_glam(q: glam::Quat) -> Self {
        Quat::wxyz(q.w, q.x, q.y, q.z)
    }

    /// glm::normalize(qua): length from `(ww + xx) + (yy + zz)`, identity when it is not positive.
    pub fn normalize(self) -> Self {
        let len = ((self.w * self.w + self.x * self.x) + (self.y * self.y + self.z * self.z)).sqrt();
        if len <= 0.0 {
            return Quat::IDENTITY;
        }
        let inv = 1.0 / len;
        Quat::wxyz(self.w * inv, self.x * inv, self.y * inv, self.z * inv)
    }

    /// glm::mat3_cast.
    pub fn to_mat3(self) -> Mat3 {
        let q = self;
        let qxx = q.x * q.x;
        let qyy = q.y * q.y;
        let qzz = q.z * q.z;
        let qxz = q.x * q.z;
        let qxy = q.x * q.y;
        let qyz = q.y * q.z;
        let qwx = q.w * q.x;
        let qwy = q.w * q.y;
        let qwz = q.w * q.z;
        Mat3([
            Vec3::new(1.0 - 2.0 * (qyy + qzz), 2.0 * (qxy + qwz), 2.0 * (qxz - qwy)),
            Vec3::new(2.0 * (qxy - qwz), 1.0 - 2.0 * (qxx + qzz), 2.0 * (qyz + qwx)),
            Vec3::new(2.0 * (qxz + qwy), 2.0 * (qyz - qwx), 1.0 - 2.0 * (qxx + qyy)),
        ])
    }

    /// glm::mat4_cast.
    pub fn to_mat4(self) -> Mat4 {
        Mat4::from_mat3(self.to_mat3())
    }
}

/// Column-major 3x3 matrix.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Mat3(pub [Vec3; 3]);

impl Mat3 {
    fn at(&self, c: usize, r: usize) -> f32 {
        self.0[c][r]
    }

    /// `(m0 * x + m1 * y) + m2 * z`.
    pub fn mul_vec3(&self, v: Vec3) -> Vec3 {
        self.0[0] * v.x + self.0[1] * v.y + self.0[2] * v.z
    }

    pub fn transpose(&self) -> Mat3 {
        let m = &self.0;
        Mat3([
            Vec3::new(m[0].x, m[1].x, m[2].x),
            Vec3::new(m[0].y, m[1].y, m[2].y),
            Vec3::new(m[0].z, m[1].z, m[2].z),
        ])
    }

    pub fn determinant(&self) -> f32 {
        let m = |c, r| self.at(c, r);
        m(0, 0) * (m(1, 1) * m(2, 2) - m(2, 1) * m(1, 2)) - m(1, 0) * (m(0, 1) * m(2, 2) - m(2, 1) * m(0, 2))
            + m(2, 0) * (m(0, 1) * m(1, 2) - m(1, 1) * m(0, 2))
    }

    /// glm inv3x3, unaligned path.
    pub fn inverse(&self) -> Mat3 {
        let m = |c, r| self.at(c, r);
        let one_over = 1.0 / self.determinant();
        let mut inv = [Vec3::ZERO; 3];
        inv[0].x = m(1, 1) * m(2, 2) - m(2, 1) * m(1, 2);
        inv[1].x = -(m(1, 0) * m(2, 2) - m(2, 0) * m(1, 2));
        inv[2].x = m(1, 0) * m(2, 1) - m(2, 0) * m(1, 1);
        inv[0].y = -(m(0, 1) * m(2, 2) - m(2, 1) * m(0, 2));
        inv[1].y = m(0, 0) * m(2, 2) - m(2, 0) * m(0, 2);
        inv[2].y = -(m(0, 0) * m(2, 1) - m(2, 0) * m(0, 1));
        inv[0].z = m(0, 1) * m(1, 2) - m(1, 1) * m(0, 2);
        inv[1].z = -(m(0, 0) * m(1, 2) - m(1, 0) * m(0, 2));
        inv[2].z = m(0, 0) * m(1, 1) - m(1, 0) * m(0, 1);
        Mat3([inv[0] * one_over, inv[1] * one_over, inv[2] * one_over])
    }

    /// glm::quat_cast(mat3).
    pub fn to_quat(&self) -> Quat {
        let m = |c, r| self.at(c, r);
        let four_x = m(0, 0) - m(1, 1) - m(2, 2);
        let four_y = m(1, 1) - m(0, 0) - m(2, 2);
        let four_z = m(2, 2) - m(0, 0) - m(1, 1);
        let four_w = m(0, 0) + m(1, 1) + m(2, 2);
        let mut index = 0;
        let mut biggest = four_w;
        if four_x > biggest {
            biggest = four_x;
            index = 1;
        }
        if four_y > biggest {
            biggest = four_y;
            index = 2;
        }
        if four_z > biggest {
            biggest = four_z;
            index = 3;
        }
        let val = (biggest + 1.0).sqrt() * 0.5;
        let mult = 0.25 / val;
        match index {
            0 => Quat::wxyz(
                val,
                (m(1, 2) - m(2, 1)) * mult,
                (m(2, 0) - m(0, 2)) * mult,
                (m(0, 1) - m(1, 0)) * mult,
            ),
            1 => Quat::wxyz(
                (m(1, 2) - m(2, 1)) * mult,
                val,
                (m(0, 1) + m(1, 0)) * mult,
                (m(2, 0) + m(0, 2)) * mult,
            ),
            2 => Quat::wxyz(
                (m(2, 0) - m(0, 2)) * mult,
                (m(0, 1) + m(1, 0)) * mult,
                val,
                (m(1, 2) + m(2, 1)) * mult,
            ),
            _ => Quat::wxyz(
                (m(0, 1) - m(1, 0)) * mult,
                (m(2, 0) + m(0, 2)) * mult,
                (m(1, 2) + m(2, 1)) * mult,
                val,
            ),
        }
    }
}

/// Column-major 4x4 matrix.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Mat4(pub [Vec4; 4]);

impl Default for Mat4 {
    fn default() -> Self {
        Mat4::IDENTITY
    }
}

impl Mat4 {
    pub const IDENTITY: Mat4 = Mat4([Vec4::X, Vec4::Y, Vec4::Z, Vec4::W]);

    pub fn from_mat3(m: Mat3) -> Mat4 {
        Mat4([
            m.0[0].extend(0.0),
            m.0[1].extend(0.0),
            m.0[2].extend(0.0),
            Vec4::W,
        ])
    }

    pub fn to_mat3(&self) -> Mat3 {
        Mat3([self.0[0].truncate(), self.0[1].truncate(), self.0[2].truncate()])
    }

    pub fn col3(&self, c: usize) -> Vec3 {
        self.0[c].truncate()
    }

    /// glm mul4x4, unaligned path: each column accumulates left to right.
    pub fn mul(&self, b: &Mat4) -> Mat4 {
        let a = &self.0;
        let mut out = [Vec4::ZERO; 4];
        for (o, src) in out.iter_mut().zip(b.0.iter()) {
            let mut t = a[0] * src.x;
            t += a[1] * src.y;
            t += a[2] * src.z;
            t += a[3] * src.w;
            *o = t;
        }
        Mat4(out)
    }

    /// glm mat4 * vec4: `(m0 v0 + m1 v1) + (m2 v2 + m3 v3)`.
    pub fn mul_vec4(&self, v: Vec4) -> Vec4 {
        let m = &self.0;
        (m[0] * v.x + m[1] * v.y) + (m[2] * v.z + m[3] * v.w)
    }

    pub fn mul_scalar(&self, s: f32) -> Mat4 {
        Mat4([self.0[0] * s, self.0[1] * s, self.0[2] * s, self.0[3] * s])
    }

    /// glm::translate(m, v).
    pub fn translate(&self, v: Vec3) -> Mat4 {
        let m = &self.0;
        let mut r = *self;
        r.0[3] = m[0] * v.x + m[1] * v.y + m[2] * v.z + m[3];
        r
    }

    /// glm::scale(m, v).
    pub fn scale(&self, v: Vec3) -> Mat4 {
        let m = &self.0;
        Mat4([m[0] * v.x, m[1] * v.y, m[2] * v.z, m[3]])
    }

    /// glm::rotate(m, angle, axis).
    pub fn rotate(&self, angle: f32, v: Vec3) -> Mat4 {
        let c = angle.cos();
        let s = angle.sin();
        let axis = normalize3(v);
        let temp = axis * (1.0 - c);
        let r00 = c + temp.x * axis.x;
        let r01 = temp.x * axis.y + s * axis.z;
        let r02 = temp.x * axis.z - s * axis.y;
        let r10 = temp.y * axis.x - s * axis.z;
        let r11 = c + temp.y * axis.y;
        let r12 = temp.y * axis.z + s * axis.x;
        let r20 = temp.z * axis.x + s * axis.y;
        let r21 = temp.z * axis.y - s * axis.x;
        let r22 = c + temp.z * axis.z;
        let m = &self.0;
        Mat4([
            m[0] * r00 + m[1] * r01 + m[2] * r02,
            m[0] * r10 + m[1] * r11 + m[2] * r12,
            m[0] * r20 + m[1] * r21 + m[2] * r22,
            m[3],
        ])
    }

    /// glm compute_inverse<4, 4>.
    pub fn inverse(&self) -> Mat4 {
        let m = |c: usize, r: usize| self.0[c][r];
        let coef00 = m(2, 2) * m(3, 3) - m(3, 2) * m(2, 3);
        let coef02 = m(1, 2) * m(3, 3) - m(3, 2) * m(1, 3);
        let coef03 = m(1, 2) * m(2, 3) - m(2, 2) * m(1, 3);
        let coef04 = m(2, 1) * m(3, 3) - m(3, 1) * m(2, 3);
        let coef06 = m(1, 1) * m(3, 3) - m(3, 1) * m(1, 3);
        let coef07 = m(1, 1) * m(2, 3) - m(2, 1) * m(1, 3);
        let coef08 = m(2, 1) * m(3, 2) - m(3, 1) * m(2, 2);
        let coef10 = m(1, 1) * m(3, 2) - m(3, 1) * m(1, 2);
        let coef11 = m(1, 1) * m(2, 2) - m(2, 1) * m(1, 2);
        let coef12 = m(2, 0) * m(3, 3) - m(3, 0) * m(2, 3);
        let coef14 = m(1, 0) * m(3, 3) - m(3, 0) * m(1, 3);
        let coef15 = m(1, 0) * m(2, 3) - m(2, 0) * m(1, 3);
        let coef16 = m(2, 0) * m(3, 2) - m(3, 0) * m(2, 2);
        let coef18 = m(1, 0) * m(3, 2) - m(3, 0) * m(1, 2);
        let coef19 = m(1, 0) * m(2, 2) - m(2, 0) * m(1, 2);
        let coef20 = m(2, 0) * m(3, 1) - m(3, 0) * m(2, 1);
        let coef22 = m(1, 0) * m(3, 1) - m(3, 0) * m(1, 1);
        let coef23 = m(1, 0) * m(2, 1) - m(2, 0) * m(1, 1);

        let fac0 = Vec4::new(coef00, coef00, coef02, coef03);
        let fac1 = Vec4::new(coef04, coef04, coef06, coef07);
        let fac2 = Vec4::new(coef08, coef08, coef10, coef11);
        let fac3 = Vec4::new(coef12, coef12, coef14, coef15);
        let fac4 = Vec4::new(coef16, coef16, coef18, coef19);
        let fac5 = Vec4::new(coef20, coef20, coef22, coef23);

        let vec0 = Vec4::new(m(1, 0), m(0, 0), m(0, 0), m(0, 0));
        let vec1 = Vec4::new(m(1, 1), m(0, 1), m(0, 1), m(0, 1));
        let vec2 = Vec4::new(m(1, 2), m(0, 2), m(0, 2), m(0, 2));
        let vec3 = Vec4::new(m(1, 3), m(0, 3), m(0, 3), m(0, 3));

        let inv0 = vec1 * fac0 - vec2 * fac1 + vec3 * fac2;
        let inv1 = vec0 * fac0 - vec2 * fac3 + vec3 * fac4;
        let inv2 = vec0 * fac1 - vec1 * fac3 + vec3 * fac5;
        let inv3 = vec0 * fac2 - vec1 * fac4 + vec2 * fac5;

        let sign_a = Vec4::new(1.0, -1.0, 1.0, -1.0);
        let sign_b = Vec4::new(-1.0, 1.0, -1.0, 1.0);
        let inverse = Mat4([inv0 * sign_a, inv1 * sign_b, inv2 * sign_a, inv3 * sign_b]);

        let row0 = Vec4::new(inverse.0[0].x, inverse.0[1].x, inverse.0[2].x, inverse.0[3].x);
        let dot0 = self.0[0] * row0;
        let dot1 = (dot0.x + dot0.y) + (dot0.z + dot0.w);
        let one_over = 1.0 / dot1;
        inverse.mul_scalar(one_over)
    }
}
