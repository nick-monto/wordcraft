use super::*;

#[test]
fn new_document_is_valid() {
    let d = Document::new();
    assert_eq!(d.body.len(), 1);
    assert_eq!(d.para_paths(StoryRef::Body).len(), 1);
    assert_eq!(d.plain_text(StoryRef::Body), "");
    assert_eq!(d.sections().len(), 1);
}

#[test]
fn word_count() {
    let d = Document::from_text("Hello, world!\nThis is — a test 42.\n\n");
    assert_eq!(d.word_count(), 7);
    assert_eq!(count_words(" -- "), 0);
}

#[test]
fn sections_and_section_mut() {
    let mut d = Document::from_text("a\nb\nc");
    if let Some(Block::Para(p)) = d.body.get_mut(0).map(Arc::make_mut) {
        p.section = Some(Box::new(SectionProps { landscape: true, ..Default::default() }));
    }
    let s = d.sections();
    assert_eq!(s.len(), 2);
    assert_eq!(s[0].0, 0);
    assert_eq!(d.section_index_of(0), 0);
    assert_eq!(d.section_index_of(2), 1);
    d.section_mut(0).margin_left = 10.0;
    d.section_mut(2).margin_left = 20.0;
    assert_eq!(d.sections()[0].1.margin_left, 10.0);
    assert_eq!(d.last_section.margin_left, 20.0);
}

#[test]
fn parts_and_media() {
    let mut d = Document::new();
    let id = d.add_part(PartKind::Header, Vec::new());
    assert_eq!(d.story(StoryRef::Part(id)).map(|b| b.len()), Some(1));
    let k1 = d.add_media(vec![1, 2, 3], "png");
    let k2 = d.add_media(vec![1, 2, 3], "png");
    let k3 = d.add_media(vec![4], "png");
    assert_eq!(k1, k2);
    assert_ne!(k1, k3);
}

#[test]
fn clone_is_shallow() {
    let d = Document::from_text("a\nb");
    let mut e = d.clone();
    assert!(Arc::ptr_eq(&d.body[0], &e.body[0]));
    e.insert_text(&Pos::body(1, 0), "x", &CharProps::default()).unwrap();
    assert!(Arc::ptr_eq(&d.body[0], &e.body[0]));
    assert!(!Arc::ptr_eq(&d.body[1], &e.body[1]));
    assert_eq!(d.plain_text(StoryRef::Body), "a\nb");
}

#[test]
fn json_round_trip() {
    let mut d = Document::from_text("Hello\nWorld");
    d.format_range(&Pos::body(0, 0), &Pos::body(0, 5), &|c| c.bold = Some(true)).unwrap();
    let j = serde_json::to_string(&d).unwrap();
    let back: Document = serde_json::from_str(&j).unwrap();
    assert_eq!(back.plain_text(StoryRef::Body), "Hello\nWorld");
    assert_eq!(back.para(StoryRef::Body, &Path::top(0)).unwrap().runs[0].props.bold, Some(true));
}

#[test]
fn ensure_nonempty_repairs() {
    let mut d = Document::new();
    d.body.clear();
    d.body.push(Arc::new(Block::Table(Table::new(1, 1, 100.0))));
    d.ensure_nonempty();
    assert!(matches!(d.body.last().map(|b| &**b), Some(Block::Para(_))));
}

// ---------------------------------------------------------------------------
// math parsing
// ---------------------------------------------------------------------------

use super::math::{AccentKind, MathError, MathNode, MathStyle, cmd_symbol, parse, parse_latex, parse_linear, symbol_cmd, to_latex, to_plain};

/// True when `n` (recursively) contains a `Space` node.
fn has_space(n: &MathNode) -> bool {
    match n {
        MathNode::Space(_) => true,
        MathNode::Text(..) => false,
        MathNode::Row(v) => v.iter().any(has_space),
        MathNode::Frac { num, den } => has_space(num) || has_space(den),
        MathNode::Sqrt { index, body } => index.as_deref().is_some_and(has_space) || has_space(body),
        MathNode::Script { base, sub, sup } => has_space(base) || sub.as_deref().is_some_and(has_space) || sup.as_deref().is_some_and(has_space),
        MathNode::Big { sub, sup, .. } => sub.as_deref().is_some_and(has_space) || sup.as_deref().is_some_and(has_space),
        MathNode::Delim { body, .. } => has_space(body),
        MathNode::Accent { body, .. } => has_space(body),
        MathNode::Styled { body, .. } => has_space(body),
        MathNode::Matrix { rows, .. } => rows.iter().flatten().any(has_space),
    }
}

