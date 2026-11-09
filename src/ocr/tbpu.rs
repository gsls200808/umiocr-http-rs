use std::cmp::Ordering;

use super::types::{IgnoreRect, TextBlock};

/// 标准化包围盒（轴对齐）与索引，用于排序与分行。
#[derive(Debug, Clone, Copy)]
struct Nb {
    idx: usize,
    x0: f64,
    y0: f64,
    x1: f64,
    y1: f64,
    cy: f64,
    h: f64,
}

fn normalize(blocks: &[TextBlock]) -> Vec<Nb> {
    blocks
        .iter()
        .enumerate()
        .map(|(idx, b)| {
            let (mut x0, mut y0, mut x1, mut y1) = (0.0, 0.0, 0.0, 0.0);
            if let Some(first) = b.box_.first() {
                x0 = first[0];
                y0 = first[1];
                x1 = x0;
                y1 = y0;
                for p in &b.box_ {
                    x0 = x0.min(p[0]);
                    x1 = x1.max(p[0]);
                    y0 = y0.min(p[1]);
                    y1 = y1.max(p[1]);
                }
            }
            let cy = (y0 + y1) / 2.0;
            let mut h = y1 - y0;
            if h <= 0.0 {
                h = 1.0;
            }
            Nb {
                idx,
                x0,
                y0,
                x1,
                y1,
                cy,
                h,
            }
        })
        .collect()
}

fn first_rune(s: &str) -> char {
    s.chars().next().unwrap_or('\0')
}

fn last_rune(s: &str) -> char {
    s.chars().last().unwrap_or('\0')
}

/// 判定是否为中日韩及全角字符（与 Umi-OCR word_separator 一致）。
fn is_cjk(r: char) -> bool {
    let c = r as u32;
    const RANGES: [[u32; 2]; 8] = [
        [0x4E00, 0x9FFF],
        [0x3040, 0x30FF],
        [0x1100, 0x11FF],
        [0x3130, 0x318F],
        [0xAC00, 0xD7AF],
        [0x3000, 0x303F],
        [0xFE30, 0xFE4F],
        [0xFF00, 0xFFEF],
    ];
    RANGES.iter().any(|rg| c >= rg[0] && c <= rg[1])
}

fn is_punct(r: char) -> bool {
    if r.is_ascii_punctuation() {
        return true;
    }
    let c = r as u32;
    (0x3000..=0x303F).contains(&c) || (0xFE30..=0xFE4F).contains(&c) || (0xFF01..=0xFF20).contains(&c) || (0xFF3B..=0xFF40).contains(&c) || (0xFF5B..=0xFF65).contains(&c)
}

/// 预测相邻文本块之间的间隔符（对应 Umi-OCR paragraph_parse.word_separator）。
fn word_separator(l1: char, l2: char) -> String {
    if is_cjk(l1) && is_cjk(l2) {
        return String::new();
    }
    if l1 == '-' {
        return String::new();
    }
    if is_punct(l2) {
        return String::new();
    }
    " ".to_string()
}

/// 将文块按 "text + end" 拼接为纯文本。
pub fn get_data_text(blocks: &[TextBlock]) -> String {
    let mut out = String::new();
    for b in blocks {
        out.push_str(&b.text);
        out.push_str(&b.end);
    }
    out
}

/// 剔除落在忽略矩形内的文本块（与 Umi-OCR IgnoreArea 一致）。
pub fn apply_ignore_area(blocks: Vec<TextBlock>, rects: &[IgnoreRect]) -> Vec<TextBlock> {
    if rects.is_empty() {
        return blocks;
    }
    let mut out = Vec::with_capacity(blocks.len());
    'outer: for b in blocks {
        for r in rects {
            if b.box_.len() >= 3
                && r.x1 <= b.box_[0][0]
                && r.y1 <= b.box_[0][1]
                && r.x2 >= b.box_[2][0]
                && r.y2 >= b.box_[2][1]
            {
                continue 'outer;
            }
        }
        out.push(b);
    }
    out
}

fn cmp_f64(a: f64, b: f64) -> Ordering {
    a.partial_cmp(&b).unwrap_or(Ordering::Equal)
}

