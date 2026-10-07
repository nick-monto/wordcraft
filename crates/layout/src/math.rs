//! Math typesetting: a [`MathNode`] tree (see `wordcraft_doc::math`) laid out as positioned glyphs
//! and rules.
//!
//! TeX-like, with the constants below as fractions of the size `S` in force at a node: scripts at
//! `0.7*S`, script-scripts at `0.5*S`, the axis (the fraction bar's height) at `0.25*S`, rules at
//! `0.06*S` (never thinner than 0.4 pt). A math font's size variants are approximated by scaling
//! the glyph itself (radicals, stretchy delimiters) — as close as a single font file gets.
//!
//! Everything is in points. A [`MathBox`] is measured from its left edge and its own baseline:
//! `x` grows right, `y` grows up, and `ascent`/`descent` cover every glyph and rule in it. Callers
//! place a box by that baseline (see `para` and `display`).
//!
//! Parsing and typesetting are total: a hostile source can be deep, long or full of characters no
//! font has, and none of it panics — the work is bounded by [`MAX_DEPTH`], [`MAX_GLYPHS`] and
//! [`MAX_RULES`], and anything that cannot be drawn is dropped rather than drawn as a tofu box.

use std::collections::HashMap;

use wordcraft_doc::math::{self, AccentKind, MathError, MathNode, MathStyle};
use wordcraft_fonts::word::{self, Resolved};
use wordcraft_fonts::{FaceRef, FontDb, FontFace};

/// One positioned glyph of a typeset equation.
#[derive(Clone, Debug)]
pub struct MathGlyph {
    pub face: FaceRef,
    pub size: f32,
    pub synth_bold: bool,
    pub synth_italic: bool,
    pub gid: u32,
    pub x: f32,
    pub y: f32,
}

/// A horizontal rule (fraction bar, radical overbar, `\bar`, matrix separators).
#[derive(Clone, Copy, Debug)]
pub struct MathRule {
    pub x0: f32,
    pub x1: f32,
    pub y: f32,
    pub thickness: f32,
}

/// A typeset equation: `x` from the box's left edge, `y` from the baseline, POSITIVE UP.
#[derive(Clone, Debug, Default)]
pub struct MathBox {
    pub w: f32,
    pub ascent: f32,
    pub descent: f32,
    pub glyphs: Vec<MathGlyph>,
    pub rules: Vec<MathRule>,
}

/// One equation in a paragraph: the box laid out for cluster `cluster`.
#[derive(Clone, Debug)]
pub struct MathItem {
    pub cluster: usize,
    pub layout: MathBox,
}

/// The family symbols, operators, delimiters and accents are taken from; a document asks for this
/// name, so an installation without it substitutes a serif face (see `wordcraft_fonts::word`).
const FAMILY: &str = "Cambria Math";

/// Script size as a fraction of the size in force.
const SCRIPT: f32 = 0.7;
/// Script-script size: scripts of scripts, and radicals' indices.
const SCRIPTSCRIPT: f32 = 0.5;
/// The axis: the fraction bar's height above the baseline.
const AXIS: f32 = 0.25;
/// Gap between a fraction bar and the ink above and below it.
const FRAC_GAP: f32 = 0.1;
/// Superscript raise, below the clearance check against the base.
const SUP_RAISE: f32 = 0.45;
/// Subscript drop, above the clearance check against the base.
const SUB_LOWER: f32 = 0.15;
/// Ink clearance kept when a script has to move further to clear its base.
const CLEAR: f32 = 0.05;
/// Gap between a big operator and its limits.
const LIMIT_GAP: f32 = 0.15;
/// Gap between a radicand and the bar over it.
const RADICAL_GAP: f32 = 0.1;
/// Gap between an accent and the top of its base.
const ACCENT_GAP: f32 = 0.05;
/// Big operators (sums, integrals) are set at this multiple of the size in force.
const BIG_SCALE: f32 = 1.4;
/// Distance between matrix baselines.
const MATRIX_ROW: f32 = 1.4;
/// Padding added to the widest cell of a matrix column.
const MATRIX_COL: f32 = 0.8;
/// Rule thickness in ems of the size in force.
const RULE_EM: f32 = 0.06;
/// Thinnest rule, points: below this a bar disappears at small sizes.
const RULE_MIN: f32 = 0.4;
/// Space (`MathNode::Space`) is clamped to this em range.
const SPACE_MIN: f32 = -0.5;
const SPACE_MAX: f32 = 4.0;
/// Smallest and largest size accepted from a caller.
const MIN_SIZE: f32 = 1.0;
const MAX_SIZE: f32 = 1638.0;
/// Size used when a caller passes a size that is not finite.
const DEFAULT_SIZE: f32 = 12.0;
/// Most glyphs one equation may be typeset into.
const MAX_GLYPHS: usize = 20_000;
/// Most rules one equation may be typeset into.
const MAX_RULES: usize = 4_000;
/// Deepest nesting typeset. The parsers stop at 32, so this only bounds hand-built trees.
const MAX_DEPTH: u8 = 64;

/// A glyph scaled vertically until its outline covers a required height.
struct Stretched {
    face: FaceRef,
    synth_bold: bool,
    synth_italic: bool,
    gid: u32,
    size: f32,
    ascent: f32,
    descent: f32,
    advance: f32,
}

