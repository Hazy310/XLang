// geometry.x —— 二维几何：多边形面积 / 点在多边形内 / 点到线段距离 / 直线交点 / 两圆相交（并入 std.math，import std.math 即可用）
// 多边形用扁平顶点数组 [x1, y1, x2, y2, ...]，与 figure.x polygon_perimeter 一致

// 多边形面积（鞋带公式），n 个顶点，返回 float
fn polygon_area(points, n) {
    let s = 0.0;
    for i in 0..n {
        let xi = to_float(points[2 * i]);
        let yi = to_float(points[2 * i + 1]);
        let xj = to_float(points[2 * ((i + 1) % n)]);
        let yj = to_float(points[2 * ((i + 1) % n) + 1]);
        s = s + (xi * yj - xj * yi);
    }
    return std.math.abs(s) / 2.0;
}

// 射线法判断点是否在多边形内，返回 1/0
fn point_in_polygon(px, py, points, n) {
    let inside = 0;
    let j = n - 1;
    for i in 0..n {
        let yi = to_float(points[2 * i + 1]);
        let yj = to_float(points[2 * j + 1]);
        let xi = to_float(points[2 * i]);
        let xj = to_float(points[2 * j]);
        if (yi > py) != (yj > py) {
            let xint = xi + (py - yi) * (xj - xi) / (yj - yi);
            if xint > px {
                inside = 1 - inside;
            }
        }
        j = i;
    }
    return inside;
}

// 点 P 到线段 AB 的距离
fn point_segment_distance(px, py, ax, ay, bx, by) {
    let dx = to_float(bx - ax);
    let dy = to_float(by - ay);
    if dx == 0 and dy == 0 {
        return std.math.distance(px, py, ax, ay);
    }
    let t = (to_float(px - ax) * dx + to_float(py - ay) * dy) / (dx * dx + dy * dy);
    if t < 0.0 {
        t = 0.0;
    }
    if t > 1.0 {
        t = 1.0;
    }
    let cx = to_float(ax) + t * dx;
    let cy = to_float(ay) + t * dy;
    let ddx = to_float(px) - cx;
    let ddy = to_float(py) - cy;
    return std.math.sqrt(ddx * ddx + ddy * ddy);
}

// 两直线（过 AB、过 CD）交点，返回 [x, y]；平行返回 0
fn line_intersection(ax, ay, bx, by, cx, cy, dx, dy) {
    let denom = (bx - ax) * (dy - cy) - (by - ay) * (dx - cx);
    if denom == 0 {
        return 0;
    }
    let t = (to_float(cx - ax) * (dy - cy) - to_float(cy - ay) * (dx - cx)) / denom;
    return [to_float(ax) + t * (bx - ax), to_float(ay) + t * (by - ay)];
}

// 两圆是否相交（含相切 / 内含边界），返回 1/0
fn circles_overlap(x1, y1, r1, x2, y2, r2) {
    let d = std.math.distance(x1, y1, x2, y2);
    if std.math.abs(to_float(r1) - to_float(r2)) <= d and d <= to_float(r1) + to_float(r2) {
        return 1;
    }
    return 0;
}
