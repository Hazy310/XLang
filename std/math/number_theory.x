// number_theory.x —— 数论（并入 std.math，import std.math 即可用）
// 素数 / 质因数分解 / 欧拉函数 / 快速幂取模 / 互质

// 返回 >= n 的最小素数
fn next_prime(n) {
    if n < 2 {
        n = 2;
    }
    while !std.math.is_prime(n) {
        n = n + 1;
    }
    return n;
}

// 质因数分解（含重复），n<=1 返回空数组
fn prime_factors(n) {
    let out = [];
    if n <= 1 {
        return out;
    }
    n = std.math.abs(n);
    while n % 2 == 0 {
        out = out + [2];
        n = n / 2;
    }
    let d = 3;
    while d <= n / d {
        while n % d == 0 {
            out = out + [d];
            n = n / d;
        }
        d = d + 2;
    }
    if n > 1 {
        out = out + [n];
    }
    return out;
}

// 欧拉函数 φ(n)：<= n 且与 n 互质的正整数个数
fn totient(n) {
    if n <= 1 {
        return 0;
    }
    let result = n;
    let temp = n;
    let p = 2;
    while p <= temp / p {
        if temp % p == 0 {
            while temp % p == 0 {
                temp = temp / p;
            }
            result = result - result / p;
        }
        if p == 2 {
            p = 3;
        } else {
            p = p + 2;
        }
    }
    if temp > 1 {
        result = result - result / temp;
    }
    return result;
}

// 快速幂取模：(base^exp) % mod，mod<=0 返回 0
fn mod_pow(base, exp, mod) {
    if mod <= 0 {
        return 0;
    }
    let result: i64 = 1;
    let b: i64 = base % mod;
    let e = exp;
    while e > 0 {
        if e % 2 == 1 {
            result = (result * b) % mod;
        }
        b = (b * b) % mod;
        e = e / 2;
    }
    return result;
}

// 互质判断：gcd(a,b)==1 返回 1 否则 0
fn is_coprime(a, b) {
    if std.math.gcd(a, b) == 1 {
        return 1;
    }
    return 0;
}
