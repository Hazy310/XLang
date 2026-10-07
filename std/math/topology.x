// topology.x —— 距离与度量（并入 std.math，点用数值数组表示，点集为点数组的数组）
// 欧氏 / 曼哈顿 / 切比雪夫 / Hausdorff / 闭球内判定

// n 维欧氏距离 sqrt(Σ(a[i]-b[i])²)
fn euclid_dist(a, b) {
    let s = 0.0;
    for i in 0..len(a) {
        let d = a[i] - b[i];
        s = s + d * d;
    }
    return std.math.sqrt(s);
}

// 曼哈顿距离 Σ|a[i]-b[i]|
fn manhattan_dist(a, b) {
    let s = 0.0;
    for i in 0..len(a) {
        s = s + std.math.abs(a[i] - b[i]);
    }
    return s;
}

// 切比雪夫距离 max|a[i]-b[i]|
fn chebyshev_dist(a, b) {
    let m = 0.0;
    for i in 0..len(a) {
        let d = std.math.abs(a[i] - b[i]);
        if d > m {
            m = d;
        }
    }
    return m;
}

// Hausdorff 距离（单向）：max over a∈A of min over b∈B of euclid_dist(a,b)，空集返回 0
fn hausdorff_dist(set_a, set_b) {
    if len(set_a) == 0 or len(set_b) == 0 {
        return 0;
    }
    let maxd = 0.0;
    for i in 0..len(set_a) {
        let a = set_a[i];
        let mind = -1.0;
        for j in 0..len(set_b) {
            let b = set_b[j];
            let d = std.math.euclid_dist(a, b);
            if mind < 0.0 or d < mind {
                mind = d;
            }
        }
        if mind > maxd {
            maxd = mind;
        }
    }
    return maxd;
}

// 点 p 是否在以 center 为心、半径 r 的闭球内：euclid_dist(p,center)<=r 返回 1 否则 0
fn in_ball(p, center, r) {
    if std.math.euclid_dist(p, center) <= r {
        return 1;
    }
    return 0;
}
