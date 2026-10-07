//! OMML (`m:*`) → [`wordcraft_doc::math::MathNode`].
//!
//! The bridge is structural: every construct our writer emits maps back onto the same
//! [`MathNode`], and anything unrecognised falls back to its concatenated `m:t` text so no
//! visible content is lost. Recursion is capped ([`MAX_DEPTH`]) so hostile nesting cannot
//! overflow the stack.

use wordcraft_doc::math::{AccentKind, MathNode, MathStyle, symbol_cmd};

use crate::xml::El;

/// Deepest `m:*` nesting we descend.
pub(super) const MAX_DEPTH: usize = 32;

/// The math tree for the contents of `el` (usually `m:oMath`, sometimes an `m:e`).
pub(super) fn tree(el: &El) -> MathNode {
    collapse(nodes(el, 0))
}

/// Collapse a statement list the way [`wordcraft_doc::math`] does: one node stays bare, several
/// become a [`MathNode::Row`].
fn collapse(mut nodes: Vec<MathNode>) -> MathNode {
    if nodes.len() == 1 {
        match nodes.pop() {
            Some(n) => n,
            None => MathNode::Row(Vec::new()),
        }
    } else {
        MathNode::Row(nodes)
    }
}

/// Is this a property element (skipped in content position)?
fn prop_name(name: &str) -> bool {
    matches!(
        name,
        "m:rPr"
            | "m:ctrlPr"
            | "m:fPr"
            | "m:radPr"
            | "m:dPr"
            | "m:naryPr"
            | "m:accPr"
            | "m:barPr"
            | "m:sSupPr"
            | "m:sSubPr"
            | "m:sSubSupPr"
            | "m:mPr"
            | "m:oMathParaPr"
    )
}

/// The text of every `m:t` in this subtree, concatenated (the lossless fallback).
fn flat_text(el: &El) -> String {
    let mut s = String::new();
    fn go(e: &El, s: &mut String, depth: usize) {
        if depth > MAX_DEPTH {
            return;
        }
        for c in e.els() {
            if c.name == "m:t" {
                s.push_str(&c.text());
            } else {
                go(c, s, depth + 1);
            }
        }
    }
    go(el, &mut s, 0);
    s
}

fn fallback(el: &El, out: &mut Vec<MathNode>) {
    let t = flat_text(el);
    if !t.is_empty() {
        out.push(MathNode::Text(t, MathStyle::Italic));
    }
}

/// The nodes of a content element (the children of `m:oMath`, `m:e`, `m:num`, …).
fn nodes(el: &El, depth: usize) -> Vec<MathNode> {
    let mut out = Vec::new();
    if depth > MAX_DEPTH {
        fallback(el, &mut out);
        return out;
    }
    for c in el.els() {
        if !prop_name(&c.name) {
            elem(c, depth + 1, &mut out);
        }
    }
    out
}

/// The single node for `el`, or an empty [`MathNode::Row`] when absent.
fn content(el: Option<&El>, depth: usize) -> MathNode {
    el.map_or_else(|| MathNode::Row(Vec::new()), |e| collapse(nodes(e, depth)))
}

/// Whether a node is empty (draws nothing).
fn is_empty(n: &MathNode) -> bool {
    matches!(n, MathNode::Row(v) if v.is_empty())
}

/// A sub/sup slot: `None` when absent or empty.
fn slot(el: Option<&El>, depth: usize) -> Option<Box<MathNode>> {
    let n = content(el, depth);
    if is_empty(&n) { None } else { Some(Box::new(n)) }
}

fn truthy(v: Option<&str>) -> bool {
    matches!(v, Some("1" | "true" | "on"))
}

fn elem(e: &El, depth: usize, out: &mut Vec<MathNode>) {
    if depth > MAX_DEPTH {
        fallback(e, out);
        return;
    }
    match e.name.as_str() {
        "m:r" => {
            let t = run_text(e);
            if !t.is_empty() {
                out.push(MathNode::Text(t, run_style(e)));
            }
        }
        // A bare `m:t` outside a run: keep it as text.
        "m:t" => {
            let t = e.text();
            if !t.is_empty() {
                out.push(MathNode::Text(t, MathStyle::Italic));
            }
        }
        "m:f" => {
            let num = Box::new(content(e.child("m:num"), depth));
            let den = Box::new(content(e.child("m:den"), depth));
            out.push(MathNode::Frac { num, den });
        }
        "m:rad" => out.push(read_rad(e, depth)),
        "m:sSup" | "m:sSub" | "m:sSubSup" => out.push(read_script(e, depth)),
        "m:nary" => read_nary(e, depth, out),
        "m:d" => read_delim(e, depth, out),
        "m:acc" => {
            let kind = accent_kind(e.child("m:accPr").and_then(|p| p.child("m:chr")).and_then(|c| c.attr("m:val")));
            out.push(MathNode::Accent { kind, body: Box::new(content(e.child("m:e"), depth)) });
        }
        "m:bar" => {
            let kind = bar_kind(e.child("m:barPr").and_then(|p| p.child("m:pos")).and_then(|c| c.attr("m:val")));
            out.push(MathNode::Accent { kind, body: Box::new(content(e.child("m:e"), depth)) });
        }
        "m:m" => out.push(read_matrix(e, depth, '.', '.')),
        "m:oMath" => out.extend(nodes(e, depth)),
        _ => fallback(e, out),
    }
}