/// 执行排版解析：排序、分行、设置每个文块的结尾分隔符 end。
/// 实现 Umi-OCR 的 8 种 tbpu.parser，多栏方案用简单的间隙分栏近似 GapTree。
pub fn apply_parser(mut blocks: Vec<TextBlock>, parser: &str) -> Vec<TextBlock> {
    if blocks.is_empty() {
        return blocks;
    }

    if parser == "none" {
        for b in blocks.iter_mut() {
            b.end = "\n".to_string();
        }
        return blocks;
    }

    let nbs = normalize(&blocks);
    let mut heights: Vec<f64> = nbs.iter().map(|n| n.h).collect();
    heights.sort_by(|a, b| cmp_f64(*a, *b));
    let mut med_h = heights[heights.len() / 2];
    if med_h <= 0.0 {
        med_h = 10.0;
    }

    let multi = parser.starts_with("multi_");
    let mut lines = group_lines(&nbs, multi, med_h);

    match parser {
        "single_none" | "multi_none" => {
            let flat = flatten(&lines);
            let n = flat.len();
            for i in 0..n {
                let idx = flat[i];
                let end = if i + 1 == n {
                    "\n".to_string()
                } else {
                    word_separator(
                        last_rune(&blocks[idx].text),
                        first_rune(&blocks[flat[i + 1]].text),
                    )
                };
                blocks[idx].end = end;
            }
        }
        "single_para" | "multi_para" => {
            for li in 0..lines.len() {
                let len = lines[li].len();
                for i in 0..len {
                    let idx = lines[li][i].idx;
                    let end = if i + 1 < len {
                        word_separator(
                            last_rune(&blocks[idx].text),
                            first_rune(&blocks[lines[li][i + 1].idx].text),
                        )
                    } else {
                        "\n".to_string()
                    };
                    blocks[idx].end = end;
                }
                if li + 1 < lines.len() && same_paragraph(&lines[li], &lines[li + 1], med_h) {
                    let a = lines[li].last().unwrap().idx;
                    let b = lines[li + 1][0].idx;
                    let sep = word_separator(last_rune(&blocks[a].text), first_rune(&blocks[b].text));
                    blocks[a].end = sep;
                }
            }
        }
        "single_code" => {
            let mut min_x = f64::INFINITY;
            for line in &lines {
                if let Some(first) = line.first() {
                    if first.x0 < min_x {
                        min_x = first.x0;
                    }
                }
            }
            for li in 0..lines.len() {
                if lines[li].is_empty() {
                    continue;
                }
                let line = lines[li].clone();
                merge_code_line(&line, &mut blocks);
                let level = ((line[0].x0 - min_x) / med_h).round().max(0.0) as usize;
                let idx = line[0].idx;
                let prefixed = "  ".repeat(level) + &blocks[idx].text;
                blocks[idx].text = prefixed;
                lines[li].truncate(1);
            }
        }
        _ => {
            // single_line, multi_line 及未知值：逐行换行
            for line in &lines {
                let len = line.len();
                for i in 0..len {
                    let idx = line[i].idx;
                    let end = if i + 1 == len {
                        "\n".to_string()
                    } else {
                        word_separator(
                            last_rune(&blocks[idx].text),
                            first_rune(&blocks[line[i + 1].idx].text),
                        )
                    };
                    blocks[idx].end = end;
                }
            }
        }
    }

    flatten(&lines).into_iter().map(|i| blocks[i].clone()).collect()
}

