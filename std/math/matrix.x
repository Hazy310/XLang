// matrix.x —— 矩阵运算（并入 std.math，矩阵用行数组的数组表示 [[a,b],[c,d]]）
// 乘法 / 转置 / 行列式 / 逆 / 加 / 数乘 / 单位阵

// 矩阵乘法：a(n×m) * b(m×p)，维度不匹配返回 0
fn mmul(a, b) {
    let n = len(a);
    if n == 0 {
        return 0;
    }
    let m = len(a[0]);
    let p = len(b[0]);
    if len(b) != m {
        return 0;
    }
    let out = [];
    for i in 0..n {
        let row = [];
        for j in 0..p {
            let s = 0.0;
            for k in 0..m {
                s = s + a[i][k] * b[k][j];
            }
            row = row + [s];
        }
        out = out + [row];
    }
    return out;
}

// 转置
fn mtranspose(m) {
    let n = len(m);
    if n == 0 {
        return 0;
    }
    let c = len(m[0]);
    let out = [];
    for j in 0..c {
        let row = [];
        for i in 0..n {
            row = row + [m[i][j]];
        }
        out = out + [row];
    }
    return out;
}

// 行列式：2×2 / 3×3 用公式，n×n 按第一行拉普拉斯展开（递归）
fn mdet(m) {
    let n = len(m);
    if n == 0 {
        return 0;
    }
    if n == 1 {
        return m[0][0];
    }
    if n == 2 {
        return m[0][0] * m[1][1] - m[0][1] * m[1][0];
    }
    if n == 3 {
        return m[0][0] * (m[1][1] * m[2][2] - m[1][2] * m[2][1])
            - m[0][1] * (m[1][0] * m[2][2] - m[1][2] * m[2][0])
            + m[0][2] * (m[1][0] * m[2][1] - m[1][1] * m[2][0]);
    }
    let det = 0.0;
    for j in 0..n {
        let sub = [];
        for i in 1..n {
            let row = [];
            for c in 0..n {
                if c != j {
                    row = row + [m[i][c]];
                }
            }
            sub = sub + [row];
        }
        let sign = 1;
        if j % 2 == 1 {
            sign = -1;
        }
        det = det + to_float(sign) * m[0][j] * std.math.mdet(sub);
    }
    return det;
}

// 逆矩阵：高斯-约当消元（[A|I] → [I|A⁻¹]），奇异返回 0
fn minverse(m) {
    let n = len(m);
    if n == 0 {
        return 0;
    }
    // 转浮点并拼接单位阵
    let aug = [];
    for i in 0..n {
        let row = [];
        for j in 0..n {
            row = row + [to_float(m[i][j])];
        }
        for j in 0..n {
            if j == i {
                row = row + [1.0];
            } else {
                row = row + [0.0];
            }
        }
        aug = aug + [row];
    }
    for col in 0..n {
        // 选主元：当前列绝对值最大的行
        let pivot = col;
        let maxv = std.math.abs(aug[col][col]);
        let r = col + 1;
        while r < n {
            let v = std.math.abs(aug[r][col]);
            if v > maxv {
                maxv = v;
                pivot = r;
            }
            r = r + 1;
        }
        if maxv == 0 {
            return 0;
        }
        // 交换到当前行
        if pivot != col {
            let t = aug[pivot];
            aug[pivot] = aug[col];
            aug[col] = t;
        }
        // 归一化当前行
        let pv = aug[col][col];
        let nrow = [];
        for j in 0..2 * n {
            nrow = nrow + [aug[col][j] / pv];
        }
        aug[col] = nrow;
        // 消去其他行的当前列
        for r2 in 0..n {
            if r2 != col {
                let f = aug[r2][col];
                if f != 0 {
                    let erow = [];
                    for j in 0..2 * n {
                        erow = erow + [aug[r2][j] - f * aug[col][j]];
                    }
                    aug[r2] = erow;
                }
            }
        }
    }
    // 取右半部分
    let inv = [];
    for i in 0..n {
        let row = [];
        for j in 0..n {
            row = row + [aug[i][n + j]];
        }
        inv = inv + [row];
    }
    return inv;
}

// 矩阵加法（同尺寸，逐元素）
fn madd(a, b) {
    let n = len(a);
    if n == 0 {
        return 0;
    }
    let out = [];
    for i in 0..n {
        let row = [];
        for j in 0..len(a[0]) {
            row = row + [a[i][j] + b[i][j]];
        }
        out = out + [row];
    }
    return out;
}

// 数乘
fn mscale(k, m) {
    let n = len(m);
    if n == 0 {
        return 0;
    }
    let out = [];
    for i in 0..n {
        let row = [];
        for j in 0..len(m[0]) {
            row = row + [k * m[i][j]];
        }
        out = out + [row];
    }
    return out;
}

// 单位矩阵
fn midentity(n) {
    let out = [];
    for i in 0..n {
        let row = [];
        for j in 0..n {
            if j == i {
                row = row + [1];
            } else {
                row = row + [0];
            }
        }
        out = out + [row];
    }
    return out;
}
