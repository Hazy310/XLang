// finance.x —— 金融计算（std 根目录模块）
// 用法：import std.finance;  P 本金/现值，r 年利率（如 0.05），t 年数（整数）
// 提供：simple_interest compound fv pv pmt total_repay annuity_fv
// 说明：期数为整数，用循环复利，避免浮点幂依赖

// 单利利息：P*r*t
fn simple_interest(P, r, t) {
    to_float(P) * to_float(r) * to_float(t)
}

// 复利终值：P*(1+r)^t（t 整数年）
fn compound(P, r, t) {
    let v = to_float(P);
    let i = 0;
    while i < t {
        v = v * (1.0 + to_float(r));
        i = i + 1;
    }
    v
}

// 终值（复利）
fn fv(P, r, t) {
    std.finance.compound(P, r, t)
}

// 现值：F/(1+r)^t
fn pv(F, r, t) {
    let v = to_float(F);
    let i = 0;
    while i < t {
        v = v / (1.0 + to_float(r));
        i = i + 1;
    }
    v
}

// 等额本息月供：贷款 P，年利率 annual_r，还款月数 months
fn pmt(P, annual_r, months) {
    let mr = to_float(annual_r) / 12.0;
    let factor = 1.0;
    let i = 0;
    while i < months {
        factor = factor * (1.0 + mr);
        i = i + 1;
    }
    let denom = 1.0 - 1.0 / factor;
    if denom == 0 {
        return to_float(P) / to_float(months);
    }
    to_float(P) * mr / denom
}

// 贷款总还款额：月供 * 月数
fn total_repay(P, annual_r, months) {
    std.finance.pmt(P, annual_r, months) * to_float(months)
}

// 年金终值：每月存入 pmt_amt，月利率 r（月），共 months 期
fn annuity_fv(pmt_amt, r, months) {
    let v = 0.0;
    let i = 0;
    while i < months {
        v = (v + to_float(pmt_amt)) * (1.0 + to_float(r));
        i = i + 1;
    }
    v
}