/// 将文块按阅读顺序组织为行，每行内按 x 排序。
fn group_lines(nbs: &[Nb], multi: bool, med_h: f64) -> Vec<Vec<Nb>> {
    let columns: Vec<Vec<Nb>> = if multi {
        split_columns(nbs, med_h, 0)
    } else {
        vec![nbs.to_vec()]
    };

    let mut lines: Vec<Vec<Nb>> = Vec::new();
    for mut col in columns {
        col.sort_by(|a, b| {
            if (a.y0 - b.y0).abs() > med_h * 0.5 {
                cmp_f64(a.y0, b.y0)
            } else {
                cmp_f64(a.x0, b.x0)
            }
        });

        let mut cur: Vec<Nb> = Vec::new();
        let (mut cur_top, mut cur_bottom) = (0.0, 0.0);
        for nb in col {
            if cur.is_empty() {
                cur.push(nb);
                cur_top = nb.y0;
                cur_bottom = nb.y1;
                continue;
            }
            let overlap = cur_bottom.min(nb.y1) - cur_top.max(nb.y0);
            let last_cy = cur.last().unwrap().cy;
            if overlap > med_h * 0.3 || (nb.cy - last_cy).abs() < med_h * 0.6 {
                cur.push(nb);
                cur_top = cur_top.min(nb.y0);
                cur_bottom = cur_bottom.max(nb.y1);
            } else {
                cur.sort_by(|a, b| cmp_f64(a.x0, b.x0));
                lines.push(std::mem::take(&mut cur));
                cur_top = nb.y0;
                cur_bottom = nb.y1;
                cur.push(nb);
            }
        }
        if !cur.is_empty() {
            cur.sort_by(|a, b| cmp_f64(a.x0, b.x0));
            lines.push(cur);
        }
    }
    lines
}

/// 用水平间隙递归分栏，左栏在前（近似 GapTree 的多栏排序）。
fn split_columns(nbs: &[Nb], med_h: f64, depth: usize) -> Vec<Vec<Nb>> {
    if depth > 2 || nbs.len() < 4 {
        return vec![nbs.to_vec()];
    }
    let mut sorted = nbs.to_vec();
    sorted.sort_by(|a, b| cmp_f64(a.x0, b.x0));

    let mut max_gap = 0.0;
    let mut split_idx: Option<usize> = None;
    for i in 1..sorted.len() {
        let gap = sorted[i].x0 - sorted[i - 1].x1;
        if gap > max_gap {
            max_gap = gap;
            split_idx = Some(i);
        }
    }
    let Some(si) = split_idx else {
        return vec![nbs.to_vec()];
    };
    if max_gap < med_h * 1.5 {
        return vec![nbs.to_vec()];
    }

    let threshold = (sorted[si - 1].x1 + sorted[si].x0) / 2.0;
    let mut left = Vec::new();
    let mut right = Vec::new();
    for nb in nbs {
        if nb.x0 + (nb.x1 - nb.x0) / 2.0 < threshold {
            left.push(*nb);
        } else {
            right.push(*nb);
        }
    }
    if left.is_empty() || right.is_empty() {
        return vec![nbs.to_vec()];
    }
    let mut out = split_columns(&left, med_h, depth + 1);
    out.extend(split_columns(&right, med_h, depth + 1));
    out
}

fn same_paragraph(a: &[Nb], b: &[Nb], med_h: f64) -> bool {
    if a.is_empty() || b.is_empty() {
        return false;
    }
    let mut bottom1 = a[0].y1;
    for nb in a {
        bottom1 = bottom1.max(nb.y1);
    }
    let mut top2 = b[0].y0;
    for nb in b {
        top2 = top2.min(nb.y0);
    }
    (a[0].x0 - b[0].x0).abs() <= med_h * 1.2 && (top2 - bottom1) < med_h * 1.5
}

fn merge_code_line(line: &[Nb], blocks: &mut [TextBlock]) {
    if line.is_empty() {
        return;
    }
    let a = line[0];
    let mut text = blocks[a.idx].text.clone();
    let mut quad = blocks[a.idx].box_.clone();
    for i in 1..line.len() {
        let space = ((line[i].x0 - line[i - 1].x1) / a.h).round().max(1.0) as usize;
        text.push_str(&" ".repeat(space));
        text.push_str(&blocks[line[i].idx].text);
        let other = &blocks[line[i].idx];
        if quad.len() >= 4 && other.box_.len() >= 4 {
            quad[1][0] = quad[1][0].max(other.box_[1][0]);
            quad[2][0] = quad[2][0].max(other.box_[2][0]);
            quad[2][1] = quad[2][1].max(other.box_[2][1]);
            quad[3][1] = quad[3][1].max(other.box_[3][1]);
        }
    }
    let b = &mut blocks[a.idx];
    b.text = text;
    b.end = "\n".to_string();
    b.box_ = quad;
}

/// 将行序列展开为有序的文块索引。
fn flatten(lines: &[Vec<Nb>]) -> Vec<usize> {
    let mut out = Vec::new();
    for line in lines {
        for nb in line {
            out.push(nb.idx);
        }
    }
    out
}
