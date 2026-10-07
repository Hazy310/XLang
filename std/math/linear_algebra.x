// linear_algebra.x —— 线性代数（并入 std.math，import std.math 即可用）
// 矩阵向量乘法 / 外积 / 高斯消元解线性方程组

// 矩阵×向量：out[i] = Σ_k m[i][k]*v[k]（float 起步 s=0.0）
fn mat_vec_mul(m, v) {
    let n = len(m);
    let out = [];
    for i in 0..n {
        let s = 0.0;
        let row = m[i];
        for k in 0..len(row) {
            s = s + row[k] * v[k];
        }
        out = out + [s];
    }
    return out;
}

// 外积：out[i][j] = a[i]*b[j]
fn outer(a, b) {
    let out = [];
    for i in 0..len(a) {
        let row = [];
        for j in 0..len(b) {
            row = row + [a[i] * b[j]];
        }
        out = out + [row];
    }
    return out;
}

// 高斯消元解 Ax=b，返回 x（float 数组）；奇异返回 0
fn solve_linear(A, b) {
    let n = len(A);
    // 1) 增广转 float：每行 = [A[i][0..n-1] float] + [to_float(b[i])]，n 行 n+1 列
    let aug = [];
    for i in 0..n {
        let row = [];
        for j in 0..n {
            row = row + [to_float(A[i][j])];
        }
        row = row + [to_float(b[i])];
        aug = aug + [row];
    }
    // 2) 前向消元（列主元）
    for col in 0..n {
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
        if pivot != col {
            let t = aug[pivot];
            aug[pivot] = aug[col];
            aug[col] = t;
        }
        let r2 = col + 1;
        while r2 < n {
            let f = aug[r2][col] / aug[col][col];
            if f != 0 {
                let nrow = [];
                for j in 0..n + 1 {
                    nrow = nrow + [aug[r2][j] - f * aug[col][j]];
                }
                aug[r2] = nrow;
            }
            r2 = r2 + 1;
        }
    }
    // 3) 回代
    let x = [];
    for i in 0..n {
        x = x + [0.0];
    }
    let i = n - 1;
    while i >= 0 {
        let s = aug[i][n];
        let j = i + 1;
        while j < n {
            s = s - aug[i][j] * x[j];
            j = j + 1;
        }
        x[i] = s / aug[i][i];
        i = i - 1;
    }
    return x;
}
