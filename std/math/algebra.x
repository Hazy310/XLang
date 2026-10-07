// algebra.x —— 初等代数（并入 std.math，import std.math 即可用）
// 二次方程求根 / 多项式求值、加法、乘法、求导；多项式系数从最高次到常数排列

// 解二次方程 ax²+bx+c=0：返回根数组；a=0 退化为一次；判别式<0 返回 0
fn solve_quadratic(a, b, c) {
    if a == 0 {
        if b == 0 {
            return 0;
        }
        return [-to_float(c) / to_float(b)];
    }
    let d = to_float(b) * to_float(b) - 4.0 * to_float(a) * to_float(c);
    if d < 0 {
        return 0;
    }
    let sq = std.math.sqrt(d);
    return [(-to_float(b) + sq) / (2.0 * to_float(a)), (-to_float(b) - sq) / (2.0 * to_float(a))];
}

// 多项式求值：Horner 法，coeffs 从最高次到常数
fn poly_eval(coeffs, x) {
    let r = to_float(coeffs[0]);
    for i in 1..len(coeffs) {
        r = r * x + to_float(coeffs[i]);
    }
    return r;
}

// 多项式加法：短的前面补 0 对齐次数后逐项相加
fn poly_add(p, q) {
    let a = p;
    let b = q;
    while len(a) < len(b) {
        a = [0] + a;
    }
    while len(b) < len(a) {
        b = [0] + b;
    }
    let out = [];
    for i in 0..len(a) {
        out = out + [a[i] + b[i]];
    }
    return out;
}

// 多项式乘法：卷积，结果长度 len(p)+len(q)-1
fn poly_mul(p, q) {
    let np = len(p);
    let nq = len(q);
    let r = [];
    for i in 0..np + nq - 1 {
        r = r + [0];
    }
    for i in 0..np {
        for j in 0..nq {
            r[i + j] = r[i + j] + p[i] * q[j];
        }
    }
    return r;
}

// 多项式求导：[(n-1)p0, (n-2)p1, ..., 1·p(n-2)]；长度<=1 返回空数组
fn poly_derivative(p) {
    let n = len(p);
    if n <= 1 {
        return [];
    }
    let out = [];
    for i in 0..n - 1 {
        out = out + [to_float(n - 1 - i) * p[i]];
    }
    return out;
}
