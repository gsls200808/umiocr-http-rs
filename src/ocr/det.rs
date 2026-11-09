use std::cmp::Ordering;

#[derive(Debug, Clone, Copy)]
struct Pt {
    x: f64,
    y: f64,
}

impl Pt {
    fn new(x: f64, y: f64) -> Self {
        Self { x, y }
    }
}

fn cmp_x(a: &Pt, b: &Pt) -> Ordering {
    a.x.partial_cmp(&b.x).unwrap_or(Ordering::Equal)
}

/// DB 后处理：阈值分割 -> 连通域 -> 最小外接矩形 -> 外扩 -> 还原到原图坐标。
pub fn db_post_process(
    pred: &[f32],
    h: usize,
    w: usize,
    dest_w: usize,
    dest_h: usize,
) -> Vec<[[f64; 2]; 4]> {
    const THRESH: f32 = 0.3;
    const BOX_THRESH: f64 = 0.5;
    const UNCLIP_RATIO: f64 = 1.6;
    const MIN_SIZE: f64 = 3.0;
    const MAX_CANDIDATES: usize = 1000;

    let total = h * w;
    let mut bitmap = vec![false; total];
    for (i, slot) in bitmap.iter_mut().enumerate() {
        if let Some(v) = pred.get(i) {
            *slot = *v > THRESH;
        }
    }

    let mut comps = connected_components(&bitmap, h, w);
    if comps.len() > MAX_CANDIDATES {
        comps.truncate(MAX_CANDIDATES);
    }

    let mut boxes = Vec::with_capacity(comps.len());
    for pts in comps {
        if pts.len() < 4 {
            continue;
        }
        let (mini, sside) = mini_box(&pts);
        if sside < MIN_SIZE {
            continue;
        }
        let score = box_score_fast(pred, h, w, &mini);
        if score < BOX_THRESH {
            continue;
        }
        let expanded = unclip(&mini, UNCLIP_RATIO);
        let (box_, sside2) = mini_box(&expanded);
        if sside2 < MIN_SIZE + 2.0 {
            continue;
        }
        let mut out = [[0f64; 2]; 4];
        for (i, p) in box_.iter().enumerate() {
            let x = (p.x / w as f64 * dest_w as f64).round();
            let y = (p.y / h as f64 * dest_h as f64).round();
            out[i][0] = x.clamp(0.0, dest_w as f64);
            out[i][1] = y.clamp(0.0, dest_h as f64);
        }
        boxes.push(out);
    }
    boxes
}

/// 8 连通域，返回每个连通域的像素点集。
fn connected_components(bitmap: &[bool], h: usize, w: usize) -> Vec<Vec<Pt>> {
    let mut visited = vec![false; h * w];
    let mut comps = Vec::new();
    for y in 0..h {
        for x in 0..w {
            let idx = y * w + x;
            if !bitmap[idx] || visited[idx] {
                continue;
            }
            visited[idx] = true;
            let mut stack = vec![(x, y)];
            let mut pts = Vec::with_capacity(16);
            while let Some((nx, ny)) = stack.pop() {
                pts.push(Pt::new(nx as f64, ny as f64));
                for dy in -1i32..=1 {
                    for dx in -1i32..=1 {
                        if dx == 0 && dy == 0 {
                            continue;
                        }
                        let ax = nx as i32 + dx;
                        let ay = ny as i32 + dy;
                        if ax < 0 || ay < 0 || ax >= w as i32 || ay >= h as i32 {
                            continue;
                        }
                        let ai = ay as usize * w + ax as usize;
                        if bitmap[ai] && !visited[ai] {
                            visited[ai] = true;
                            stack.push((ax as usize, ay as usize));
                        }
                    }
                }
            }
            comps.push(pts);
        }
    }
    comps
}

