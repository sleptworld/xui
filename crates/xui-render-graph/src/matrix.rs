use crate::ColorMatrix;

pub(crate) const IDENTITY: ColorMatrix = [
    1.0, 0.0, 0.0, 0.0, 0.0, // R
    0.0, 1.0, 0.0, 0.0, 0.0, // G
    0.0, 0.0, 1.0, 0.0, 0.0, // B
    0.0, 0.0, 0.0, 1.0, 0.0, // A
];

pub(crate) fn compose(after: ColorMatrix, before: ColorMatrix) -> ColorMatrix {
    let mut result = [0.0; 20];
    for row in 0..4 {
        for column in 0..4 {
            result[row * 5 + column] = (0..4)
                .map(|inner| after[row * 5 + inner] * before[inner * 5 + column])
                .sum();
        }
        result[row * 5 + 4] = after[row * 5 + 4]
            + (0..4)
                .map(|inner| after[row * 5 + inner] * before[inner * 5 + 4])
                .sum::<f32>();
    }
    canonicalize_matrix(result)
}

/// Whether the matrix maps every colour in [0, 1]^4 to a colour in [0, 1]^4.
///
/// Every executor clamps after each colour matrix. Two matrices can be fused
/// into one only when the first never needs that clamp, because the fused
/// matrix is linear and clamps once at the end: `x2` then `x0.5` is the
/// identity fused, but clamped between, it takes 0.8 to 0.5.
///
/// The map is affine, so the image of the cube is the hull of its corners'
/// images, and checking the sixteen corners is exact.
pub(crate) fn preserves_unit_cube(matrix: &ColorMatrix) -> bool {
    const EPSILON: f32 = 1e-6;
    (0..16u32).all(|corner| {
        let input = [0, 1, 2, 3].map(|bit| ((corner >> bit) & 1) as f32);
        (0..4).all(|row| {
            let base = row * 5;
            let output = matrix[base + 4]
                + (0..4).map(|column| matrix[base + column] * input[column]).sum::<f32>();
            (-EPSILON..=1.0 + EPSILON).contains(&output)
        })
    })
}

pub(crate) fn is_identity(matrix: &ColorMatrix) -> bool {
    matrix == &IDENTITY
}

pub(crate) fn canonicalize_matrix(mut matrix: ColorMatrix) -> ColorMatrix {
    for value in &mut matrix {
        *value = canonical_zero(*value);
    }
    matrix
}

pub(crate) fn brightness(amount: f32) -> ColorMatrix {
    [
        amount, 0.0, 0.0, 0.0, 0.0, 0.0, amount, 0.0, 0.0, 0.0, 0.0, 0.0, amount, 0.0, 0.0, 0.0,
        0.0, 0.0, 1.0, 0.0,
    ]
}

pub(crate) fn contrast(amount: f32) -> ColorMatrix {
    let offset = 0.5 * (1.0 - amount);
    [
        amount, 0.0, 0.0, 0.0, offset, 0.0, amount, 0.0, 0.0, offset, 0.0, 0.0, amount, 0.0,
        offset, 0.0, 0.0, 0.0, 1.0, 0.0,
    ]
}

pub(crate) fn saturate(amount: f32) -> ColorMatrix {
    const R: f32 = 0.213;
    const G: f32 = 0.715;
    const B: f32 = 0.072;
    [
        R + (1.0 - R) * amount,
        G - G * amount,
        B - B * amount,
        0.0,
        0.0,
        R - R * amount,
        G + (1.0 - G) * amount,
        B - B * amount,
        0.0,
        0.0,
        R - R * amount,
        G - G * amount,
        B + (1.0 - B) * amount,
        0.0,
        0.0,
        0.0,
        0.0,
        0.0,
        1.0,
        0.0,
    ]
}

pub(crate) fn grayscale(amount: f32) -> ColorMatrix {
    saturate(1.0 - amount)
}

pub(crate) fn sepia(amount: f32) -> ColorMatrix {
    let full = [
        0.393, 0.769, 0.189, 0.0, 0.0, 0.349, 0.686, 0.168, 0.0, 0.0, 0.272, 0.534, 0.131, 0.0,
        0.0, 0.0, 0.0, 0.0, 1.0, 0.0,
    ];
    lerp(IDENTITY, full, amount)
}

pub(crate) fn invert(amount: f32) -> ColorMatrix {
    let scale = 1.0 - 2.0 * amount;
    [
        scale, 0.0, 0.0, 0.0, amount, 0.0, scale, 0.0, 0.0, amount, 0.0, 0.0, scale, 0.0, amount,
        0.0, 0.0, 0.0, 1.0, 0.0,
    ]
}

pub(crate) fn hue_rotate(radians: f32) -> ColorMatrix {
    let cosine = radians.cos();
    let sine = radians.sin();
    [
        0.213 + 0.787 * cosine - 0.213 * sine,
        0.715 - 0.715 * cosine - 0.715 * sine,
        0.072 - 0.072 * cosine + 0.928 * sine,
        0.0,
        0.0,
        0.213 - 0.213 * cosine + 0.143 * sine,
        0.715 + 0.285 * cosine + 0.140 * sine,
        0.072 - 0.072 * cosine - 0.283 * sine,
        0.0,
        0.0,
        0.213 - 0.213 * cosine - 0.787 * sine,
        0.715 - 0.715 * cosine + 0.715 * sine,
        0.072 + 0.928 * cosine + 0.072 * sine,
        0.0,
        0.0,
        0.0,
        0.0,
        0.0,
        1.0,
        0.0,
    ]
}

fn lerp(from: ColorMatrix, to: ColorMatrix, amount: f32) -> ColorMatrix {
    let mut result = [0.0; 20];
    for index in 0..20 {
        result[index] = from[index] + (to[index] - from[index]) * amount;
    }
    canonicalize_matrix(result)
}

pub(crate) fn canonical_zero(value: f32) -> f32 {
    if value == 0.0 { 0.0 } else { value }
}
