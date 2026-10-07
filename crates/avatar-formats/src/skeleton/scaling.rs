// Port of xenia/kernel/xam/avatars/skeleton_scaling.{h,cpp}.

use glam::Vec3;

use super::Skeleton;

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum BodyType {
    #[default]
    Unknown,
    Male,
    Female,
}

type ScaleTable = [(usize, [f32; 3]); 14];

#[rustfmt::skip]
const V1_MALE_HEAVY: ScaleTable = [
    (4, [1.6, 1.6, 2.2]),
    (7, [1.5, 1.0, 1.5]),
    (9, [1.5, 1.0, 1.5]),
    (10, [1.8, 1.0, 1.9]),
    (13, [1.5, 1.0, 1.5]),
    (17, [1.5, 1.0, 1.5]),
    (18, [1.5, 1.0, 1.4]),
    (24, [1.9, 1.0, 1.5]),
    (26, [1.0, 1.5, 1.5]),
    (27, [1.0, 1.5, 1.5]),
    (29, [1.0, 1.5, 1.5]),
    (30, [1.0, 1.5, 1.5]),
    (32, [1.0, 1.5, 1.5]),
    (35, [1.0, 1.5, 1.5]),
];

#[rustfmt::skip]
const V1_MALE_LIGHT: ScaleTable = [
    (4, [0.8, 1.0, 0.9]),
    (7, [0.7, 1.0, 0.7]),
    (9, [0.7, 1.0, 0.7]),
    (10, [0.6, 1.0, 0.6]),
    (13, [0.7, 1.0, 0.7]),
    (17, [0.7, 1.0, 0.7]),
    (18, [0.9, 1.0, 0.8]),
    (24, [0.7, 1.0, 0.7]),
    (26, [1.0, 0.7, 0.7]),
    (27, [1.0, 0.7, 0.7]),
    (29, [1.0, 0.7, 0.7]),
    (30, [1.0, 0.7, 0.7]),
    (32, [1.0, 0.7, 0.7]),
    (35, [1.0, 0.7, 0.7]),
];

#[rustfmt::skip]
const V1_FEMALE_HEAVY: ScaleTable = [
    (4, [1.5, 1.5, 2.0]),
    (7, [1.6, 1.0, 1.6]),
    (9, [1.6, 1.0, 1.6]),
    (10, [1.6, 1.0, 2.0]),
    (13, [1.6, 1.0, 1.6]),
    (17, [1.6, 1.0, 1.6]),
    (18, [1.6, 1.0, 1.6]),
    (24, [2.0, 1.0, 1.6]),
    (26, [1.0, 1.6, 1.6]),
    (27, [1.0, 1.6, 1.6]),
    (29, [1.0, 1.6, 1.6]),
    (30, [1.0, 1.6, 1.6]),
    (32, [1.0, 1.6, 1.6]),
    (35, [1.0, 1.6, 1.6]),
];

#[rustfmt::skip]
const V1_FEMALE_LIGHT: ScaleTable = [
    (4, [0.65, 1.0, 0.7]),
    (7, [0.7, 1.0, 0.7]),
    (9, [0.7, 1.0, 0.7]),
    (10, [0.7, 1.0, 0.5]),
    (13, [0.7, 1.0, 0.7]),
    (17, [0.7, 1.0, 0.7]),
    (18, [0.8, 1.0, 0.7]),
    (24, [0.7, 1.0, 0.7]),
    (26, [1.0, 0.7, 0.7]),
    (27, [1.0, 0.7, 0.7]),
    (29, [1.0, 0.7, 0.7]),
    (30, [1.0, 0.7, 0.7]),
    (32, [1.0, 0.7, 0.7]),
    (35, [1.0, 0.7, 0.7]),
];

#[rustfmt::skip]
const V2_MALE_HEAVY: ScaleTable = [
    (4, [1.6, 1.6, 2.2]),
    (7, [1.5, 1.0, 1.5]),
    (9, [1.5, 1.0, 1.5]),
    (10, [1.8, 1.0, 1.9]),
    (13, [1.5, 1.0, 1.5]),
    (17, [1.5, 1.0, 1.5]),
    (18, [1.5, 1.0, 1.4]),
    (24, [1.9, 1.0, 1.5]),
    (26, [1.0, 1.5, 1.5]),
    (27, [1.0, 1.5, 1.5]),
    (29, [1.0, 1.5, 1.5]),
    (30, [1.0, 1.5, 1.5]),
    (32, [1.0, 1.5, 1.5]),
    (35, [1.0, 1.5, 1.5]),
];

#[rustfmt::skip]
const V2_MALE_LIGHT: ScaleTable = [
    (4, [0.84, 1.0, 0.92]),
    (7, [0.76, 1.0, 0.76]),
    (9, [0.76, 1.0, 0.76]),
    (10, [0.68, 1.0, 0.68]),
    (13, [0.76, 1.0, 0.76]),
    (17, [0.76, 1.0, 0.76]),
    (18, [0.92, 1.0, 0.84]),
    (24, [0.76, 1.0, 0.76]),
    (26, [1.0, 0.76, 0.76]),
    (27, [1.0, 0.76, 0.76]),
    (29, [1.0, 0.76, 0.76]),
    (30, [1.0, 0.76, 0.76]),
    (32, [1.0, 0.76, 0.76]),
    (35, [1.0, 0.76, 0.76]),
];