/// 求最小面积外接矩形，输出 [tl,tr,br,bl] 顺序，返回短边长度。
fn mini_box(pts: &[Pt]) -> ([Pt; 4], f64) {
    let hull = convex_hull(pts);
    if hull.len() < 3 {
        return ([Pt::new(0.0, 0.0); 4], 0.0);
    }
    let mut best_area = f64::INFINITY;
    let mut best_box = [Pt::new(0.0, 0.0); 4];
    let mut best_short = 0.0;
    for i in 0..hull.len() {
        let p1 = hull[i];
        let p2 = hull[(i + 1) % hull.len()];
        let (mut ex, mut ey) = (p2.x - p1.x, p2.y - p1.y);
        let len = ex.hypot(ey);
        if len < 1e-9 {
            continue;
        }
        ex /= len;
        ey /= len;
        let (px, py) = (-ey, ex);
        let (mut min_u, mut max_u, mut min_v, mut max_v) =
            (f64::INFINITY, f64::NEG_INFINITY, f64::INFINITY, f64::NEG_INFINITY);
        for p in &hull {
            let u = p.x * ex + p.y * ey;
            let v = p.x * px + p.y * py;
            min_u = min_u.min(u);
            max_u = max_u.max(u);
            min_v = min_v.min(v);
            max_v = max_v.max(v);
        }
        let area = (max_u - min_u) * (max_v - min_v);
        if area < best_area {
            best_area = area;
            best_short = (max_u - min_u).min(max_v - min_v);
            best_box = [
                Pt::new(min_u * ex + min_v * px, min_u * ey + min_v * py),
                Pt::new(max_u * ex + min_v * px, max_u * ey + min_v * py),
                Pt::new(max_u * ex + max_v * px, max_u * ey + max_v * py),
                Pt::new(min_u * ex + max_v * px, min_u * ey + max_v * py),
            ];
        }
    }

    // 按 x 排序后重排为 [tl,tr,br,bl]
    let mut s = best_box.to_vec();
    s.sort_by(cmp_x);
    let (mut i1, mut i4, mut i2, mut i3) = (0usize, 1usize, 2usize, 3usize);
    if s[1].y <= s[0].y {
        i1 = 1;
        i4 = 0;
    }
    if s[3].y <= s[2].y {
        i2 = 3;
        i3 = 2;
    }
    ([s[i1], s[i2], s[i3], s[i4]], best_short)
}

/// Andrew monotone chain 凸包。
fn convex_hull(pts: &[Pt]) -> Vec<Pt> {
    let mut p = pts.to_vec();
    p.sort_by(|a, b| {
        if a.x != b.x {
            a.x.partial_cmp(&b.x).unwrap_or(Ordering::Equal)
        } else {
            a.y.partial_cmp(&b.y).unwrap_or(Ordering::Equal)
        }
    });
    let cross = |o: Pt, a: Pt, b: Pt| (a.x - o.x) * (b.y - o.y) - (a.y - o.y) * (b.x - o.x);
    let mut lower: Vec<Pt> = Vec::new();
    for &q in &p {
        while lower.len() >= 2 && cross(lower[lower.len() - 2], lower[lower.len() - 1], q) <= 0.0 {
            lower.pop();
        }
        lower.push(q);
    }
    let mut upper: Vec<Pt> = Vec::new();
    for &q in p.iter().rev() {
        while upper.len() >= 2 && cross(upper[upper.len() - 2], upper[upper.len() - 1], q) <= 0.0 {
            upper.pop();
        }
        upper.push(q);
    }
    lower.pop();
    upper.pop();
    lower.extend(upper);
    lower
}

fn poly_area(pts: &[Pt]) -> f64 {
    let mut a = 0.0;
    for i in 0..pts.len() {
        let j = (i + 1) % pts.len();
        a += pts[i].x * pts[j].y - pts[j].x * pts[i].y;
    }
    a.abs() / 2.0
}

/// 沿各边外法线平移 distance 后求相邻边交点（等价 pyclipper 对凸多边形的等距外扩）。
fn unclip(square: &[Pt; 4], ratio: f64) -> Vec<Pt> {
    let area = poly_area(square);
    let mut perim = 0.0;
    for i in 0..4 {
        let a = square[i];
        let b = square[(i + 1) % 4];
        perim += (b.x - a.x).hypot(b.y - a.y);
    }
    if perim <= 0.0 {
        return square.to_vec();
    }
    let d = area * ratio / perim;
    let (mut cx, mut cy) = (0.0, 0.0);
    for p in square {
        cx += p.x;
        cy += p.y;
    }
    cx /= 4.0;
    cy /= 4.0;

    struct Line {
        ax: f64,
        ay: f64,
        ex: f64,
        ey: f64,
    }
    let mut lines: [Line; 4] = [
        Line { ax: 0.0, ay: 0.0, ex: 0.0, ey: 0.0 },
        Line { ax: 0.0, ay: 0.0, ex: 0.0, ey: 0.0 },
        Line { ax: 0.0, ay: 0.0, ex: 0.0, ey: 0.0 },
        Line { ax: 0.0, ay: 0.0, ex: 0.0, ey: 0.0 },
    ];
    for i in 0..4 {
        let p1 = square[i];
        let p2 = square[(i + 1) % 4];
        let (mut ex, mut ey) = (p2.x - p1.x, p2.y - p1.y);
        let len = ex.hypot(ey);
        if len < 1e-9 {
            ex = 1.0;
            ey = 0.0;
        } else {
            ex /= len;
            ey /= len;
        }
        let (mut nx, mut ny) = (ey, -ex);
        let (mx, my) = ((p1.x + p2.x) / 2.0, (p1.y + p2.y) / 2.0);
        if (mx - cx) * nx + (my - cy) * ny < 0.0 {
            nx = -nx;
            ny = -ny;
        }
        lines[i] = Line {
            ax: p1.x + nx * d,
            ay: p1.y + ny * d,
            ex,
            ey,
        };
    }

    let mut out = vec![Pt::new(0.0, 0.0); 4];
    for i in 0..4 {
        let prev = &lines[(i + 3) % 4];
        let cur = &lines[i];
        let denom = prev.ex * cur.ey - prev.ey * cur.ex;
        if denom.abs() < 1e-9 {
            out[i] = Pt::new(cur.ax, cur.ay);
            continue;
        }
        let (dx, dy) = (cur.ax - prev.ax, cur.ay - prev.ay);
        let t = (dx * cur.ey - dy * cur.ex) / denom;
        out[i] = Pt::new(prev.ax + t * prev.ex, prev.ay + t * prev.ey);
    }
    out
}

