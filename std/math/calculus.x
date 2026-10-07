// calculus.x —— 微积分（并入 std.math，import std.math 即可用）
// exp/ln 为全模块共享基础函数；另含数值微分与数值积分

// 自然指数 e^x：对半缩放自变量，16 项泰勒展开，再逐次平方还原
fn exp(x) {
    let k = 0;
    let xr = to_float(x);
    while xr > 1.0 or xr < -1.0 {
        xr = xr / 2.0;
        k = k + 1;
    }
    let res = 1.0;
    let term = 1.0;
    for i in 1..16 {
        term = term * xr / to_float(i);
        res = res + term;
    }
    while k > 0 {
        res = res * res;
        k = k - 1;
    }
    return res;
}

// 自然对数 ln(x)：x<=0 返回 0；先归一化到 [1,2)，再牛顿迭代 + 2 的幂补偿
fn ln(x) {
    if x <= 0 {
        return 0;
    }
    let n = 0;
    let xr = to_float(x);
    while xr >= 2.0 {
        xr = xr / 2.0;
        n = n + 1;
    }
    while xr < 1.0 {
        xr = xr * 2.0;
        n = n - 1;
    }
    let y = xr - 1.0;
    for i in 0..30 {
        y = y - 1.0 + xr / std.math.exp(y);
    }
    return y + to_float(n) * 0.6931471805599453;
}

// 导数：中心差分 (f(x+h)-f(x-h))/(2h)
fn derivative(f, x, h) {
    return (f(x + h) - f(x - h)) / (2.0 * h);
}

// 定积分：梯形法，把 [a,b] 均分为 n 段
fn integrate(f, a, b, n) {
    let h = (to_float(b) - to_float(a)) / to_float(n);
    let s = 0.0;
    for i in 0..n {
        let xi = to_float(a) + to_float(i) * h;
        let xj = to_float(a) + to_float(i + 1) * h;
        s = s + (f(xi) + f(xj)) / 2.0 * h;
    }
    return s;
}

// 定积分：辛普森法（n 强制为偶数，奇数则减 1），奇数步 4 倍、偶数步 2 倍
fn integrate_simpson(f, a, b, n) {
    if n % 2 == 1 {
        n = n - 1;
    }
    let h = (to_float(b) - to_float(a)) / to_float(n);
    let s = f(to_float(a)) + f(to_float(b));
    for i in 1..n {
        let xi = to_float(a) + to_float(i) * h;
        if i % 2 == 1 {
            s = s + 4.0 * f(xi);
        } else {
            s = s + 2.0 * f(xi);
        }
    }
    return s * h / 3.0;
}