/// The typesetter: the run's faces, its glyph/rule budget, and the glyph outlines it has measured.
struct Typer {
    /// Symbols, operators, delimiters and accents.
    sym: Resolved,
    /// Text in [`MathStyle::Roman`].
    roman: Resolved,
    /// Text in [`MathStyle::Italic`].
    italic: Resolved,
    /// Text in [`MathStyle::Bold`].
    bold: Resolved,
    /// Text in [`MathStyle::BoldItalic`].
    bolditalic: Resolved,
    /// Glyphs emitted so far, capped at [`MAX_GLYPHS`].
    glyphs: usize,
    /// Rules emitted so far, capped at [`MAX_RULES`].
    rules: usize,
    /// Outline box per (face, glyph), in font units: `(x0, y0, x1, y1)`, y down.
    ink: HashMap<(u32, u32), (f64, f64, f64, f64)>,
}

/// `v`, or 0 when it is not a positive finite number: metrics out of a font are hostile input too.
fn pos(v: f32) -> f32 {
    if v.is_finite() && v > 0.0 { v } else { 0.0 }
}

/// The thickness of every rule drawn at `size`.
fn rule_thickness(size: f32) -> f32 {
    (RULE_EM * size).max(RULE_MIN)
}

/// Script size for the script level a node sits at: script at the top level, script-script below.
fn script_size(size: f32, level: u8) -> f32 {
    size * if level == 0 { SCRIPT } else { SCRIPTSCRIPT }
}

/// The combining mark for a glyph accent and the spacing character to fall back on when no font
/// has the mark. `\bar`, `\overline` and `\underline` are rules, not glyphs.
fn accent_glyphs(kind: AccentKind) -> Option<(char, &'static str)> {
    Some(match kind {
        AccentKind::Hat => ('\u{302}', "^"),
        AccentKind::Tilde => ('\u{303}', "~"),
        AccentKind::Dot => ('\u{307}', "."),
        AccentKind::Ddot => ('\u{308}', ".."),
        AccentKind::Vec => ('\u{20D7}', "\u{2192}"),
        AccentKind::Bar | AccentKind::Overline | AccentKind::Underline => return None,
    })
}

/// The size to typeset with, from a caller: finite and clamped to what fonts can express.
fn sane_size(size: f32) -> f32 {
    if size.is_finite() { size.clamp(MIN_SIZE, MAX_SIZE) } else { DEFAULT_SIZE }
}

impl Typer {
    fn new(bold: bool, italic: bool) -> Typer {
        Typer {
            sym: word::resolve(FAMILY, bold, italic),
            roman: word::resolve(FAMILY, bold, false),
            italic: word::resolve(FAMILY, bold, italic),
            bold: word::resolve(FAMILY, true, false),
            bolditalic: word::resolve(FAMILY, true, true),
            glyphs: 0,
            rules: 0,
            ink: HashMap::new(),
        }
    }

    /// The face a run of text in `style` is set in.
    fn face_for(&self, style: MathStyle) -> Resolved {
        match style {
            MathStyle::Roman => self.roman,
            MathStyle::Italic => self.italic,
            MathStyle::Bold => self.bold,
            MathStyle::BoldItalic => self.bolditalic,
        }
    }

    /// The face, synthetic flags and glyph id for `c` in `r`, falling back to any font that covers
    /// `c`. `None` when nothing has it: a gap beats a tofu box.
    fn glyph_of(&self, r: &Resolved, c: char) -> Option<(FaceRef, bool, bool, u32)> {
        let gid = r.face.glyph_for(c);
        if gid != 0 {
            return Some((r.face, r.synth_bold, r.synth_italic, gid));
        }
        let fallback = FontDb::global().fallback_for(c, r.face.id())?;
        let gid = fallback.glyph_for(c);
        if gid == 0 {
            // The fallback claims the character but maps it to .notdef.
            return None;
        }
        Some((FaceRef::of(&fallback), false, false, gid))
    }

    /// Outline box of `gid` in font units, measured once per (face, glyph). Control points, never
    /// tight curve extrema: an over-estimate, so boxes built from it always cover what is drawn.
    fn ink_units(&mut self, face: &FontFace, gid: u32) -> (f64, f64, f64, f64) {
        *self.ink.entry((face.id(), gid)).or_insert_with(|| {
            let bb = FontDb::global().outline(face, gid).control_box();
            (bb.x0, bb.y0, bb.x1, bb.y1)
        })
    }

    /// The glyph's ink box at `size` points, y up from the baseline: `(x0, y0, x1, y1)`.
    fn ink_box(&mut self, face: &FontFace, gid: u32, size: f32) -> (f32, f32, f32, f32) {
        let (x0, y0, x1, y1) = self.ink_units(face, gid);
        let k = if face.upem > 0.0 { (size as f64) / face.upem } else { 0.0 };
        // The stored box is y down (an outline convention); flip it to y up.
        ((x0 * k) as f32, (-y1 * k) as f32, (x1 * k) as f32, (-y0 * k) as f32)
    }

    /// Ink extents of `gid` at `size`: (above the baseline, below it), both ≥ 0.
    fn ink(&mut self, face: &FontFace, gid: u32, size: f32) -> (f32, f32) {
        let (_, y0, _, y1) = self.ink_box(face, gid, size);
        (pos(y1), pos(-y0))
    }

    /// Advance width of `gid` at `size` points.
    fn advance(face: &FontFace, gid: u32, size: f32) -> f32 {
        if face.upem <= 0.0 {
            return 0.0;
        }
        pos((face.advance(gid) * (size as f64) / face.upem) as f32)
    }

    /// Claim one of the [`MAX_GLYPHS`] glyph slots.
    fn take_glyph(&mut self) -> bool {
        if self.glyphs >= MAX_GLYPHS {
            return false;
        }
        self.glyphs += 1;
        true
    }

    /// Claim one of the [`MAX_RULES`] rule slots.
    fn take_rule(&mut self) -> bool {
        if self.rules >= MAX_RULES {
            return false;
        }
        self.rules += 1;
        true
    }

