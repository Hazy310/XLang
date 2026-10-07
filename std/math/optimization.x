// optimization.x —— 一维优化：黄金分割求极小 / 极大、最小二乘线性回归（并入 std.math）

// 黄金分割法求 f 在 [a, b] 上的极小值点 x，eps 为区间宽度容差
fn golden_min(f, a, b, eps) {
    let g = 0.6180339887498949;
    let x1 = to_float(b) - g * (to_float(b) - to_float(a));
    let x2 = to_float(a) + g * (to_float(b) - to_float(a));
    let f1 = f(x1);
    let f2 = f(x2);
    while to_float(b) - to_float(a) > eps {
        if f1 < f2 {
            b = x2;
            x2 = x1;
            f2 = f1;
            x1 = to_float(b) - g * (to_float(b) - to_float(a));
            f1 = f(x1);
        } else {
            a = x1;
            x1 = x2;
            f1 = f2;
            x2 = to_float(a) + g * (to_float(b) - to_float(a));
            f2 = f(x2);
        }
    }
    return (to_float(a) + to_float(b)) / 2.0;
}

// 黄金分割法求 f 在 [a, b] 上的极大值点（转成 -f 的极小值）
fn golden_max(f, a, b, eps) {
    return std.math.golden_min(fn(x) { -f(x) }, a, b, eps);
}

// 最小二乘线性回归 y = a + b·x，返回 [斜率 b, 截距 a]，全 float
fn least_squares(xs, ys) {
    let n = len(xs);
    let sx = 0.0;
    let sy = 0.0;
    let sxx = 0.0;
    let sxy = 0.0;
    for i in 0..n {
        let xi = to_float(xs[i]);
        let yi = to_float(ys[i]);
        sx = sx + xi;
        sy = sy + yi;
        sxx = sxx + xi * xi;
        sxy = sxy + xi * yi;
    }
    let denom = to_float(n) * sxx - sx * sx;
    if denom == 0.0 {
        return [0, 0];
    }
    let b = (to_float(n) * sxy - sx * sy) / denom;
    let a = (sy - b * sx) / to_float(n);
    return [b, a];
}