fn run_text(e: &El) -> String {
    let mut s = String::new();
    for t in e.children("m:t") {
        s.push_str(&t.text());
    }
    if s.is_empty() {
        // Tolerate producers that put the text directly under `m:r`.
        s = e.text();
    }
    s
}

fn run_style(e: &El) -> MathStyle {
    match e.child("m:rPr").and_then(|p| p.child("m:sty")).and_then(|s| s.attr("m:val")) {
        Some("p") => MathStyle::Roman,
        Some("b") => MathStyle::Bold,
        Some("bi") => MathStyle::BoldItalic,
        _ => MathStyle::Italic,
    }
}

fn read_rad(e: &El, depth: usize) -> MathNode {
    let hidden = is_deg_hidden(e.child("m:radPr"));
    let index = if hidden { None } else { slot(e.child("m:deg"), depth) };
    MathNode::Sqrt { index, body: Box::new(content(e.child("m:e"), depth)) }
}

fn is_deg_hidden(radpr: Option<&El>) -> bool {
    radpr.and_then(|p| p.child("m:degHide")).is_some_and(|h| truthy(h.attr("m:val")))
}

fn read_script(e: &El, depth: usize) -> MathNode {
    let base = Box::new(content(e.child("m:e"), depth));
    let sub = slot(e.child("m:sub"), depth);
    let sup = slot(e.child("m:sup"), depth);
    match (sub, sup) {
        (None, None) => *base,
        (sub, sup) => MathNode::Script { base, sub, sup },
    }
}

fn read_nary(e: &El, depth: usize, out: &mut Vec<MathNode>) {
    let pr = e.child("m:naryPr");
    let chr = pr.and_then(|p| p.child("m:chr")).and_then(|c| c.attr("m:val")).unwrap_or("");
    let limits = pr.and_then(|p| p.child("m:limLoc")).and_then(|c| c.attr("m:val")) == Some("undOvr");
    let op = op_for_glyph(chr);
    let sub = slot(e.child("m:sub"), depth);
    let sup = slot(e.child("m:sup"), depth);
    let big = MathNode::Big { op, sub, sup, limits };
    // `Big` has no operand of its own; keep a foreign one as a sibling so it isn't lost.
    let body = content(e.child("m:e"), depth);
    let nodes = if is_empty(&body) { vec![big] } else { vec![big, body] };
    out.push(MathNode::Row(nodes));
}

/// The command for an `m:chr` glyph: a single character names its symbol; a word is the command
/// itself (`"lim"`, `"sin"`, …); nothing defaults to the n-ary `int`.
fn op_for_glyph(g: &str) -> String {
    let mut chars = g.chars();
    match (chars.next(), chars.next()) {
        (Some(c), None) => symbol_cmd(c).unwrap_or(g).to_string(),
        (Some(_), Some(_)) => g.to_string(),
        _ => "int".to_string(),
    }
}

fn read_delim(e: &El, depth: usize, out: &mut Vec<MathNode>) {
    let (left, right) = delim_chars(e.child("m:dPr"));
    let body = e.child("m:e");
    match matrix_child(body) {
        Some(m) => out.push(read_matrix(m, depth, left, right)),
        None => out.push(MathNode::Delim { left, right, body: Box::new(content(body, depth)) }),
    }
}

/// The `m:m` inside an `m:e` (which marks a delimited matrix rather than a plain delimiter).
fn matrix_child(e: Option<&El>) -> Option<&El> {
    let e = e?;
    let mut kids = e.els().filter(|c| !prop_name(&c.name));
    let first = kids.next()?;
    (first.name == "m:m" && kids.next().is_none()).then_some(first)
}

/// A default side is absent; an empty `m:val` means the invisible `.`.
fn delim_chars(dpr: Option<&El>) -> (char, char) {
    (delim_side(dpr, "m:begChr", '('), delim_side(dpr, "m:endChr", ')'))
}

fn delim_side(dpr: Option<&El>, name: &str, default: char) -> char {
    let Some(el) = dpr.and_then(|p| p.child(name)) else { return default };
    match el.attr("m:val") {
        Some("") | Some(".") => '.',
        Some(v) => v.chars().next().unwrap_or(default),
        None => default,
    }
}

fn read_matrix(e: &El, depth: usize, left: char, right: char) -> MathNode {
    let mut rows = Vec::new();
    for mr in e.children("m:mr") {
        let row = mr.children("m:e").map(|cell| content(Some(cell), depth)).collect();
        rows.push(row);
    }
    MathNode::Matrix { rows, left, right }
}

fn accent_kind(chr: Option<&str>) -> AccentKind {
    match chr {
        Some("\u{0303}") => AccentKind::Tilde,
        Some("\u{0307}") => AccentKind::Dot,
        Some("\u{0308}") => AccentKind::Ddot,
        Some("\u{20D7}") => AccentKind::Vec,
        _ => AccentKind::Hat,
    }
}

fn bar_kind(pos: Option<&str>) -> AccentKind {
    match pos {
        Some("bot") => AccentKind::Underline,
        Some("top") => AccentKind::Overline,
        _ => AccentKind::Bar,
    }
}
