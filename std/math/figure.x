// figure.x —— 平面 / 立体几何计算（并入 std.math 模块，import std.math 即可用）
// 周长 / 面积组织在 Perimeter / Area 类的 static 方法；体积与点线几何为顶层函数

// ---------- 周长 ----------
class Perimeter {
    // 圆周长
    static fn circle(r) {
        return 6.283185307179586 * r;
    }
    // 正方形
    static fn square(s) {
        return 4 * s;
    }
    // 矩形
    static fn rect(w, h) {
        return 2 * (w + h);
    }
    // 正多边形：n 边、边长 side
    static fn regular(n, side) {
        return n * side;
    }
}

// ---------- 面积 ----------
class Area {
    static fn circle(r) {
        return 3.141592653589793 * r * r;
    }
    static fn square(s) {
        return s * s;
    }
    static fn rect(w, h) {
        return w * h;
    }
    static fn parallelogram(base, height) {
        return base * height;
    }
    static fn triangle(base, height) {
        return base * height / 2;
    }
    // 海伦公式：三边求面积，不构成三角形时返回 0
    static fn heron(a, b, c) {
        if a + b <= c or a + c <= b or b + c <= a {
            return 0;
        }
        let s = (a + b + c) / 2;
        return std.math.sqrt(s * (s - a) * (s - b) * (s - c));
    }
    // 梯形
    static fn trapezoid(a, b, h) {
        return (a + b) * h / 2;
    }
    // 菱形（对角线）
    static fn rhombus(d1, d2) {
        return d1 * d2 / 2;
    }
    // 扇形面积：angle 弧度
    static fn sector(r, angle) {
        return 0.5 * r * r * angle;
    }
    // 正多边形面积
    static fn regular(n, side) {
        return n * side * side / (4 * std.math.tan(3.141592653589793 / n));
    }
    // 球表面积
    static fn sphere(r) {
        return 4 * 3.141592653589793 * r * r;
    }
    // 圆柱表面积
    static fn cylinder(r, h) {
        return 2 * 3.141592653589793 * r * (r + h);
    }
    // 长方体表面积
    static fn cuboid(l, w, h) {
        return 2 * (l * w + l * h + w * h);
    }
}

// ---------- 体积 ----------
class Volume {
    static fn sphere(r) {
        return 4 * 3.141592653589793 * r * r * r / 3;
    }
    static fn cylinder(r, h) {
        return 3.141592653589793 * r * r * h;
    }
    static fn cone(r, h) {
        return 3.141592653589793 * r * r * h / 3;
    }
    static fn cube(s) {
        return s * s * s;
    }
    static fn cuboid(l, w, h) {
        return l * w * h;
    }
    // 圆锥母线长
    static fn cone_slant(r, h) {
        return std.math.sqrt(r * r + h * h);
    }
    // 棱柱：底面积 × 高
    static fn prism(base_area, h) {
        return base_area * h;
    }
    // 棱锥：底面积 × 高 ÷ 3
    static fn pyramid(base_area, h) {
        return base_area * h / 3;
    }
    // 圆台（圆锥台）：大底半径 R、小底半径 r、高 h
    static fn cone_frustum(R, r, h) {
        return 3.141592653589793 * h * (R * R + R * r + r * r) / 3;
    }
    // 球冠：球半径 r、冠高 h
    static fn sphere_cap(r, h) {
        return 3.141592653589793 * h * h * (3 * r - h) / 3;
    }
    // 椭球：三半轴 a、b、c
    static fn ellipsoid(a, b, c) {
        return 4 * 3.141592653589793 * a * b * c / 3;
    }
    // 环面（甜甜圈）：环半径 R、管半径 r
    static fn torus(R, r) {
        return 2 * 3.141592653589793 * 3.141592653589793 * R * r * r;
    }
}

// ---------- 点 / 线 / 三角形 ----------
// 两点距离
fn distance(x1, y1, x2, y2) {
    let dx = x2 - x1;
    let dy = y2 - y1;
    return std.math.sqrt(dx * dx + dy * dy);
}
// 中点，返回 [mx, my]
fn midpoint(x1, y1, x2, y2) {
    return [(x1 + x2) / 2, (y1 + y2) / 2];
}
// 斜率（竖直线返回 0）
fn slope(x1, y1, x2, y2) {
    if x2 == x1 {
        return 0;
    }
    return (y2 - y1) / (x2 - x1);
}
// 点到直线距离：直线过 A(ax, ay)、B(bx, by)
fn point_line_distance(px, py, ax, ay, bx, by) {
    let dx = bx - ax;
    let dy = by - ay;
    let denom = std.math.sqrt(dx * dx + dy * dy);
    if denom == 0 {
        return std.math.distance(px, py, ax, ay);
    }
    return std.math.abs(dx * (ay - py) - dy * (ax - px)) / denom;
}
// 三点坐标三角形面积
fn triangle_area_coord(x1, y1, x2, y2, x3, y3) {
    let s = x1 * (y2 - y3) + x2 * (y3 - y1) + x3 * (y1 - y2);
    return std.math.abs(s) / 2;
}
// 多边形周长：扁平顶点数组 [x1, y1, x2, y2, ...]，n 个顶点
fn polygon_perimeter(points, n) {
    let total = 0.0;
    for i in 0..n {
        let x1 = points[2 * i];
        let y1 = points[2 * i + 1];
        let x2 = points[2 * ((i + 1) % n)];
        let y2 = points[2 * ((i + 1) % n) + 1];
        total = total + std.math.distance(x1, y1, x2, y2);
    }
    return total;
}
