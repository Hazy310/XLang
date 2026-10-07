// category_theory.x —— 范畴论基础：高阶函数与函数组合（并入 std.math）
// 复合 / 恒等 / 常函数 / 应用 / 换序 / 柯里化

// 复合 f∘g：返回 fn(x){ f(g(x)) }
fn compose(f, g) {
    return fn(x) {
        return f(g(x));
    };
}

// 恒等函数 id(x)=x
fn id(x) {
    return x;
}

// 常函数：返回 fn(x){ c }
fn constant(c) {
    return fn(x) {
        return c;
    };
}

// 函数应用 apply(f,x)=f(x)
fn apply(f, x) {
    return f(x);
}

// 交换参数 flip(f,a,b)=f(b,a)
fn flip(f, a, b) {
    return f(b, a);
}

// 柯里化 curry(f,a)：返回 fn(b){ f(a,b) }
fn curry(f, a) {
    return fn(b) {
        return f(a, b);
    };
}
