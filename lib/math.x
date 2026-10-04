// lib/math.x

const PI = 3.141592653589793;
const E = 2.718281828459045;

// 绝对值：支持整数、浮点数
fn abs(n) {
    if n < 0 {
        return -n;
    }
    return n;
}

// 幂运算：base^exp，exp 为整数，支持负指数
fn pow(base, exp) {
    if exp == 0 {
        return 1;
    }
    if exp < 0 {
        if base == 0 {
            return 0;
        }
        return 1.0 / lib::math::pow(base * 1.0, -exp);
    }
    let result = 1;
    let b = base;
    let e = exp;
    while e > 0 {
        if e % 2 == 1 {
            result = result * b;
        }
        b = b * b;
        e = e / 2;
    }
    return result;
}

// 平方根：牛顿迭代法，非负数输入，迭代10次保证精度
fn sqrt(n) {
    if n < 0 {
        return 0;
    }
    if n == 0 {
        return 0;
    }
    let x = n;
    for i in 0..10 {
        x = (x + n / x) / 2;
    }
    return x;
}

// 取两数较大值
fn max(a, b) {
    if a > b {
        return a;
    }
    return b;
}

// 取两数较小值
fn min(a, b) {
    if a < b {
        return a;
    }
    return b;
}

// 阶乘：n 为非负整数，非法输入返回0
fn factorial(n) {
    if n < 0 or n % 1 != 0 {
        return 0;
    }
    if n == 0 or n == 1 {
        return 1;
    }
    let result = 1;
    for i in 2..n + 1 {
        result = result * i;
    }
    return result;
}

// 最大公约数：辗转相除法，自动处理正负
fn gcd(a, b) {
    a = lib::math::abs(a);
    b = lib::math::abs(b);
    if b == 0 {
        return a;
    }
    while b != 0 {
        let temp = b;
        b = a % b;
        a = temp;
    }
    return a;
}

// 最小公倍数
fn lcm(a, b) {
    if a == 0 or b == 0 {
        return 0;
    }
    let g = lib::math::gcd(a, b);
    return lib::math::abs(a) / g * lib::math::abs(b);
}

// 素数判断：是返回 true，否返回 false
fn is_prime(n) {
    if n <= 1 {
        return false;
    }
    // 非整数不是素数
    if n % 1 != 0 {
        return false;
    }
    if n == 2 {
        return true;
    }
    if n % 2 == 0 {
        return false;
    }
    let i = 3;
    // 用除法替代乘法，彻底避免溢出
    while i <= n / i {
        if n % i == 0 {
            return false;
        }
        i = i + 2;
    }
    return true;
}

// 数值钳制：将 n 限制在 [min_val, max_val] 区间内
fn clamp(n, min_val, max_val) {
    let real_min = min_val;
    let real_max = max_val;
    if real_min > real_max {
        let temp = real_min;
        real_min = real_max;
        real_max = temp;
    }
    if n < real_min {
        return real_min;
    }
    if n > real_max {
        return real_max;
    }
    return n;
}

// 符号函数：正数返回 1，负数返回 -1，0 返回 0
fn sign(n) {
    if n > 0 {
        return 1;
    }
    if n < 0 {
        return -1;
    }
    return 0;
}

// 向下取整
fn floor(n) {
    if n >= 0 {
        return n - n % 1;
    } else {
        return n - (n % 1 + 1);
    }
}

// 向上取整
fn ceil(n) {
    if n <= 0 {
        return n - n % 1;
    } else {
        if n % 1 == 0 {
            return n;
        }
        return n - n % 1 + 1;
    }
}