    /// Add a glyph to `out`, with ink `ascent` above and `descent` below its own `y`. Over budget
    /// the glyph is dropped but the box still covers it, so later placement stays sane.
    fn put_glyph(&mut self, out: &mut MathBox, glyph: MathGlyph, ascent: f32, descent: f32) {
        let y = glyph.y;
        out.ascent = out.ascent.max(y + ascent);
        out.descent = out.descent.max(descent - y);
        if self.take_glyph() {
            out.glyphs.push(glyph);
        }
    }

    /// Add a rule to `out` (same budget rule as [`Typer::put_glyph`]).
    fn put_rule(&mut self, out: &mut MathBox, rule: MathRule) {
        let half = 0.5 * rule.thickness;
        out.ascent = out.ascent.max(rule.y + half);
        out.descent = out.descent.max(half - rule.y);
        if self.take_rule() {
            out.rules.push(rule);
        }
    }

    /// Grow `out` so it covers `b`'s ink when `b` is placed `dy` above the baseline.
    fn cover_box(out: &mut MathBox, b: &MathBox, dy: f32) {
        out.ascent = out.ascent.max(dy + b.ascent);
        out.descent = out.descent.max(b.descent - dy);
    }

    /// Move an already-typeset box into `out` at `(dx, dy)`. Its glyphs and rules were counted when
    /// they were made, so moving them spends no budget.
    fn move_box(out: &mut MathBox, b: MathBox, dx: f32, dy: f32) {
        Self::cover_box(out, &b, dy);
        for mut g in b.glyphs {
            g.x += dx;
            g.y += dy;
            out.glyphs.push(g);
        }
        for mut r in b.rules {
            r.x0 += dx;
            r.x1 += dx;
            r.y += dy;
            out.rules.push(r);
        }
        out.w = out.w.max(dx + b.w);
    }

    /// Raise a box's contents by `dy`, keeping its ink box and width consistent. Nothing sits
    /// below the baseline any more when everything moved up past it.
    fn raise(b: &mut MathBox, dy: f32) {
        for g in &mut b.glyphs {
            g.y += dy;
        }
        for r in &mut b.rules {
            r.y += dy;
        }
        b.ascent = pos(b.ascent + dy);
        b.descent = pos(b.descent - dy);
    }

    /// Draw `c` in `r` at `(x, y)` and return its advance (0 when nothing has the character).
    fn emit(&mut self, out: &mut MathBox, r: &Resolved, c: char, size: f32, x: f32, y: f32) -> f32 {
        let Some((face, synth_bold, synth_italic, gid)) = self.glyph_of(r, c) else { return 0.0 };
        let adv = Self::advance(&face, gid, size);
        let (a, d) = self.ink(&face, gid, size);
        self.put_glyph(out, MathGlyph { face, size, synth_bold, synth_italic, gid, x, y }, a, d);
        out.w = out.w.max(x + adv);
        adv
    }

    /// A run of text in `style`: glyphs on the baseline, the box as wide as their advances.
    fn text(&mut self, s: &str, style: MathStyle, size: f32) -> MathBox {
        let r = self.face_for(style);
        let mut out = MathBox::default();
        let mut x = 0.0;
        for c in s.chars() {
            x += self.emit(&mut out, &r, c, size, x, 0.0);
        }
        out.w = x;
        out
    }

    /// `c` in `r` scaled up (never down) until its outline is at least `need` points tall. The
    /// stand-in for a math font's optical size variants, used for radicals and tall delimiters.
    fn stretch(&mut self, r: &Resolved, c: char, need: f32, size: f32) -> Option<Stretched> {
        let (face, synth_bold, synth_italic, gid) = self.glyph_of(r, c)?;
        let (a, d) = self.ink(&face, gid, size);
        let height = a + d;
        let mut k = if height > 0.0 { (need / height).max(1.0) } else { 1.0 };
        if !k.is_finite() {
            k = 1.0;
        }
        Some(Stretched {
            face,
            synth_bold,
            synth_italic,
            gid,
            size: size * k,
            ascent: a * k,
            descent: d * k,
            advance: Self::advance(&face, gid, size) * k,
        })
    }

    /// Place a stretched glyph at `x`, with the centre of its ink at `centre` above the baseline.
    fn put_stretched(&mut self, out: &mut MathBox, g: &Stretched, x: f32, centre: f32) {
        let y = centre - 0.5 * (g.ascent - g.descent);
        self.put_glyph(
            out,
            MathGlyph { face: g.face, size: g.size, synth_bold: g.synth_bold, synth_italic: g.synth_italic, gid: g.gid, x, y },
            g.ascent,
            g.descent,
        );
    }

    /// The ink box of `b`'s glyphs, from `b`'s own origin: `(x0, y0, x1, y1)`.
    fn ink_extent(&mut self, b: &MathBox) -> (f32, f32, f32, f32) {
        let mut lo_x = f32::INFINITY;
        let mut lo_y = f32::INFINITY;
        let mut hi_x = f32::NEG_INFINITY;
        let mut hi_y = f32::NEG_INFINITY;
        for g in &b.glyphs {
            let (x0, y0, x1, y1) = self.ink_box(&g.face, g.gid, g.size);
            lo_x = lo_x.min(g.x + x0);
            lo_y = lo_y.min(g.y + y0);
            hi_x = hi_x.max(g.x + x1);
            hi_y = hi_y.max(g.y + y1);
        }
        if lo_x <= hi_x { (lo_x, lo_y, hi_x, hi_y) } else { (0.0, 0.0, 0.0, 0.0) }
    }

