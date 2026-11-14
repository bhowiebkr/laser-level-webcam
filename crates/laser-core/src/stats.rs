//! Small robust statistics used by the beam finder and the monitor.

/// Median, or `None` for an empty slice.
pub fn median(values: &[f64]) -> Option<f64> {
    quantile(values, 0.5)
}

/// Quantile `q` (0 to 1) with linear interpolation, or `None` for an empty slice.
pub fn quantile(values: &[f64], q: f64) -> Option<f64> {
    if values.is_empty() {
        return None;
    }
    let mut v = values.to_vec();
    v.sort_by(f64::total_cmp);
    let pos = q.clamp(0.0, 1.0) * (v.len() - 1) as f64;
    let i = pos.floor() as usize;
    let j = (i + 1).min(v.len() - 1);
    let f = pos - i as f64;
    Some(v[i] + (v[j] - v[i]) * f)
}

/// Median absolute deviation, scaled to the standard deviation of a normal distribution.
pub fn mad_sigma(values: &[f64]) -> Option<f64> {
    let m = median(values)?;
    let dev: Vec<f64> = values.iter().map(|v| (v - m).abs()).collect();
    Some(1.4826 * median(&dev)?)
}

/// Noise of a series from its first differences: robust to slow trends and to a few jumps.
pub fn diff_noise(values: &[f64]) -> Option<f64> {
    if values.len() < 3 {
        return None;
    }
    let d: Vec<f64> = values.windows(2).map(|w| w[1] - w[0]).collect();
    Some(mad_sigma(&d)? / std::f64::consts::SQRT_2)
}

/// Least-squares line `y = a + b·x`. `None` when there are fewer than two points or no spread in x.
pub fn line_fit(x: &[f64], y: &[f64]) -> Option<(f64, f64)> {
    let n = x.len().min(y.len());
    if n < 2 {
        return None;
    }
    let mx = x[..n].iter().sum::<f64>() / n as f64;
    let my = y[..n].iter().sum::<f64>() / n as f64;
    let (mut sxx, mut sxy) = (0.0, 0.0);
    for (xi, yi) in x[..n].iter().zip(&y[..n]) {
        sxx += (xi - mx) * (xi - mx);
        sxy += (xi - mx) * (yi - my);
    }
    if sxx <= 0.0 {
        return None;
    }
    let b = sxy / sxx;
    Some((my - b * mx, b))
}

/// Theil–Sen slope: the median of the slopes of all pairs. Robust to outliers.
pub fn theil_sen(x: &[f64], y: &[f64]) -> Option<f64> {
    let n = x.len().min(y.len());
    let mut slopes = Vec::with_capacity(n * n.saturating_sub(1) / 2);
    for i in 0..n {
        for j in i + 1..n {
            let dx = x[j] - x[i];
            if dx != 0.0 {
                slopes.push((y[j] - y[i]) / dx);
            }
        }
    }
    median(&slopes)
}

/// Lag-1 autocorrelation of a series, or `None` when it is too short or flat.
pub fn lag1(values: &[f64]) -> Option<f64> {
    let n = values.len();
    if n < 4 {
        return None;
    }
    let m = values.iter().sum::<f64>() / n as f64;
    let var: f64 = values.iter().map(|v| (v - m) * (v - m)).sum();
    if var <= 0.0 {
        return None;
    }
    let cov: f64 = values.windows(2).map(|w| (w[0] - m) * (w[1] - m)).sum();
    Some(cov / var)
}

/// Factor by which correlated residuals make a variance larger than white residuals would:
/// `(1 + ρ) / (1 − ρ)` for lag-1 correlation ρ, at least 1.
pub fn correlation_inflation(residuals: &[f64]) -> f64 {
    let rho = lag1(residuals).unwrap_or(0.0).clamp(0.0, 0.95);
    (1.0 + rho) / (1.0 - rho)
}

/// Result of [`weighted_lsq`].
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Lsq<const K: usize> {
    pub coef: [f64; K],
    /// Inverse of the normal matrix: the covariance of `coef` when the weights are 1/σ².
    pub cov: [[f64; K]; K],
    /// Weighted sum of squared residuals.
    pub chi2: f64,
}

/// Weighted least squares for `y[i] ≈ rows[i] · coef` with weights `w[i]`. `None` when the
/// columns do not determine the coefficients.
pub fn weighted_lsq<const K: usize>(rows: &[[f64; K]], y: &[f64], w: &[f64]) -> Option<Lsq<K>> {
    let mut normal = [[0.0; K]; K];
    let mut rhs = [0.0; K];
    for ((row, &yi), &wi) in rows.iter().zip(y).zip(w) {
        for (i, &ri) in row.iter().enumerate() {
            rhs[i] += wi * ri * yi;
            for (j, &rj) in row.iter().enumerate() {
                normal[i][j] += wi * ri * rj;
            }
        }
    }
    let cov = invert(normal)?;
    let coef: [f64; K] = std::array::from_fn(|i| cov[i].iter().zip(&rhs).map(|(c, r)| c * r).sum());
    let chi2 = rows
        .iter()
        .zip(y)
        .zip(w)
        .map(|((row, yi), wi)| wi * (yi - row.iter().zip(&coef).map(|(r, c)| r * c).sum::<f64>()).powi(2))
        .sum();
    Some(Lsq { coef, cov, chi2 })
}

