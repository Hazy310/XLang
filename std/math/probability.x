// probability.x —— 概率分布与期望（并入 std.math，import std.math 即可用）
// 组合数复用 math.x 的 factorial，幂复用 math.x 的 pow

// 二项分布概率质量：C(n,k)·p^k·(1-p)^(n-k)，n<k 或 k<0 返回 0
fn binomial(n, k, p) {
    if n < k or k < 0 {
        return 0;
    }
    let c = to_float(std.math.factorial(n) / (std.math.factorial(k) * std.math.factorial(n - k)));
    return c * std.math.pow(p, k) * std.math.pow(1.0 - p, n - k);
}

// 泊松分布概率质量：e^(-λ)·λ^k/k!，k<0 返回 0
fn poisson(lam, k) {
    if k < 0 {
        return 0;
    }
    return std.math.exp(-lam) * std.math.pow(lam, k) / to_float(std.math.factorial(k));
}

// 几何分布概率质量：(1-p)^(k-1)·p（首次成功在第 k 次试验），k<1 返回 0
fn geometric(p, k) {
    if k < 1 {
        return 0;
    }
    return std.math.pow(1.0 - p, k - 1) * p;
}

// 正态分布概率密度：sigma<=0 返回 0
fn normal_pdf(x, mu, sigma) {
    if sigma <= 0 {
        return 0;
    }
    let z = (to_float(x) - to_float(mu)) / to_float(sigma);
    return std.math.exp(-z * z / 2.0) / (to_float(sigma) * std.math.sqrt(2.0 * 3.141592653589793));
}

// 离散期望：Σ xs[i]·ps[i]（两数组等长）
fn expected(xs, ps) {
    let s = 0.0;
    for i in 0..len(xs) {
        s = s + to_float(xs[i]) * to_float(ps[i]);
    }
    return s;
}
