// differential_equations.x —— 常微分方程初值问题：显式欧拉法 / 四阶龙格-库塔法（并入 std.math）
// 右端函数签名 f(t, y)，返回含初值共 n+1 个 y 值数组

// 显式欧拉法求解 y' = f(t, y)，步长 h，共 n 步
fn euler(f, t0, y0, h, n) {
    let y = to_float(y0);
    let t = to_float(t0);
    let hh = to_float(h);
    let ys = [];
    ys = ys + [y];
    let i = 0;
    while i < n {
        y = y + hh * f(t, y);
        t = t + hh;
        ys = ys + [y];
        i = i + 1;
    }
    return ys;
}

// 四阶龙格-库塔法（RK4）求解 y' = f(t, y)，步长 h，共 n 步
fn rk4(f, t0, y0, h, n) {
    let y = to_float(y0);
    let t = to_float(t0);
    let hh = to_float(h);
    let ys = [];
    ys = ys + [y];
    let i = 0;
    while i < n {
        let k1 = f(t, y);
        let k2 = f(t + hh / 2.0, y + hh * k1 / 2.0);
        let k3 = f(t + hh / 2.0, y + hh * k2 / 2.0);
        let k4 = f(t + hh, y + hh * k3);
        y = y + hh / 6.0 * (k1 + 2.0 * k2 + 2.0 * k3 + k4);
        t = t + hh;
        ys = ys + [y];
        i = i + 1;
    }
    return ys;
}