    /// Lay out one node at `size`, under a [`MathNode::Styled`] `style` when there is one, at
    /// script `level` and `depth` levels of nesting.
    fn layout(&mut self, n: &MathNode, size: f32, style: Option<MathStyle>, level: u8, depth: u8) -> MathBox {
        if depth > MAX_DEPTH {
            return MathBox::default();
        }
        match n {
            MathNode::Row(nodes) => self.row(nodes, size, style, level, depth),
            MathNode::Text(s, st) => self.text(s, style.unwrap_or(*st), size),
            MathNode::Space(em) => {
                let em = if em.is_finite() { em.clamp(SPACE_MIN, SPACE_MAX) } else { 0.0 };
                MathBox { w: em * size, ..MathBox::default() }
            }
            MathNode::Frac { num, den } => self.frac(num, den, size, style, level, depth),
            MathNode::Sqrt { index, body } => self.sqrt(body, index.as_deref(), size, style, level, depth),
            MathNode::Script { base, sub, sup } => {
                // A big operator takes its scripts itself: stacked when it wants limits.
                if let MathNode::Big { op, sub: bsub, sup: bsup, limits } = &**base {
                    let sub = sub.as_deref().or(bsub.as_deref());
                    let sup = sup.as_deref().or(bsup.as_deref());
                    return self.big(op, sub, sup, *limits, size, style, level, depth);
                }
                let base = self.layout(base, size, style, level, depth + 1);
                self.attach(base, sub.as_deref(), sup.as_deref(), size, style, level, depth)
            }
            MathNode::Big { op, sub, sup, limits } => self.big(op, sub.as_deref(), sup.as_deref(), *limits, size, style, level, depth),
            MathNode::Delim { left, right, body } => {
                let inner = self.layout(body, size, style, level, depth + 1);
                self.delimit(inner, *left, *right, size)
            }
            MathNode::Accent { kind, body } => {
                let base = self.layout(body, size, style, level, depth + 1);
                self.accent(*kind, base, size)
            }
            MathNode::Matrix { rows, left, right } => self.matrix(rows, *left, *right, size, style, level, depth),
            MathNode::Styled { style: st, body } => self.layout(body, size, Some(*st), level, depth + 1),
        }
    }

    /// A sequence, left to right on one baseline: each child's descent hangs below it by as much as
    /// the child has, and the row is as tall and as deep as its tallest and deepest child.
    fn row(&mut self, nodes: &[MathNode], size: f32, style: Option<MathStyle>, level: u8, depth: u8) -> MathBox {
        let mut out = MathBox::default();
        let mut x = 0.0;
        for n in nodes {
            let mut b = self.layout(n, size, style, level, depth + 1);
            for g in &mut b.glyphs {
                g.x += x;
            }
            for r in &mut b.rules {
                r.x0 += x;
                r.x1 += x;
            }
            Self::cover_box(&mut out, &b, 0.0);
            out.glyphs.append(&mut b.glyphs);
            out.rules.append(&mut b.rules);
            x += b.w;
        }
        out.w = x;
        out
    }

    /// A fraction: the numerator above the bar, the denominator below, both centred on it. The bar
    /// is at the axis height and each is set `0.1*S` clear of it — counting its own ink, so the
    /// gap is between the bar and the ink, not between the bar and the baseline.
    fn frac(&mut self, num: &MathNode, den: &MathNode, size: f32, style: Option<MathStyle>, level: u8, depth: u8) -> MathBox {
        let mut n = self.layout(num, size, style, level, depth + 1);
        let mut d = self.layout(den, size, style, level, depth + 1);
        let axis = AXIS * size;
        let pad = FRAC_GAP * size;
        let t = rule_thickness(size);
        let w = (n.w.max(d.w) + 2.0 * pad).max(t);
        let ny = axis + pad + n.descent;
        let dy = -(axis + pad + d.ascent);
        let nx = 0.5 * (w - n.w);
        let dx = 0.5 * (w - d.w);
        for g in &mut n.glyphs {
            g.x += nx;
            g.y += ny;
        }
        for r in &mut n.rules {
            r.x0 += nx;
            r.x1 += nx;
            r.y += ny;
        }
        for g in &mut d.glyphs {
            g.x += dx;
            g.y += dy;
        }
        for r in &mut d.rules {
            r.x0 += dx;
            r.x1 += dx;
            r.y += dy;
        }
        let mut out = MathBox { w, ..MathBox::default() };
        Self::cover_box(&mut out, &n, ny);
        Self::cover_box(&mut out, &d, dy);
        out.glyphs.append(&mut n.glyphs);
        out.glyphs.append(&mut d.glyphs);
        out.rules.append(&mut n.rules);
        out.rules.append(&mut d.rules);
        self.put_rule(&mut out, MathRule { x0: 0.0, x1: w, y: axis, thickness: t });
        out
    }

    /// A radical: the surd scaled to cover the radicand, a bar over it at `0.1*S` above the
    /// radicand's ink, and the index (if any) at script-script size above the surd's top-left.
    fn sqrt(&mut self, body: &MathNode, index: Option<&MathNode>, size: f32, style: Option<MathStyle>, level: u8, depth: u8) -> MathBox {
        let body = self.layout(body, size, style, level, depth + 1);
        let t = rule_thickness(size);
        let gap = RADICAL_GAP * size;
        let sym = self.sym;
        let need = body.ascent + body.descent + 2.0 * gap;
        let surd = self.stretch(&sym, '\u{221A}', need, size);
        let index = index.map(|n| self.layout(n, SCRIPTSCRIPT * size, style, level + 1, depth + 1));
        let surd_w = surd.as_ref().map_or(0.0, |s| s.advance);
        let idx_w = index.as_ref().map_or(0.0, |b| b.w);
        let body_x = idx_w + surd_w;
        let mut out = MathBox { w: body_x + body.w, ..MathBox::default() };
        let mut surd_top = 0.0;
        if let Some(s) = surd {
            // Sitting on the radicand's ink bottom, so the surd's overshoot reaches over the bar.
            let y = s.descent - body.descent;
            surd_top = y + s.ascent;
            self.put_glyph(
                &mut out,
                MathGlyph { face: s.face, size: s.size, synth_bold: s.synth_bold, synth_italic: s.synth_italic, gid: s.gid, x: idx_w, y },
                s.ascent,
                s.descent,
            );
        }
        let bar_y = body.ascent + gap;
        let x0 = idx_w + 0.7 * surd_w;
        let x1 = body_x + body.w;
        Self::move_box(&mut out, body, body_x, 0.0);
        self.put_rule(&mut out, MathRule { x0, x1: x1.max(x0), y: bar_y, thickness: t });
        if let Some(idx) = index {
            // The index hangs above the surd's top-left; its ink can overhang its advance.
            let (_, _, right, _) = self.ink_extent(&idx);
            Self::move_box(&mut out, idx, 0.0, surd_top);
            out.w = out.w.max(right);
        }
        out
    }

