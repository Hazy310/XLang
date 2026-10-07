// vector.x —— 向量运算（并入 std.math，向量用 [x, y] 或 [x, y, z] 数组表示）
// 点积 / 叉积 / 模长 / 单位化 / 加减 / 数乘 / 夹角

// 点积（任意维）
fn vdot(a, b) {
    let s = 0.0;
    for i in 0..len(a) {
        s = s + a[i] * b[i];
    }
    return s;
}

// 叉积（三维）
fn vcross(a, b) {
    return [a[1] * b[2] - a[2] * b[1], a[2] * b[0] - a[0] * b[2], a[0] * b[1] - a[1] * b[0]];
}

// 模长
fn vnorm(a) {
    return std.math.sqrt(std.math.vdot(a, a));
}

// 单位向量（零向量原样返回）
fn vnormalize(a) {
    let n = std.math.vnorm(a);
    if n == 0 {
        return a;
    }
    let out = [];
    for i in 0..len(a) {
        out = out + [a[i] / n];
    }
    return out;
}

// 向量加法（等长）
fn vadd(a, b) {
    let out = [];
    for i in 0..len(a) {
        out = out + [a[i] + b[i]];
    }
    return out;
}

// 向量减法（等长）
fn vsub(a, b) {
    let out = [];
    for i in 0..len(a) {
        out = out + [a[i] - b[i]];
    }
    return out;
}

// 数乘
fn vscale(a, k) {
    let out = [];
    for i in 0..len(a) {
        out = out + [a[i] * k];
    }
    return out;
}

// 夹角（弧度，余弦值截断到 [-1,1] 防浮点误差）
fn vangle(a, b) {
    let na = std.math.vnorm(a);
    let nb = std.math.vnorm(b);
    if na == 0 or nb == 0 {
        return 0;
    }
    let d = std.math.vdot(a, b) / (na * nb);
    let dc = d;
    if dc > 1.0 {
        dc = 1.0;
    }
    if dc < -1.0 {
        dc = -1.0;
    }
    return std.math.acos(dc);
}
