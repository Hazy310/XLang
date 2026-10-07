// complex.x —— 复数运算（并入 std.math，复数用 [实部, 虚部] 数组表示）
// 加减乘除 / 模长 / 共轭 / 辐角

// 加法
fn cadd(a, b) {
    return [a[0] + b[0], a[1] + b[1]];
}

// 减法
fn csub(a, b) {
    return [a[0] - b[0], a[1] - b[1]];
}

// 乘法：(a+bi)(c+di) = (ac-bd) + (ad+bc)i
fn cmul(a, b) {
    return [a[0] * b[0] - a[1] * b[1], a[0] * b[1] + a[1] * b[0]];
}

// 除法：(a+bi)/(c+di) = ((ac+bd) + (bc-ad)i) / (c²+d²)，除数为零返回 0
fn cdiv(a, b) {
    let denom = to_float(b[0] * b[0] + b[1] * b[1]);
    if denom == 0 {
        return 0;
    }
    return [to_float(a[0] * b[0] + a[1] * b[1]) / denom, to_float(a[1] * b[0] - a[0] * b[1]) / denom];
}

// 模长
fn cmod(a) {
    return std.math.sqrt(a[0] * a[0] + a[1] * a[1]);
}

// 共轭
fn cconj(a) {
    return [a[0], -a[1]];
}

// 辐角（弧度）
fn carg(a) {
    return std.math.atan2(a[1], a[0]);
}