    /// A base with its scripts to the right, each raised or dropped far enough to clear the base's
    /// own ink: `0.45*S` up and `0.15*S` down is the usual place, and only a deep or tall base
    /// pushes its scripts further.
    fn attach(
        &mut self,
        base: MathBox,
        sub: Option<&MathNode>,
        sup: Option<&MathNode>,
        size: f32,
        style: Option<MathStyle>,
        level: u8,
        depth: u8,
    ) -> MathBox {
        if sub.is_none() && sup.is_none() {
            return base;
        }
        let child = script_size(size, level);
        let sub = sub.map(|n| self.layout(n, child, style, level + 1, depth + 1));
        let sup = sup.map(|n| self.layout(n, child, style, level + 1, depth + 1));
        let mut out = base;
        let x = out.w;
        let mut w = out.w;
        if let Some(mut s) = sub {
            let mut y = -SUB_LOWER * size;
            if out.descent > -y {
                y = -(out.descent + CLEAR * size + s.ascent);
            }
            for g in &mut s.glyphs {
                g.x += x;
                g.y += y;
            }
            for r in &mut s.rules {
                r.x0 += x;
                r.x1 += x;
                r.y += y;
            }
            Self::cover_box(&mut out, &s, y);
            w = w.max(x + s.w);
            out.glyphs.append(&mut s.glyphs);
            out.rules.append(&mut s.rules);
        }
        if let Some(mut u) = sup {
            let mut y = SUP_RAISE * size;
            if out.ascent > y - u.descent {
                y = out.ascent + CLEAR * size + u.descent;
            }
            for g in &mut u.glyphs {
                g.x += x;
                g.y += y;
            }
            for r in &mut u.rules {
                r.x0 += x;
                r.x1 += x;
                r.y += y;
            }
            Self::cover_box(&mut out, &u, y);
            w = w.max(x + u.w);
            out.glyphs.append(&mut u.glyphs);
            out.rules.append(&mut u.rules);
        }
        out.w = w;
        out
    }

    /// A big operator: its glyph at `1.4*S` (the letters of the command when the font has no
    /// symbol for it), with limits stacked above and below when it asks for them, and its scripts
    /// to the right when it does not.
    fn big(
        &mut self,
        op: &str,
        sub: Option<&MathNode>,
        sup: Option<&MathNode>,
        limits: bool,
        size: f32,
        style: Option<MathStyle>,
        level: u8,
        depth: u8,
    ) -> MathBox {
        let body = BIG_SCALE * size;
        let text = math::cmd_symbol(op).map(str::to_string).unwrap_or_else(|| op.to_string());
        let sym = self.sym;
        let mut oper = MathBox::default();
        let mut chars = text.chars();
        match (chars.next(), chars.next()) {
            (Some(c), None) => {
                let adv = self.emit(&mut oper, &sym, c, body, 0.0, 0.0);
                oper.w = adv;
            }
            _ => oper = self.text(&text, style.unwrap_or(MathStyle::Roman), body),
        }
        if !limits || (sub.is_none() && sup.is_none()) {
            return self.attach(oper, sub, sup, size, style, level, depth);
        }
        let child = script_size(size, level);
        let sub = sub.map(|n| self.layout(n, child, style, level + 1, depth + 1));
        let sup = sup.map(|n| self.layout(n, child, style, level + 1, depth + 1));
        let gap = LIMIT_GAP * size;
        let w = oper.w.max(sub.as_ref().map_or(0.0, |b| b.w)).max(sup.as_ref().map_or(0.0, |b| b.w));
        let (op_a, op_d, op_w) = (oper.ascent, oper.descent, oper.w);
        let mut out = MathBox { w, ..MathBox::default() };
        Self::move_box(&mut out, oper, 0.5 * (w - op_w), 0.0);
        if let Some(s) = sub {
            let dy = -(op_d + gap + s.ascent);
            let dx = 0.5 * (w - s.w);
            Self::move_box(&mut out, s, dx, dy);
        }
        if let Some(u) = sup {
            let dy = op_a + gap + u.descent;
            let dx = 0.5 * (w - u.w);
            Self::move_box(&mut out, u, dx, dy);
        }
        out
    }

