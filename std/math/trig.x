// trig.x —— 三角函数（并入 std.math 模块，import std.math 即可用 std.math.sin 等）
// 无内置三角库，用泰勒级数 + 角度归约到 [0, π/2] 实现

// 内部：a∈[0, π/2] 的泰勒正弦（5 项）
fn sin_reduced(a) {
    let x2 = a * a;
    let res = a;
    let term = a;
    for i in 1..6 {
        let d1 = to_float(2 * i);
        let d2 = to_float(2 * i + 1);
        term = term * x2 / d1 / d2;
        if i % 2 == 1 {
            res = res - term;
        } else {
            res = res + term;
        }
    }
    return res;
}

// 正弦：角度归约到 [0, π/2] 后泰勒
fn sin(x) {
    let a = x;
    while a >= 6.283185307179586 {
        a = a - 6.283185307179586;
    }
    while a < 0.0 {
        a = a + 6.283185307179586;
    }
    let sign = 1.0;
    if a > 3.141592653589793 {
        a = 6.283185307179586 - a;
        sign = -1.0;
    }
    if a > 1.5707963267948966 {
        a = 3.141592653589793 - a;
    }
    let r = std.math.sin_reduced(a);
    return sign * r;
}

// 余弦
fn cos(x) {
    let a = x;
    while a >= 6.283185307179586 {
        a = a - 6.283185307179586;
    }
    while a < 0.0 {
        a = a + 6.283185307179586;
    }
    let sign = 1.0;
    if a > 3.141592653589793 {
        a = 6.283185307179586 - a;
    }
    if a > 1.5707963267948966 {
        a = 3.141592653589793 - a;
        sign = -1.0;
    }
    let x2 = a * a;
    let res = 1.0;
    let term = 1.0;
    for i in 1..6 {
        let d1 = to_float(2 * i - 1);
        let d2 = to_float(2 * i);
        term = term * x2 / d1 / d2;
        if i % 2 == 1 {
            res = res - term;
        } else {
            res = res + term;
        }
    }
    return sign * res;
}

// 正切：sin/cos，cos≈0 时返回 0
fn tan(x) {
    let c = std.math.cos(x);
    if std.math.abs(c) < 0.0000000001 {
        return 0;
    }
    return std.math.sin(x) / c;
}

// 余切
fn cot(x) {
    let s = std.math.sin(x);
    if std.math.abs(s) < 0.0000000001 {
        return 0;
    }
    return std.math.cos(x) / s;
}

// 内部：|x|≤1 的反正切泰勒（20 项）
fn atan_reduced(x) {
    let x2 = x * x;
    let res = 0.0;
    let term = x;
    let n = 0;
    while n < 20 {
        res = res + term / to_float(2 * n + 1);
        term = -term * x2;
        n = n + 1;
    }
    return res;
}

// 反正切：|x|>1 用 atan(x) = π/2 - atan(1/x) 变换
fn atan(x) {
    let neg = x < 0.0;
    let ax = x;
    if neg {
        ax = -ax;
    }
    // |x|>1：atan(x) = π/2 - atan(1/x)
    if ax > 1.0 {
        let r = 1.5707963267948966 - std.math.atan_reduced(1.0 / ax);
        if neg {
            return -r;
        }
        return r;
    }
    // 0.414 < |x| ≤ 1：加法公式 atan(x) = π/4 + atan((x-1)/(1+x))，把角度缩小使级数快速收敛
    if ax > 0.414213562373095 {
        let y = (ax - 1.0) / (1.0 + ax);
        let r = 0.7853981633974483 + std.math.atan_reduced(y);
        if neg {
            return -r;
        }
        return r;
    }
    let r = std.math.atan_reduced(ax);
    if neg {
        return -r;
    }
    return r;
}

// 反正弦：atan(x / sqrt(1 - x²))
fn asin(x) {
    if x < -1.0 or x > 1.0 {
        return 0;
    }
    if x == 1.0 {
        return 1.5707963267948966;
    }
    if x == -1.0 {
        return -1.5707963267948966;
    }
    return std.math.atan(x / std.math.sqrt(1.0 - x * x));
}

// 反余弦
fn acos(x) {
    return 1.5707963267948966 - std.math.asin(x);
}

// 双参数反正切 atan2(y, x)，返回 [-π, π]
fn atan2(y, x) {
    if x > 0.0 {
        return std.math.atan(y / x);
    }
    if x < 0.0 {
        if y >= 0.0 {
            return std.math.atan(y / x) + 3.141592653589793;
        }
        return std.math.atan(y / x) - 3.141592653589793;
    }
    if y > 0.0 {
        return 1.5707963267948966;
    }
    if y < 0.0 {
        return -1.5707963267948966;
    }
    return 0;
}

// 角度 <-> 弧度
fn deg2rad(d) {
    return d * 3.141592653589793 / 180.0;
}

fn rad2deg(r) {
    return r * 180.0 / 3.141592653589793;
}
