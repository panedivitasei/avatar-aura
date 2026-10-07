// Checks avatar_aura/ae_convert.py helper ports against values printed by CPython 3.14 (repr, bit exact).

use avatar_export::math::*;

const Q: [f64; 4] = [0.1, -0.7, 0.3, 0.6];

#[test]
fn quat_to_r3_matches_python() {
    let expected = [
        [-0.2210526315789476, -0.5263157894736843, -0.8210526315789475],
        [0.23157894736842108, 0.7894736842105263, -0.568421052631579],
        [0.9473684210526316, -0.3157894736842105, -0.052631578947368585],
    ];
    assert_eq!(quat_to_r3(&Q), expected);
}

#[test]
fn r3_to_quat_matches_python_on_every_branch() {
    assert_eq!(r3_to_quat(&FRAME_XBOX), [0.0, 0.0, 0.0, 1.0]);
    let cases = [
        (
            [1.0, 0.01, 0.02, 0.05],
            [
                0.9985033665845888,
                0.009985033665845891,
                0.019970067331691783,
                0.04992516832922945,
            ],
        ),
        (
            [0.02, 1.0, 0.01, 0.03],
            [
                0.019986014682870992,
                0.9993007341435493,
                0.009993007341435496,
                0.029979022024306485,
            ],
        ),
        (
            [0.01, 0.02, 1.0, 0.04],
            [
                0.009989516508612455,
                0.01997903301722491,
                0.9989516508612454,
                0.03995806603444982,
            ],
        ),
    ];
    for (q, expected) in cases {
        assert_eq!(r3_to_quat(&quat_to_r3(&q)), expected);
    }
}

#[test]
fn euler_matches_python() {
    assert_eq!(
        r3_to_euler_xyz(&quat_to_r3(&Q)),
        [-1.735945004209524, -1.2449133695738581, 2.3329428673818824]
    );
    let h = 0.5f64.sqrt();
    let gimbal = r3_to_euler_xyz(&quat_to_r3(&[0.0, h, 0.0, h]));
    assert_eq!(gimbal, [0.0, std::f64::consts::FRAC_PI_2, 0.0]);
    assert!(gimbal[0].is_sign_negative());
}

#[test]
fn inverses_match_python() {
    let m = [
        [2.0, 0.5, 0.1, 1.0],
        [0.3, 1.5, -0.2, 2.0],
        [0.0, 0.4, 3.0, -1.0],
        [0.0, 0.0, 0.0, 1.0],
    ];
    let inv = mat_inverse(&m);
    assert_eq!(
        inv,
        [
            [
                0.5251089199724833,
                -0.16739279981655583,
                -0.028663150653519837,
                -0.21898647099289154
            ],
            [
                -0.1031873423526714,
                0.687915615684476,
                0.049300619124054125,
                -1.2233432698922266
            ],
            [
                0.01375831231368952,
                -0.09172208209126347,
                0.3267599174501261,
                0.49644576931896356
            ],
            [0.0, 0.0, 0.0, 1.0],
        ]
    );
    assert_eq!(
        mat_mul(&m, &inv),
        [
            [
                0.9999999999999999,
                -5.204170427930421e-18,
                3.469446951953614e-18,
                -2.7755575615628914e-17
            ],
            [
                1.5178830414797062e-17,
                0.9999999999999999,
                8.673617379884035e-18,
                -1.3877787807814457e-16
            ],
            [0.0, 5.551115123125783e-17, 1.0, -1.1102230246251565e-16],
            [0.0, 0.0, 0.0, 1.0],
        ]
    );
    assert_eq!(
        mat_inverse_rigid(&mat_from_rt(&quat_to_r3(&Q), &[1.0, 2.0, 3.0])),
        [
            [
                -0.2210526315789476,
                0.23157894736842108,
                0.9473684210526316,
                -3.0842105263157893
            ],
            [
                -0.5263157894736843,
                0.7894736842105263,
                -0.3157894736842105,
                -0.10526315789473684
            ],
            [
                -0.8210526315789475,
                -0.568421052631579,
                -0.052631578947368585,
                2.1157894736842113
            ],
            [0.0, 0.0, 0.0, 1.0],
        ]
    );
    let singular = [[0.0; 4]; 4];
    assert_eq!(mat_inverse(&singular), mat_identity());
}

#[test]
fn sum_is_compensated_like_cpython() {
    assert_eq!(psum([0.1; 10]), 1.0);
    assert_eq!(psum([1e100, 1.0, -1e100]), 1.0);
    assert!(psum([-0.0, -0.0]).is_sign_positive());
    assert_eq!(psum([]), 0.0);
}

#[test]
fn text_helpers_match_python() {
    let cases = [
        (0.125, 2, "0.12"),
        (-0.00001, 4, "0"),
        (10.0, 3, "10"),
        (-1.23456789, 5, "-1.23457"),
        (2.5, 0, "2"),
        (1e-7, 6, "0"),
    ];
    for (v, nd, expected) in cases {
        assert_eq!(fmt(v, nd), expected, "{v} at {nd}");
    }
    assert_eq!(safe_id("Head 1/é(x)"), "Head_1_é_x_");
    assert_eq!(safe_id(""), "x");
    assert_eq!(title_case("eye left smile"), "Eye Left Smile");
    assert_eq!(title_case("o'neil x2y"), "O'Neil X2Y");
}

#[test]
fn widen_recovers_the_decimal_text() {
    assert_eq!(widen(0.07443f32), 0.07443);
    assert_eq!(widen(-0.00866f32), -0.00866);
    assert_eq!(widen(18.65625f32), 18.65625);
    assert_eq!(widen(1e-5f32), 0.00001);
}
