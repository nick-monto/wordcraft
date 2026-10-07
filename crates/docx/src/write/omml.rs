//! [`wordcraft_doc::math::MathNode`] → OMML (`m:*`).

use wordcraft_doc::math::{AccentKind, MathNode, MathStyle, cmd_symbol};

use crate::xml::W;

/// Deepest math tree we walk. A parsed tree is far shallower; the cap only bounds stray input.
const MAX_DEPTH: usize = 64;

/// Emit one equation, parsing `source` (LaTeX or Word-linear) into structured OMML.
///
/// `display` selects `m:oMathPara` (a display equation) over a bare `m:oMath`. A source the math
/// parser rejects keeps the flat `m:r`/`m:t` shape, so its text still survives a round trip.
pub(super) fn equation(w: &mut W, source: &str, display: bool) {
    if display {
        w.open("m:oMathPara", &[]);
    }
    w.open("m:oMath", &[]);
    match wordcraft_doc::math::parse(source) {
        Ok(ast) => emit(w, &ast, 0, None),
        Err(_) => run(w, source, MathStyle::Italic),
    }
    w.close("m:oMath");
    if display {
        w.close("m:oMathPara");
    }
}

/// Emit `n` as siblings inside the current element. `over` carries the style of an enclosing
/// `\mathbf`/`\mathrm` group down onto the text runs it covers.
fn emit(w: &mut W, n: &MathNode, depth: usize, over: Option<MathStyle>) {
    if depth > MAX_DEPTH {
        return;
    }
    match n {
        MathNode::Row(nodes) => {
            for c in nodes {
                emit(w, c, depth + 1, over);
            }
        }
        MathNode::Text(s, style) => {
            if !s.is_empty() {
                run(w, s, over.unwrap_or(*style));
            }
        }
        // OMML has no standalone space; a non-breaking space is the simplest stand-in (the exact
        // em width is not preserved).
        MathNode::Space(_) => run(w, "\u{a0}", MathStyle::Italic),
        MathNode::Frac { num, den } => {
            w.open("m:f", &[]);
            slot(w, "m:num", num, depth, over);
            slot(w, "m:den", den, depth, over);
            w.close("m:f");
        }
        MathNode::Sqrt { index, body } => {
            w.open("m:rad", &[]);
            if index.is_none() {
                w.open("m:radPr", &[]);
                w.empty("m:degHide", &[("m:val", "1")]);
                w.close("m:radPr");
            }
            if let Some(idx) = index {
                slot(w, "m:deg", idx, depth, over);
            }
            slot(w, "m:e", body, depth, over);
            w.close("m:rad");
        }
        MathNode::Script { base, sub, sup } => match (sub, sup) {
            (Some(sub), Some(sup)) => {
                w.open("m:sSubSup", &[]);
                slot(w, "m:e", base, depth, over);
                slot(w, "m:sub", sub, depth, over);
                slot(w, "m:sup", sup, depth, over);
                w.close("m:sSubSup");
            }
            (Some(sub), None) => {
                w.open("m:sSub", &[]);
                slot(w, "m:e", base, depth, over);
                slot(w, "m:sub", sub, depth, over);
                w.close("m:sSub");
            }
            (None, Some(sup)) => {
                w.open("m:sSup", &[]);
                slot(w, "m:e", base, depth, over);
                slot(w, "m:sup", sup, depth, over);
                w.close("m:sSup");
            }
            (None, None) => emit(w, base, depth + 1, over),
        },
        MathNode::Big { op, sub, sup, limits } => {
            w.open("m:nary", &[]);
            w.open("m:naryPr", &[]);
            w.empty("m:chr", &[("m:val", cmd_symbol(op).unwrap_or(op.as_str()))]);
            if *limits {
                w.empty("m:limLoc", &[("m:val", "undOvr")]);
            }
            w.close("m:naryPr");
            if let Some(sub) = sub {
                slot(w, "m:sub", sub, depth, over);
            }
            if let Some(sup) = sup {
                slot(w, "m:sup", sup, depth, over);
            }
            w.empty("m:e", &[]);
            w.close("m:nary");
        }
        MathNode::Delim { left, right, body } => {
            w.open("m:d", &[]);
            delim_pr(w, *left, *right);
            slot(w, "m:e", body, depth, over);
            w.close("m:d");
        }
        // Word has one bar; `m:pos` splits under- from overline, and an explicit "top" (vs. an
        // omitted `m:pos`) keeps `\bar` and `\overline` apart on read back.
        MathNode::Accent { kind, body } => match kind {
            AccentKind::Bar => bar(w, body, None, depth, over),
            AccentKind::Overline => bar(w, body, Some("top"), depth, over),
            AccentKind::Underline => bar(w, body, Some("bot"), depth, over),
            _ => acc(w, body, *kind, depth, over),
        },
        MathNode::Matrix { rows, left, right } => {
            if *left == '.' && *right == '.' {
                matrix(w, rows, depth, over);
            } else {
                w.open("m:d", &[]);
                delim_pr(w, *left, *right);
                w.open("m:e", &[]);
                matrix(w, rows, depth, over);
                w.close("m:e");
                w.close("m:d");
            }
        }
        MathNode::Styled { style, body } => emit(w, body, depth + 1, Some(*style)),
    }
}