fn point_in_poly(x: f64, y: f64, poly: &[Pt]) -> bool {
    let mut inside = false;
    let mut j = poly.len() - 1;
    for i in 0..poly.len() {
        let (xi, yi) = (poly[i].x, poly[i].y);
        let (xj, yj) = (poly[j].x, poly[j].y);
        if (yi > y) != (yj > y) && x < (xj - xi) * (y - yi) / (yj - yi) + xi {
            inside = !inside;
        }
        j = i;
    }
    inside
}

fn box_score_fast(pred: &[f32], h: usize, w: usize, square: &[Pt; 4]) -> f64 {
    let (mut min_x, mut max_x, mut min_y, mut max_y) = (
        f64::INFINITY,
        f64::NEG_INFINITY,
        f64::INFINITY,
        f64::NEG_INFINITY,
    );
    for p in square {
        min_x = min_x.min(p.x);
        max_x = max_x.max(p.x);
        min_y = min_y.min(p.y);
        max_y = max_y.max(p.y);
    }
    let x0 = min_x.floor().max(0.0) as usize;
    let x1 = max_x.ceil().min((w - 1) as f64) as usize;
    let y0 = min_y.floor().max(0.0) as usize;
    let y1 = max_y.ceil().min((h - 1) as f64) as usize;
    if x1 < x0 || y1 < y0 {
        return 0.0;
    }
    let mut sum = 0.0;
    let mut cnt = 0usize;
    for y in y0..=y1 {
        for x in x0..=x1 {
            if point_in_poly(x as f64, y as f64, square) {
                sum += pred[y * w + x] as f64;
                cnt += 1;
            }
        }
    }
    if cnt == 0 {
        0.0
    } else {
        sum / cnt as f64
    }
}

fn order_points_clockwise(b: [[f64; 2]; 4]) -> [[f64; 2]; 4] {
    let mut pts: Vec<Pt> = b.iter().map(|p| Pt::new(p[0], p[1])).collect();
    pts.sort_by(cmp_x);
    let mut left = pts[..2].to_vec();
    let mut right = pts[2..].to_vec();
    left.sort_by(|a, b| a.y.partial_cmp(&b.y).unwrap_or(Ordering::Equal));
    right.sort_by(|a, b| a.y.partial_cmp(&b.y).unwrap_or(Ordering::Equal));
    let out = [left[0], right[0], right[1], left[1]];
    let mut r = [[0f64; 2]; 4];
    for (i, p) in out.iter().enumerate() {
        r[i] = [p.x, p.y];
    }
    r
}

/// 对应 TextDetector.filter_tag_det_res：排序、裁剪、过滤过小框。
pub fn filter_boxes(
    boxes: Vec<[[f64; 2]; 4]>,
    img_w: usize,
    img_h: usize,
) -> Vec<[[f64; 2]; 4]> {
    let mut out = Vec::with_capacity(boxes.len());
    for raw in boxes {
        let mut b = order_points_clockwise(raw);
        for p in b.iter_mut() {
            p[0] = p[0].clamp(0.0, (img_w - 1) as f64);
            p[1] = p[1].clamp(0.0, (img_h - 1) as f64);
        }
        let rw = (b[0][0] - b[1][0]).hypot(b[0][1] - b[1][1]);
        let rh = (b[0][0] - b[3][0]).hypot(b[0][1] - b[3][1]);
        if rw <= 3.0 || rh <= 3.0 {
            continue;
        }
        out.push(b);
    }
    out.sort_by(|a, b| {
        if (a[0][1] - b[0][1]).abs() > 10.0 {
            a[0][1].partial_cmp(&b[0][1]).unwrap_or(Ordering::Equal)
        } else {
            a[0][0].partial_cmp(&b[0][0]).unwrap_or(Ordering::Equal)
        }
    });
    out
}