#[rustfmt::skip]
const V2_FEMALE_HEAVY: ScaleTable = [
    (4, [1.5, 1.5, 2.0]),
    (7, [1.6, 1.0, 1.6]),
    (9, [1.6, 1.0, 1.6]),
    (10, [1.6, 1.0, 2.0]),
    (13, [1.6, 1.0, 1.6]),
    (17, [1.6, 1.0, 1.6]),
    (18, [1.6, 1.0, 1.6]),
    (24, [2.0, 1.0, 1.6]),
    (26, [1.0, 1.6, 1.6]),
    (27, [1.0, 1.6, 1.6]),
    (29, [1.0, 1.6, 1.6]),
    (30, [1.0, 1.6, 1.6]),
    (32, [1.0, 1.6, 1.6]),
    (35, [1.0, 1.6, 1.6]),
];

#[rustfmt::skip]
const V2_FEMALE_LIGHT: ScaleTable = [
    (4, [0.79, 1.0, 0.82]),
    (7, [0.82, 1.0, 0.82]),
    (9, [0.82, 1.0, 0.82]),
    (10, [0.82, 1.0, 0.7]),
    (13, [0.82, 1.0, 0.82]),
    (17, [0.82, 1.0, 0.82]),
    (18, [0.88, 1.0, 0.82]),
    (24, [0.82, 1.0, 0.82]),
    (26, [1.0, 0.82, 0.82]),
    (27, [1.0, 0.82, 0.82]),
    (29, [1.0, 0.82, 0.82]),
    (30, [1.0, 0.82, 0.82]),
    (32, [1.0, 0.82, 0.82]),
    (35, [1.0, 0.82, 0.82]),
];

struct WeightTables {
    male_heavy: &'static ScaleTable,
    male_light: &'static ScaleTable,
    female_heavy: &'static ScaleTable,
    female_light: &'static ScaleTable,
}

fn lerp(v1: Vec3, v2: Vec3, amount: f32) -> Vec3 {
    Vec3::new(
        v1.x + amount * (v2.x - v1.x),
        v1.y + amount * (v2.y - v1.y),
        v1.z + amount * (v2.z - v1.z),
    )
}

fn lerp_all(vectors: &mut [Vec3], factor: f32) {
    for v in vectors.iter_mut().rev() {
        *v = lerp(Vec3::ONE, *v, factor);
    }
}

fn default_scales() -> Vec<Vec3> {
    vec![Vec3::ONE; 72]
}

fn set(scales: &mut [Vec3], table: &ScaleTable) {
    for &(i, [x, y, z]) in table {
        scales[i] = Vec3::new(x, y, z);
    }
}

fn apply_scales(skeleton: &mut Skeleton, factor: f32, scales: &[Vec3]) {
    let factor = factor.clamp(0.0, 1.0);
    for (joint, &target) in skeleton.joints.iter_mut().zip(scales).rev() {
        let scale = lerp(Vec3::ONE, target, factor);
        joint.pose.scale.x *= scale.x;
        joint.pose.scale.y *= scale.y;
        joint.pose.scale.z *= scale.z;
    }
}

fn apply(
    tables: &WeightTables,
    body_type: BodyType,
    mut weight_factor: f32,
    mut height_factor: f32,
    skeleton: &mut Skeleton,
) {
    let mut scales = default_scales();
    if height_factor >= 0.0 {
        scales[0] = Vec3::splat(1.1);
        scales[19] = Vec3::splat(0.9);
    } else {
        scales[0] = Vec3::splat(0.9);
        scales[19] = Vec3::splat(1.05);
        height_factor = -height_factor;
    }
    apply_scales(skeleton, height_factor, &scales);

    let mut scales = default_scales();
    match body_type {
        BodyType::Male => {
            if weight_factor >= 0.0 {
                set(&mut scales, tables.male_heavy);
                lerp_all(&mut scales, 0.6);
            } else {
                set(&mut scales, tables.male_light);
                weight_factor = -weight_factor;
            }
        }
        BodyType::Female => {
            if weight_factor > 0.0 {
                set(&mut scales, tables.female_heavy);
                lerp_all(&mut scales, 0.6);
            } else {
                set(&mut scales, tables.female_light);
                weight_factor = -weight_factor;
            }
        }
        BodyType::Unknown => {}
    }
    apply_scales(skeleton, weight_factor, &scales);
}

/// Height and weight joint scaling, first body revision.
pub fn apply_scales_to_skeleton_v1(
    body_type: BodyType,
    weight_factor: f32,
    height_factor: f32,
    skeleton: &mut Skeleton,
) {
    let tables = WeightTables {
        male_heavy: &V1_MALE_HEAVY,
        male_light: &V1_MALE_LIGHT,
        female_heavy: &V1_FEMALE_HEAVY,
        female_light: &V1_FEMALE_LIGHT,
    };
    apply(&tables, body_type, weight_factor, height_factor, skeleton);
}

/// Height and weight joint scaling, second body revision (milder light-weight scales).
pub fn apply_scales_to_skeleton_v2(
    body_type: BodyType,
    weight_factor: f32,
    height_factor: f32,
    skeleton: &mut Skeleton,
) {
    let tables = WeightTables {
        male_heavy: &V2_MALE_HEAVY,
        male_light: &V2_MALE_LIGHT,
        female_heavy: &V2_FEMALE_HEAVY,
        female_light: &V2_FEMALE_LIGHT,
    };
    apply(&tables, body_type, weight_factor, height_factor, skeleton);
}