    /// Delimiters around a box: each glyph scaled to cover the box's height and centred on the
    /// axis, the box itself centred on the axis too. `.` is an invisible delimiter.
    fn delimit(&mut self, inner: MathBox, left: char, right: char, size: f32) -> MathBox {
        let sym = self.sym;
        let need = inner.ascent + inner.descent;
        let lg = if left == '.' { None } else { self.stretch(&sym, left, need, size) };
        let rg = if right == '.' { None } else { self.stretch(&sym, right, need, size) };
        let lw = lg.as_ref().map_or(0.0, |g| g.advance);
        let rw = rg.as_ref().map_or(0.0, |g| g.advance);
        let axis = AXIS * size;
        let inner_w = inner.w;
        let mut out = MathBox { w: lw + inner_w + rw, ..MathBox::default() };
        if let Some(g) = lg {
            self.put_stretched(&mut out, &g, 0.0, axis);
        }
        if let Some(g) = rg {
            self.put_stretched(&mut out, &g, lw + inner_w, axis);
        }
        // Centre the contents on the axis.
        let dy = axis - 0.5 * (inner.ascent - inner.descent);
        Self::move_box(&mut out, inner, lw, dy);
        out
    }

    /// An accent over (or under) its base. `\bar`, `\overline` and `\underline` are rules; the
    /// rest are glyphs centred on the base, on its top.
    fn accent(&mut self, kind: AccentKind, base: MathBox, size: f32) -> MathBox {
        let t = rule_thickness(size);
        match kind {
            AccentKind::Bar | AccentKind::Overline => {
                let mut out = base;
                let x1 = out.w;
                let y = out.ascent + 0.5 * t;
                self.put_rule(&mut out, MathRule { x0: 0.0, x1, y, thickness: t });
                out
            }
            AccentKind::Underline => {
                let mut out = base;
                let x1 = out.w;
                let y = -(out.descent + 0.5 * t);
                self.put_rule(&mut out, MathRule { x0: 0.0, x1, y, thickness: t });
                out
            }
            kind => {
                let Some((combining, spacing)) = accent_glyphs(kind) else { return base };
                let sym = self.sym;
                let mut mark = MathBox::default();
                self.emit(&mut mark, &sym, combining, size, 0.0, 0.0);
                if mark.glyphs.is_empty() {
                    // No font here has the combining mark: the spacing character stands in.
                    let mut pen = 0.0;
                    for c in spacing.chars() {
                        pen += self.emit(&mut mark, &sym, c, size, pen, 0.0);
                    }
                }
                // Centre the mark's ink on the base and sit it on the base's ink top: a combining
                // mark's own ink is already drawn above the baseline it attaches to, a spacing
                // one's near it, so aligning ink bottoms is what works for both.
                let (lo, low, hi, _) = self.ink_extent(&mark);
                let mut out = base;
                let dx = 0.5 * out.w - 0.5 * (lo + hi);
                let dy = out.ascent + ACCENT_GAP * size - low;
                Self::cover_box(&mut out, &mark, dy);
                for mut g in mark.glyphs {
                    g.x += dx;
                    g.y += dy;
                    out.glyphs.push(g);
                }
                for mut r in mark.rules {
                    r.x0 += dx;
                    r.x1 += dx;
                    r.y += dy;
                    out.rules.push(r);
                }
                out.w = out.w.max(dx + hi);
                out
            }
        }
    }

    /// A matrix: cells centred in columns as wide as their widest cell plus `0.8*S`, rows `1.4*S`
    /// apart, the whole grid centred on the axis and delimited.
    fn matrix(&mut self, rows: &[Vec<MathNode>], left: char, right: char, size: f32, style: Option<MathStyle>, level: u8, depth: u8) -> MathBox {
        if rows.is_empty() {
            return MathBox::default();
        }
        let mut cells: Vec<Vec<MathBox>> = Vec::with_capacity(rows.len());
        for row in rows {
            let mut line = Vec::with_capacity(row.len());
            for cell in row {
                line.push(self.layout(cell, size, style, level, depth + 1));
            }
            cells.push(line);
        }
        let ncols = cells.iter().map(Vec::len).max().unwrap_or(0);
        if ncols == 0 {
            return MathBox::default();
        }
        let mut cols = vec![0.0f32; ncols];
        for line in &cells {
            for (j, cell) in line.iter().enumerate() {
                if let Some(w) = cols.get_mut(j) {
                    *w = w.max(cell.w);
                }
            }
        }
        for w in &mut cols {
            *w = pos(*w) + MATRIX_COL * size;
        }
        let step = MATRIX_ROW * size;
        let total: f32 = cols.iter().sum();
        let mut inner = MathBox { w: total, ..MathBox::default() };
        let mut baseline = 0.0;
        for line in cells {
            let mut x = 0.0;
            for (j, cell) in line.into_iter().enumerate() {
                let w = cols.get(j).copied().unwrap_or(0.0);
                let dx = x + 0.5 * (w - pos(cell.w));
                x += w;
                Self::move_box(&mut inner, cell, dx, baseline);
            }
            baseline -= step;
        }
        // Centre the grid on the axis before the delimiters measure it.
        let shift = AXIS * size - 0.5 * (inner.ascent - inner.descent);
        Self::raise(&mut inner, shift);
        self.delimit(inner, left, right, size)
    }
}

/// Typeset `node` at `size` points, as a run of `bold`/`italic` text. The size is clamped to
/// `[1, 1638]` points, and a size that is not finite is treated as 12.
pub fn typeset(node: &MathNode, size: f32, bold: bool, italic: bool) -> MathBox {
    let size = sane_size(size);
    let mut typer = Typer::new(bold, italic);
    typer.layout(node, size, None, 0, 0)
}

