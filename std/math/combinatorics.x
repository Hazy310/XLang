// combinatorics.x —— 组合数学（并入 std.math，import std.math 即可用）
// 排列 / 组合 / 卡特兰 / 第二类斯特林 / 贝尔 / 错排（均用乘法公式，不依赖 factorial）

// 排列数 nPk；k<0 or k>n 返回 0
fn perm(n, k) {
    if k < 0 or k > n {
        return 0;
    }
    let r: i64 = 1;
    for i in 0..k {
        r = r * (n - i);
    }
    return r;
}

// 组合数 nCk；k<0 or k>n 返回 0；每步乘法后整除
fn comb(n, k) {
    if k < 0 or k > n {
        return 0;
    }
    if k > n - k {
        k = n - k;
    }
    let r: i64 = 1;
    for i in 1..k + 1 {
        r = r * (n - k + i) / i;
    }
    return r;
}

// 卡特兰数 C(2n,n)/(n+1)；n<0 返回 0
fn catalan(n) {
    if n < 0 {
        return 0;
    }
    return std.math.comb(2 * n, n) / (n + 1);
}

// 第二类斯特林数（递归）
fn stirling2(n, k) {
    if k > n or k == 0 {
        return 0;
    }
    if n == 0 {
        return 0;
    }
    if k == 1 or k == n {
        return 1;
    }
    return k * std.math.stirling2(n - 1, k) + std.math.stirling2(n - 1, k - 1);
}

// 贝尔数：Σ_{k=0..n} S(n,k)
fn bell(n) {
    let s: i64 = 0;
    for k in 0..n + 1 {
        s = s + std.math.stirling2(n, k);
    }
    return s;
}

// 错排数（迭代递推）
fn derangements(n) {
    if n == 0 {
        return 1;
    }
    if n == 1 {
        return 0;
    }
    let a: i64 = 1;
    let b: i64 = 0;
    for i in 2..n + 1 {
        let c = (i - 1) * (a + b);
        a = b;
        b = c;
    }
    return b;
}