#[test]
fn math_parse_constructs() {
    // \frac -> Frac with Text numerator/denominator.
    let f = parse_latex("\\frac{a}{b}").unwrap();
    assert!(matches!(&f, MathNode::Frac { .. }));
    if let MathNode::Frac { num, den } = &f {
        assert!(matches!(**num, MathNode::Text(..)));
        assert!(matches!(**den, MathNode::Text(..)));
    } else {
        panic!("expected Frac");
    }

    // \sqrt without and with an index.
    assert!(matches!(&parse_latex("\\sqrt{x}").unwrap(), MathNode::Sqrt { index: None, .. }));
    assert!(matches!(&parse_latex("\\sqrt[3]{x}").unwrap(), MathNode::Sqrt { index: Some(_), .. }));

    // Scripts: sup only, sub only, both; order independent.
    assert!(matches!(&parse_latex("x^2").unwrap(), MathNode::Script { sup: Some(_), sub: None, .. }));
    assert!(matches!(&parse_latex("x_1").unwrap(), MathNode::Script { sub: Some(_), sup: None, .. }));
    let a = parse_latex("x_1^2").unwrap();
    let b = parse_latex("x^2_1").unwrap();
    assert!(matches!(&a, MathNode::Script { sub: Some(_), sup: Some(_), .. }));
    assert!(matches!(&b, MathNode::Script { sub: Some(_), sup: Some(_), .. }));
    assert_eq!(to_latex(&a), to_latex(&b));

    // Repeated sub/superscripts are errors.
    assert!(parse_latex("x^1^2").is_err());
    assert!(parse_latex("x_1_2").is_err());

    // Big operators: sum-style takes limits, integral-style does not.
    assert!(matches!(&parse_latex("\\sum_{i=1}^{n}").unwrap(), MathNode::Big { limits: true, sub: Some(_), sup: Some(_), .. }));
    assert!(matches!(&parse_latex("\\int_0^1").unwrap(), MathNode::Big { limits: false, .. }));

    // \left( ... \right) -> Delim.
    let d = parse_latex("\\left(x\\right)").unwrap();
    if let MathNode::Delim { left, right, body } = &d {
        assert_eq!(*left, '(');
        assert_eq!(*right, ')');
        assert!(matches!(**body, MathNode::Text(..)));
    } else {
        panic!("expected Delim");
    }

    // Accent.
    assert!(matches!(&parse_latex("\\hat{x}").unwrap(), MathNode::Accent { kind: AccentKind::Hat, .. }));

    // pmatrix / cases -> Matrix.
    let m = parse_latex("\\begin{pmatrix}a&b\\\\c&d\\end{pmatrix}").unwrap();
    if let MathNode::Matrix { rows, left, right } = &m {
        assert_eq!(*left, '(');
        assert_eq!(*right, ')');
        assert_eq!(rows.len(), 2);
        assert_eq!(rows[0].len(), 2);
        assert_eq!(rows[1].len(), 2);
    } else {
        panic!("expected Matrix");
    }
    assert!(matches!(&parse_latex("\\begin{cases}a&b\\\\c&d\\end{cases}").unwrap(), MathNode::Matrix { .. }));

    // \text is roman.
    assert!(matches!(
        &parse_latex("\\text{hi}").unwrap(),
        MathNode::Text(s, MathStyle::Roman) if s == "hi"
    ));

    // Spacing produces a Space node somewhere in the tree.
    assert!(has_space(&parse_latex("a\\,b").unwrap()));

    // Uppercase Greek.
    assert_eq!(cmd_symbol("Gamma"), Some("Γ"));
    assert_eq!(cmd_symbol("Delta"), Some("Δ"));
    assert!(parse_latex("\\Gamma").is_ok());
    assert!(symbol_cmd('Γ').is_some());

    // A handful of operator commands.
    assert!(parse_latex("\\alpha").is_ok());
    assert!(parse_latex("\\times").is_ok());
    assert!(parse_latex("\\le").is_ok());
    assert!(parse_latex("\\infty").is_ok());

    // `parse` dispatches to linear when there is no backslash command.
    assert_eq!(parse("x^2").unwrap(), parse_linear("x^2").unwrap());
}

