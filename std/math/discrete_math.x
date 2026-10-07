// discrete_math.x —— 离散数学（并入 std.math，import std.math 即可用）
// 斐波那契 / 因子枚举 / 数位和 / 二分查找 / 完全数

// 第 n 项斐波那契（float 返回防溢出）：n<=0 返回 0，n==1 返回 1
fn fibonacci(n) {
    if n <= 0 {
        return 0;
    }
    if n == 1 {
        return 1.0;
    }
    let a = 0.0;
    let b = 1.0;
    for i in 2..n + 1 {
        let c = a + b;
        a = b;
        b = c;
    }
    return b;
}

// 正整数因子数组（1..n 遍历，n%d==0 追加）；n<=0 返回 []
fn divisors(n) {
    let out = [];
    if n <= 0 {
        return out;
    }
    for d in 1..n + 1 {
        if n % d == 0 {
            out = out + [d];
        }
    }
    return out;
}

// 十进制各位数字和（int 除法）
fn digit_sum(n) {
    let m = std.math.abs(n);
    let s = 0;
    while m > 0 {
        s = s + m % 10;
        m = m / 10;
    }
    return s;
}

// 已排序数组二分查找返回下标，找不到返回 -1
fn binary_search(arr, x) {
    let lo = 0;
    let hi = len(arr) - 1;
    while lo <= hi {
        let mid = (lo + hi) / 2;
        if arr[mid] == x {
            return mid;
        }
        if arr[mid] < x {
            lo = mid + 1;
        } else {
            hi = mid - 1;
        }
    }
    return -1;
}

// 完全数判断（真因子和==n）返回 1/0；n<=1 返回 0
fn is_perfect(n) {
    if n <= 1 {
        return 0;
    }
    let s = 0;
    for d in 1..n / 2 + 1 {
        if n % d == 0 {
            s = s + d;
        }
    }
    if s == n {
        return 1;
    }
    return 0;
}