/// `<tag>` wrapping the OMML of `n`.
fn slot(w: &mut W, tag: &str, n: &MathNode, depth: usize, over: Option<MathStyle>) {
    w.open(tag, &[]);
    emit(w, n, depth + 1, over);
    w.close(tag);
}

fn matrix(w: &mut W, rows: &[Vec<MathNode>], depth: usize, over: Option<MathStyle>) {
    w.open("m:m", &[]);
    for row in rows {
        w.open("m:mr", &[]);
        for cell in row {
            slot(w, "m:e", cell, depth, over);
        }
        w.close("m:mr");
    }
    w.close("m:m");
}

/// An over- or under-bar (`\bar`, `\overline`, `\underline`). `pos` is `None`, `"top"` or `"bot"`.
fn bar(w: &mut W, body: &MathNode, pos: Option<&str>, depth: usize, over: Option<MathStyle>) {
    w.open("m:bar", &[]);
    if let Some(p) = pos {
        w.open("m:barPr", &[]);
        w.empty("m:pos", &[("m:val", p)]);
        w.close("m:barPr");
    }
    slot(w, "m:e", body, depth, over);
    w.close("m:bar");
}

/// A combining accent (`\hat`, `\tilde`, `\dot`, `\ddot`, `\vec`).
fn acc(w: &mut W, body: &MathNode, kind: AccentKind, depth: usize, over: Option<MathStyle>) {
    w.open("m:acc", &[]);
    w.open("m:accPr", &[]);
    w.empty("m:chr", &[("m:val", accent_chr(kind))]);
    w.close("m:accPr");
    slot(w, "m:e", body, depth, over);
    w.close("m:acc");
}

/// `m:dPr` with the delimiters that differ from the default `( )`.
///
/// A default side is omitted; an invisible `.` side is written as an empty `m:val` (Word's own
/// convention), which keeps `( )`, `( ·` and `· ·` distinct when read back.
fn delim_pr(w: &mut W, left: char, right: char) {
    let beg = delim_val(left, '(');
    let end = delim_val(right, ')');
    if beg.is_none() && end.is_none() {
        return;
    }
    w.open("m:dPr", &[]);
    if let Some(v) = beg {
        w.empty("m:begChr", &[("m:val", v.as_str())]);
    }
    if let Some(v) = end {
        w.empty("m:endChr", &[("m:val", v.as_str())]);
    }
    w.close("m:dPr");
}

/// `None` for the default delimiter (omitted); `Some("")` for the invisible `.`.
fn delim_val(c: char, default: char) -> Option<String> {
    if c == '.' {
        Some(String::new())
    } else if c == default {
        None
    } else {
        Some(c.to_string())
    }
}

/// The combining character Word uses for an accent.
fn accent_chr(kind: AccentKind) -> &'static str {
    match kind {
        AccentKind::Tilde => "\u{0303}",
        AccentKind::Dot => "\u{0307}",
        AccentKind::Ddot => "\u{0308}",
        AccentKind::Vec => "\u{20D7}",
        _ => "\u{0302}",
    }
}

/// A single math run. Italic is OMML's default, so it needs no `m:sty`.
fn run(w: &mut W, text: &str, style: MathStyle) {
    w.open("m:r", &[]);
    let sty = match style {
        MathStyle::Italic => None,
        MathStyle::Roman => Some("p"),
        MathStyle::Bold => Some("b"),
        MathStyle::BoldItalic => Some("bi"),
    };
    if let Some(v) = sty {
        w.open("m:rPr", &[]);
        w.empty("m:sty", &[("m:val", v)]);
        w.close("m:rPr");
    }
    w.leaf("m:t", &[("xml:space", "preserve")], text);
    w.close("m:r");
}