#[test]
fn math_to_latex_idempotent() {
    let eqs = [
        "\\frac{a}{b}",
        "\\sqrt{x}",
        "\\sqrt[3]{y}",
        "x^2",
        "x_i",
        "x_i^2",
        "\\sum_{i=1}^{n} i",
        "\\int_0^1 f",
        "\\left(a+b\\right)",
        "\\hat{x}",
        "\\vec{v}",
        "\\bar{y}",
        "\\begin{pmatrix}a&b\\\\c&d\\end{pmatrix}",
        "\\text{hi}",
        "\\alpha+\\beta",
        "a\\,b",
        "\\Gamma",
        "\\frac{1}{2}+\\frac{3}{4}",
        "\\sqrt{\\frac{a}{b}}",
        "x^{2}+y_{1}",
    ];
    for src in eqs {
        let first = to_latex(&parse_latex(src).unwrap());
        let again = to_latex(&parse_latex(&first).unwrap());
        assert_eq!(first, again, "not idempotent for {src}");
    }
}

#[test]
fn math_linear_quadratic_round_trip() {
    let ast = parse_linear("x=(-b±√(b^2-4ac))/2a").unwrap();
    let latex = to_latex(&ast);
    let reparsed = parse(&latex).unwrap();
    assert_eq!(to_latex(&reparsed), latex);
    assert!(!to_plain(&ast).is_empty());
}

#[test]
fn math_to_plain_basics() {
    let frac = to_plain(&parse_latex("\\frac{a}{b}").unwrap());
    assert!(frac.contains("a/b"), "{frac}");
    assert!(!frac.contains('\\'), "{frac}");

    let root = to_plain(&parse_latex("\\sqrt{x}").unwrap());
    assert!(root.contains('√'), "{root}");
    assert!(!root.contains('\\'), "{root}");

    for src in ["x^2", "\\sum_{i=1}^{n} i", "\\left(a\\right)", "\\hat{x}", "\\begin{pmatrix}a&b\\\\c&d\\end{pmatrix}", "\\text{hi}", "\\alpha"] {
        let plain = to_plain(&parse_latex(src).unwrap());
        assert!(!plain.is_empty(), "{src}");
        assert!(!plain.contains('\\'), "{src} -> {plain}");
    }
}

#[test]
fn math_hostile_inputs() {
    let junk: [&str; 30] = [
        "",
        "\\",
        "{{{",
        "\\frac",
        "\\frac{1}",
        "}}",
        "^{^",
        "_",
        "10 000",
        "×",
        "{",
        "}",
        "\u{FFFD}",
        "\0",
        "😀",
        "\u{202E}abc",
        "\\left(",
        "\\right)",
        "\\begin{pmatrix}",
        "&",
        "\\\\",
        "^",
        "{a",
        "a}",
        "\\sqrt[",
        "\\text{",
        "\\begin{matrix}a",
        "$",
        "#",
        "+",
    ];
    // Only requirement: `parse` returns (Ok or Err) without panicking.
    for s in junk {
        let _ = parse(s);
    }

    // Explicitly documented error cases.
    let bad = parse_latex("\\nope");
    assert!(matches!(&bad, Err(MathError::UnknownCommand(c)) if c.as_str() == "nope"));
    let long = "a".repeat(9000);
    assert!(matches!(parse(&long), Err(MathError::TooLong)));
    let deep = "\\frac{1}{".repeat(40) + &"}".repeat(40);
    assert!(matches!(parse_latex(&deep), Err(MathError::TooDeep)));
}

#[test]
fn math_symbol_round_trip() {
    assert_eq!(cmd_symbol("alpha"), Some("α"));
    assert_eq!(cmd_symbol("times"), Some("×"));
    assert_eq!(cmd_symbol("le"), Some("≤"));
    assert_eq!(cmd_symbol("infty"), Some("∞"));

    assert_eq!(symbol_cmd('α'), Some("alpha"));
    assert_eq!(symbol_cmd('×'), Some("times"));
    assert!(matches!(symbol_cmd('≤'), Some("le") | Some("leq")));
    assert_eq!(symbol_cmd('∞'), Some("infty"));

    assert_eq!(cmd_symbol("not_a_real_command_xyz"), None);
    assert_eq!(symbol_cmd('\u{10FFFF}'), None);
}
