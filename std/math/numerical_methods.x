// numerical_methods.x —— 方程求根：二分法 / 牛顿迭代 / 割线法 / 不动点迭代（并入 std.math）
// 所有方法均以闭包传入函数 f

// 二分法求 f(x)=0 在 [a, b] 的根；区间不变号返回 0
fn bisection(f, a, b, eps) {
    let fa = f(a);
    let fb = f(b);
    if fa * fb > 0 {
        return 0;
    }
    let cnt = 0;
    while to_float(b) - to_float(a) > eps and cnt < 100 {
        let mid = (to_float(a) + to_float(b)) / 2.0;
        let fm = f(mid);
        if fm == 0 {
            return mid;
        }
        if fa * fm < 0 {
            b = mid;
        } else {
            a = mid;
            fa = fm;
        }
        cnt = cnt + 1;
    }
    return (to_float(a) + to_float(b)) / 2.0;
}

// 牛顿迭代法求根，df 为 f 的导函数；最多 30 次，导数为 0 返回 0
fn newton_root(f, df, x0, eps) {
    let x = to_float(x0);
    let cnt = 0;
    while cnt < 30 {
        let fx = f(x);
        if std.math.abs(fx) < eps {
            return x;
        }
        let dx = df(x);
        if dx == 0 {
            return 0;
        }
        x = x - fx / dx;
        cnt = cnt + 1;
    }
    return x;
}

// 割线法求根；最多 50 次，分母为 0 返回 0
fn secant(f, x0, x1, eps) {
    let a = to_float(x0);
    let b = to_float(x1);
    let fa = f(a);
    let fb = f(b);
    let cnt = 0;
    while cnt < 50 {
        if std.math.abs(fb) < eps {
            return b;
        }
        let denom = fb - fa;
        if denom == 0 {
            return 0;
        }
        let c = b - fb * (b - a) / denom;
        a = b;
        fa = fb;
        b = c;
        fb = f(b);
        cnt = cnt + 1;
    }
    return b;
}

// 不动点迭代 x = g(x)；最多 100 次，相邻差小于 eps 返回
fn fixed_point(g, x0, eps) {
    let x = to_float(x0);
    let cnt = 0;
    while cnt < 100 {
        let xn = g(x);
        if std.math.abs(xn - x) < eps {
            return xn;
        }
        x = xn;
        cnt = cnt + 1;
    }
    return x;
}