/// Parse `src` (`wordcraft_doc::math::parse`) and typeset it; `Err` when the source is not valid
/// math.
pub fn typeset_source(src: &str, size: f32, bold: bool, italic: bool) -> Result<MathBox, MathError> {
    Ok(typeset(&math::parse(src)?, size, bold, italic))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn typeset_src(src: &str) -> MathBox {
        match typeset_source(src, 12.0, false, false) {
            Ok(b) => b,
            Err(e) => panic!("{src}: {e}"),
        }
    }

    /// How many of `b`'s glyphs draw `c` (any face: the glyph's own face is asked).
    fn count(b: &MathBox, c: char) -> usize {
        b.glyphs.iter().filter(|g| g.gid == g.face.glyph_for(c)).count()
    }

    #[test]
    fn fraction_has_a_bar_and_two_halves() {
        let b = typeset_src("\\frac{a}{b}");
        assert!(b.w > 0.0, "{b:?}");
        assert!(b.ascent > 0.0, "{b:?}");
        assert!(b.descent > 0.0, "{b:?}");
        assert!(b.glyphs.len() >= 2, "{b:?}");
        assert_eq!(b.rules.len(), 1, "{b:?}");
        let bar = b.rules[0];
        assert!(bar.x1 > bar.x0 && bar.thickness > 0.0);
        assert!(bar.y > 0.0 && bar.y < b.ascent);
    }

    /// Every glyph's ink and every rule sits inside the box that was typeset around it.
    fn covered(b: &MathBox) {
        let mut t = Typer::new(false, false);
        for g in &b.glyphs {
            let (a, d) = t.ink(&g.face, g.gid, g.size);
            assert!(g.y + a <= b.ascent + 0.02, "{g:?} sticks out of {b:?}");
            assert!(d - g.y <= b.descent + 0.02, "{g:?} sticks out of {b:?}");
        }
        for r in &b.rules {
            let half = 0.5 * r.thickness;
            assert!(r.y + half <= b.ascent + 0.02, "{r:?} sticks out of {b:?}");
            assert!(half - r.y <= b.descent + 0.02, "{r:?} sticks out of {b:?}");
        }
    }

    #[test]
    fn every_glyph_and_rule_is_inside_the_box() {
        // Every shape: a fraction, a radical, scripts, a big operator, delimiters, a matrix.
        let sources = ["\\frac{a}{b}", "\\sqrt[3]{x+y}", "x^2", "x_i^2", "\\sum_{i=1}^{n} i"];
        for src in sources {
            let b = typeset_src(src);
            assert!(b.ascent >= 0.0 && b.descent >= 0.0, "{src}: {b:?}");
            covered(&b);
        }
        let more = ["\\int_0^1 f+\\frac{1}{2}", "\\left(\\sqrt{\\frac{a}{b}}\\right)", "\\hat{x}+\\vec{y}z_1", "\\bar{x}+\\underline{x}"];
        for src in more {
            let b = typeset_src(src);
            assert!(b.ascent >= 0.0 && b.descent >= 0.0, "{src}: {b:?}");
            covered(&b);
        }
    }

    #[test]
    fn radical_has_a_bar_and_a_surd() {
        let b = typeset_src("\\sqrt{x}");
        assert!(!b.rules.is_empty(), "{b:?}");
        assert!(!b.glyphs.is_empty(), "{b:?}");
        assert_eq!(count(&b, '\u{221A}'), 1, "{b:?}");
        assert_eq!(count(&b, 'x'), 1, "{b:?}");
        assert!(b.rules[0].y > 0.0, "{b:?}");
        let surd = b.glyphs.iter().find(|g| g.gid == g.face.glyph_for('\u{221A}')).unwrap();
        assert!(surd.size >= 12.0, "{surd:?}");
        // A radicand taller than the surd stretches it.
        let tall = typeset_src("\\sqrt{\\frac{a}{b}}");
        let surd = tall.glyphs.iter().find(|g| g.gid == g.face.glyph_for('\u{221A}')).unwrap();
        assert!(surd.size > 12.0, "{surd:?}");
        let bare = typeset_src("\\sqrt[3]{x}");
        assert_eq!(count(&bare, '\u{221A}'), 1, "{bare:?}");
        assert_eq!(count(&bare, '3'), 1, "{bare:?}");
    }

    #[test]
    fn script_is_set_smaller_and_raised() {
        let b = typeset_src("x^2");
        let small: Vec<&MathGlyph> = b.glyphs.iter().filter(|g| g.size < 12.0).collect();
        assert!(!small.is_empty(), "{b:?}");
        for g in &small {
            assert!(g.y > 0.0, "a superscript sits above the baseline: {g:?}");
        }
        assert!(b.glyphs.iter().any(|g| g.size == 12.0), "{b:?}");
    }

    #[test]
    fn limits_stack_above_and_below() {
        let b = typeset_src("\\sum_{i=1}^{n} i");
        let above = b.glyphs.iter().filter(|g| g.size < 12.0 && g.y > 1.0).count();
        let below = b.glyphs.iter().filter(|g| g.size < 12.0 && g.y < -1.0).count();
        assert!(above > 0, "limits above: {b:?}");
        assert!(below > 0, "limits below: {b:?}");
        assert!(b.ascent > 0.0 && b.descent > 0.0, "{b:?}");
    }

    #[test]
    fn matrix_has_cells_and_delimiters() {
        let b = typeset_src("\\begin{pmatrix}a&b\\\\c&d\\end{pmatrix}");
        for c in ['a', 'b', 'c', 'd'] {
            assert_eq!(count(&b, c), 1, "{c}: {b:?}");
        }
        assert_eq!(count(&b, '('), 1, "{b:?}");
        assert_eq!(count(&b, ')'), 1, "{b:?}");
        // The delimiters cover the two rows.
        let open = b.glyphs.iter().find(|g| g.gid == g.face.glyph_for('(')).unwrap();
        assert!(open.size > 12.0, "{open:?}");
    }

    #[test]
    fn accents_sit_above_the_base() {
        let b = typeset_src("\\hat{x}");
        assert_eq!(count(&b, 'x'), 1, "{b:?}");
        assert_eq!(b.rules.len(), 0, "{b:?}");
        assert!(b.glyphs.len() >= 2, "{b:?}");
        let base = b.glyphs.iter().find(|g| g.gid == g.face.glyph_for('x')).unwrap();
        let mark = b.glyphs.iter().find(|g| g.gid != base.gid).unwrap();
        // The accent's ink clears the base's ink (its baseline may sit below the base's).
        let mut t = Typer::new(false, false);
        let (_, low, _, _) = t.ink_box(&mark.face, mark.gid, mark.size);
        let (_, _, _, top) = t.ink_box(&base.face, base.gid, base.size);
        assert!(mark.y + low >= base.y + top, "{mark:?} is not above {base:?}");

        let bar = typeset_src("\\bar{y}");
        assert_eq!(bar.rules.len(), 1, "{bar:?}");
        let under = typeset_src("\\underline{y}");
        assert!(under.rules.len() == 1 && under.rules[0].y < 0.0, "{under:?}");
    }

    #[test]
    fn a_character_no_font_has_is_skipped() {
        for c in ['\u{10FFFE}', '\u{FFFF}', '\u{E01F0}'] {
            let b = typeset(&MathNode::Text(c.to_string(), MathStyle::Roman), 12.0, false, false);
            // Drawn or skipped, never a panic and never outside the box.
            for g in &b.glyphs {
                assert!(g.gid != 0, "{c:?} was drawn as .notdef");
            }
        }
        let b = typeset(&MathNode::Text("a\u{10FFFE}b".to_string(), MathStyle::Italic), 12.0, false, true);
        assert!(b.w > 0.0, "{b:?}");
    }

    #[test]
    fn invalid_source_is_an_error() {
        assert!(typeset_source("\\nope", 12.0, false, false).is_err());
        assert!(typeset_source("\\frac{a}", 12.0, false, false).is_err());
        assert!(typeset_source("{a", 12.0, false, false).is_err());
        assert!(typeset_source("a", 12.0, false, false).is_ok());
    }

    #[test]
    fn sizes_are_sane() {
        let tiny = typeset(&MathNode::Space(1.0), 0.0, false, false);
        assert!((tiny.w - 1.0).abs() < 1e-4, "{tiny:?}");
        let nan = typeset(&MathNode::Space(1.0), f32::NAN, false, false);
        assert!((nan.w - 12.0).abs() < 1e-4, "{nan:?}");
        let huge = typeset(&MathNode::Space(1.0), f32::INFINITY, false, false);
        assert!((huge.w - 12.0).abs() < 1e-4, "{huge:?}");
    }

    #[test]
    fn long_and_deep_sources_do_not_panic() {
        let long = "a".repeat(8192);
        let _ = typeset_source(&long, 12.0, false, false);
        let nested = "\\frac{".repeat(400) + "a" + &"}".repeat(400);
        let _ = typeset_source(&nested, 12.0, false, false);
        let delims = "(".repeat(2000) + "a" + &")".repeat(2000);
        let _ = typeset_source(&delims, 12.0, false, false);
    }

    #[test]
    fn a_hand_built_tree_this_deep_does_not_panic() {
        let mut n = MathNode::Text("x".to_string(), MathStyle::Italic);
        for _ in 0..500 {
            n = MathNode::Row(vec![n]);
        }
        let b = typeset(&n, 12.0, false, false);
        assert!(b.w >= 0.0);
        let mut n = MathNode::Text("x".to_string(), MathStyle::Italic);
        for _ in 0..500 {
            n = MathNode::Script { base: Box::new(n), sub: Some(Box::new(MathNode::Text("1".to_string(), MathStyle::Roman))), sup: None };
        }
        let _ = typeset(&n, 12.0, false, false);
    }

    #[test]
    fn glyph_and_rule_budgets_hold() {
        let many: Vec<MathNode> = (0..400).map(|_| MathNode::Text("a".repeat(60), MathStyle::Italic)).collect();
        let b = typeset(&MathNode::Row(many), 12.0, false, false);
        assert!(b.glyphs.len() <= MAX_GLYPHS, "{}", b.glyphs.len());
        assert_eq!(b.glyphs.len(), MAX_GLYPHS);

        let fracs: Vec<MathNode> = (0..4_500)
            .map(|_| {
                let num = Box::new(MathNode::Text("a".to_string(), MathStyle::Italic));
                let den = Box::new(MathNode::Text("b".to_string(), MathStyle::Italic));
                MathNode::Frac { num, den }
            })
            .collect();
        let b = typeset(&MathNode::Row(fracs), 12.0, false, false);
        assert!(b.rules.len() <= MAX_RULES, "{}", b.rules.len());
        assert_eq!(b.rules.len(), MAX_RULES);
        assert!(b.glyphs.len() <= MAX_GLYPHS);
    }

    #[test]
    fn styles_pick_faces_and_spaces_advance() {
        let roman = typeset(&MathNode::Text("A".to_string(), MathStyle::Roman), 12.0, false, false);
        let italic = typeset(&MathNode::Text("A".to_string(), MathStyle::Italic), 12.0, false, true);
        let (r, i) = (roman.glyphs[0].clone(), italic.glyphs[0].clone());
        // A font without a real italic is slanted when drawn instead.
        assert!(r.face.id() != i.face.id() || i.synth_italic, "Roman {r:?} vs italic {i:?}");
        let wide = typeset(&MathNode::Row(vec![MathNode::Text("a".to_string(), MathStyle::Italic), MathNode::Space(2.0)]), 12.0, false, false);
        assert!(wide.w > 24.0, "{wide:?}");
        let clamped = typeset(&MathNode::Space(100.0), 12.0, false, false);
        assert!((clamped.w - 48.0).abs() < 1e-3, "{clamped:?}");
    }
}