/// Gauss–Jordan inverse with partial pivoting. `None` when the matrix is singular.
#[allow(clippy::needless_range_loop)]
fn invert<const K: usize>(mut a: [[f64; K]; K]) -> Option<[[f64; K]; K]> {
    let scale = (0..K).map(|i| a[i][i].abs()).fold(0.0, f64::max);
    if scale <= 0.0 || !scale.is_finite() {
        return None;
    }
    let mut inv = [[0.0; K]; K];
    for i in 0..K {
        inv[i][i] = 1.0;
    }
    for c in 0..K {
        let p = (c..K).max_by(|&i, &j| a[i][c].abs().total_cmp(&a[j][c].abs()))?;
        if a[p][c].abs() <= 1e-12 * scale {
            return None;
        }
        a.swap(c, p);
        inv.swap(c, p);
        let d = a[c][c];
        for j in 0..K {
            a[c][j] /= d;
            inv[c][j] /= d;
        }
        for i in 0..K {
            let f = a[i][c];
            if i != c && f != 0.0 {
                for j in 0..K {
                    a[i][j] -= f * a[c][j];
                    inv[i][j] -= f * inv[c][j];
                }
            }
        }
    }
    Some(inv)
}

/// Standard normal cumulative distribution.
pub fn normal_cdf(x: f64) -> f64 {
    0.5 * (1.0 + erf(x / std::f64::consts::SQRT_2))
}

/// Error function, Abramowitz and Stegun 7.1.26 (error below 1.5e-7).
fn erf(x: f64) -> f64 {
    let t = 1.0 / (1.0 + 0.327_591_1 * x.abs());
    let poly =
        t * (0.254_829_592 + t * (-0.284_496_736 + t * (1.421_413_741 + t * (-1.453_152_027 + t * 1.061_405_429))));
    let y = 1.0 - poly * (-x * x).exp();
    if x < 0.0 { -y } else { y }
}

/// Expected range (largest minus smallest) of `n` independent standard normal values, the d2
/// constant of control charts: 1.128 for 2, 3.078 for 10.
pub fn expected_range(n: usize) -> f64 {
    if n < 2 {
        return 0.0;
    }
    let f = |x: f64| {
        let p = normal_cdf(x);
        1.0 - p.powi(n as i32) - (1.0 - p).powi(n as i32)
    };
    // Simpson's rule on ±10; the integrand is below 1e-20 outside it.
    let steps = 2000;
    let h = 20.0 / steps as f64;
    let mut sum = f(-10.0) + f(10.0);
    for k in 1..steps {
        sum += if k % 2 == 1 { 4.0 } else { 2.0 } * f(-10.0 + k as f64 * h);
    }
    sum * h / 3.0
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn expected_range_matches_the_table() {
        assert!((expected_range(2) - 1.128).abs() < 1e-3, "{}", expected_range(2));
        assert!((expected_range(10) - 3.078).abs() < 1e-3, "{}", expected_range(10));
        assert!((expected_range(25) - 3.931).abs() < 1e-3, "{}", expected_range(25));
    }

    #[test]
    fn lsq_plane_and_singular() {
        let pts = [(0.0, 0.0), (1.0, 0.0), (0.0, 1.0), (1.0, 1.0), (2.0, 1.0)];
        let rows: Vec<[f64; 3]> = pts.iter().map(|&(x, y)| [1.0, x, y]).collect();
        let z: Vec<f64> = pts.iter().map(|&(x, y)| 3.0 + 2.0 * x - y).collect();
        let fit = weighted_lsq(&rows, &z, &[1.0; 5]).unwrap();
        for (c, want) in fit.coef.iter().zip([3.0, 2.0, -1.0]) {
            assert!((c - want).abs() < 1e-12, "{fit:?}");
        }
        let line: Vec<[f64; 3]> = (0..4).map(|k| [1.0, f64::from(k), f64::from(k)]).collect();
        assert_eq!(weighted_lsq(&line, &[0.0; 4], &[1.0; 4]), None);
    }

    #[test]
    fn median_and_quantile() {
        assert_eq!(median(&[3.0, 1.0, 2.0]), Some(2.0));
        assert_eq!(median(&[4.0, 1.0, 2.0, 3.0]), Some(2.5));
        assert_eq!(quantile(&[0.0, 10.0], 0.1), Some(1.0));
        assert_eq!(median(&[]), None);
    }

    #[test]
    fn line_and_theil_sen() {
        let x = [0.0, 1.0, 2.0, 3.0, 4.0];
        let y = [1.0, 3.0, 5.0, 7.0, 100.0];
        let (a, b) = line_fit(&x[..4], &y[..4]).unwrap();
        assert!((a - 1.0).abs() < 1e-12 && (b - 2.0).abs() < 1e-12);
        assert_eq!(theil_sen(&x, &y), Some(2.0));
    }
}
