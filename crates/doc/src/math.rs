//! Math expressions: LaTeX and Word/UnicodeMath ("linear") parsing, plus linearisation back to
//! canonical LaTeX or to plain text.
//!
//! [`parse`] picks the flavour: LaTeX when the source contains a backslash command (`\alpha`,
//! `\frac`), the linear flavour otherwise. Both build the same [`MathNode`] tree. [`to_latex`]
//! renders canonical, re-parsable LaTeX; [`to_plain`] renders text for extraction and
//! spell-check.
//!
//! The parsers are total: every input produces a tree or a [`MathError`]. Nesting is capped at
//! `MAX_DEPTH`, input length at `MAX_INPUT` characters, and the walk is by `char` (never by byte
//! offset), so no input can make any of these functions panic.

/// Maximum nesting depth (groups, arguments, delimiters, environments) the parsers accept.
pub(crate) const MAX_DEPTH: usize = 32;
/// Maximum input length in characters, checked before parsing.
pub(crate) const MAX_INPUT: usize = 8192;
/// Depth beyond which the writers stop descending. Parsed trees are far shallower; the cap only
/// keeps a hand-built pathological tree from overflowing the stack.
const MAX_WRITE_DEPTH: usize = 128;

/// The math alphabet of a [`MathNode::Text`] run.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum MathStyle {
    /// Italic: variables and digits.
    Italic,
    /// Upright: operators, punctuation, function names, `\text`.
    Roman,
    /// Bold: `\mathbf`.
    Bold,
    /// Bold italic.
    BoldItalic,
}

/// An accent drawn over (or under) a base.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum AccentKind {
    Hat,
    Bar,
    Tilde,
    Dot,
    Ddot,
    Vec,
    Underline,
    Overline,
}

/// A math expression. A [`MathNode::Row`] is a horizontal sequence.
///
/// (`Eq` is not derived: [`MathNode::Space`] holds an `f32`.)
#[derive(Debug, Clone, PartialEq)]
pub enum MathNode {
    Row(Vec<MathNode>),
    Text(String, MathStyle),
    /// Horizontal space in ems of the base size; may be negative.
    Space(f32),
    Frac {
        num: Box<MathNode>,
        den: Box<MathNode>,
    },
    Sqrt {
        index: Option<Box<MathNode>>,
        body: Box<MathNode>,
    },
    Script {
        base: Box<MathNode>,
        sub: Option<Box<MathNode>>,
        sup: Option<Box<MathNode>>,
    },
    /// A big operator or function name. `op` is the command name (`"sum"`, `"int"`, `"lim"`,
    /// `"sin"`); `limits` asks for limits stacked above/below rather than set beside.
    Big {
        op: String,
        sub: Option<Box<MathNode>>,
        sup: Option<Box<MathNode>>,
        limits: bool,
    },
    /// `left`/`right` are delimiter characters; `.` is an invisible delimiter.
    Delim {
        left: char,
        right: char,
        body: Box<MathNode>,
    },
    Accent {
        kind: AccentKind,
        body: Box<MathNode>,
    },
    /// `left`/`right` as for [`MathNode::Delim`].
    Matrix {
        rows: Vec<Vec<MathNode>>,
        left: char,
        right: char,
    },
    Styled {
        style: MathStyle,
        body: Box<MathNode>,
    },
}

/// Why a math source could not be parsed.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum MathError {
    #[error("unmatched `{0}`")]
    Unmatched(char),
    #[error("unexpected `{0}`")]
    Unexpected(char),
    #[error("unknown command `\\{0}`")]
    UnknownCommand(String),
    #[error("unexpected end of input")]
    UnexpectedEnd,
    #[error("input too long")]
    TooLong,
    #[error("too deeply nested")]
    TooDeep,
}

/// Greek letters and symbol commands: `\cmd` to glyph.
const SYMBOLS: &[(&str, &str)] = &[
    ("alpha", "α"),
    ("beta", "β"),
    ("gamma", "γ"),
    ("delta", "δ"),
    ("epsilon", "ε"),
    ("zeta", "ζ"),
    ("eta", "η"),
    ("theta", "θ"),
    ("iota", "ι"),
    ("kappa", "κ"),
    ("lambda", "λ"),
    ("mu", "μ"),
    ("nu", "ν"),
    ("xi", "ξ"),
    ("omicron", "ο"),
    ("pi", "π"),
    ("rho", "ρ"),
    ("sigma", "σ"),
    ("tau", "τ"),
    ("upsilon", "υ"),
    ("phi", "φ"),
    ("chi", "χ"),
    ("psi", "ψ"),
    ("omega", "ω"),
    ("Gamma", "Γ"),
    ("Delta", "Δ"),
    ("Theta", "Θ"),
    ("Lambda", "Λ"),
    ("Xi", "Ξ"),
    ("Pi", "Π"),
    ("Sigma", "Σ"),
    ("Upsilon", "Υ"),
    ("Phi", "Φ"),
    ("Psi", "Ψ"),
    ("Omega", "Ω"),
    ("varepsilon", "ϵ"),
    ("vartheta", "ϑ"),
    ("varphi", "ϕ"),
    ("varrho", "ϱ"),
    ("varsigma", "ς"),
    ("times", "×"),
    ("div", "÷"),
    ("cdot", "⋅"),
    ("pm", "±"),
    ("mp", "∓"),
    ("ast", "∗"),
    ("star", "⋆"),
    ("circ", "∘"),
    ("bullet", "∙"),
    ("le", "≤"),
    ("leq", "≤"),
    ("ge", "≥"),
    ("geq", "≥"),
    ("ne", "≠"),
    ("neq", "≠"),
    ("approx", "≈"),
    ("equiv", "≡"),
    ("sim", "∼"),
    ("simeq", "≃"),
    ("cong", "≅"),
    ("propto", "∝"),
    ("ll", "≪"),
    ("gg", "≫"),
    ("subset", "⊂"),
    ("subseteq", "⊆"),
    ("supset", "⊃"),
    ("supseteq", "⊇"),
    ("in", "∈"),
    ("notin", "∉"),
    ("ni", "∋"),
    ("cup", "∪"),
    ("cap", "∩"),
    ("emptyset", "∅"),
    ("varnothing", "∅"),
    ("forall", "∀"),
    ("exists", "∃"),
    ("neg", "¬"),
    ("land", "∧"),
    ("lor", "∨"),
    ("wedge", "∧"),
    ("vee", "∨"),
    ("to", "→"),
    ("rightarrow", "→"),
    ("leftarrow", "←"),
    ("Rightarrow", "⇒"),
    ("Leftarrow", "⇐"),
    ("Leftrightarrow", "⇔"),
    ("mapsto", "↦"),
    ("infty", "∞"),
    ("partial", "∂"),
    ("nabla", "∇"),
    ("prime", "′"),
    ("ell", "ℓ"),
    ("hbar", "ℏ"),
    ("Re", "ℜ"),
    ("Im", "ℑ"),
    ("oplus", "⊕"),
    ("otimes", "⊗"),
    ("perp", "⊥"),
    ("parallel", "∥"),
    ("angle", "∠"),
    ("triangle", "△"),
    ("ldots", "…"),
    ("cdots", "⋯"),
    ("vdots", "⋮"),
    ("ddots", "⋱"),
    ("backslash", "\\"),
];

/// Big operators and function names: command, glyph, limits-by-default.
const BIGS: &[(&str, &str, bool)] = &[
    ("sum", "∑", true),
    ("prod", "∏", true),
    ("coprod", "∐", true),
    ("bigcup", "⋃", true),
    ("bigcap", "⋂", true),
    ("lim", "lim", true),
    ("max", "max", true),
    ("min", "min", true),
    ("sup", "sup", true),
    ("inf", "inf", true),
    ("int", "∫", false),
    ("iint", "∬", false),
    ("iiint", "∭", false),
    ("oint", "∮", false),
    ("log", "log", false),
    ("ln", "ln", false),
    ("sin", "sin", false),
    ("cos", "cos", false),
    ("tan", "tan", false),
    ("exp", "exp", false),
    ("det", "det", false),
    ("gcd", "gcd", false),
];

/// The glyph for a symbol or big-operator command (`"alpha"` to `"α"`, `"times"` to `"×"`).
pub fn cmd_symbol(cmd: &str) -> Option<&'static str> {
    if let Some((_, glyph)) = SYMBOLS.iter().find(|(name, _)| *name == cmd) {
        return Some(*glyph);
    }
    BIGS.iter().find(|(name, _, _)| *name == cmd).map(|(_, glyph, _)| *glyph)
}

/// The command for a single-character glyph (`"α"` to `"alpha"`); `None` when unnamed.
pub fn symbol_cmd(ch: char) -> Option<&'static str> {
    let single = |glyph: &str| glyph.starts_with(ch) && glyph.chars().count() == 1;
    if let Some((name, _)) = SYMBOLS.iter().find(|(_, glyph)| single(glyph)) {
        return Some(*name);
    }
    BIGS.iter().find(|(_, glyph, _)| single(glyph)).map(|(name, _, _)| *name)
}

/// Parse LaTeX math.
pub fn parse_latex(src: &str) -> Result<MathNode, MathError> {
    let mut parser = Parser::new(src, false)?;
    let nodes = parser.seq(&[], Mode::Normal)?;
    Ok(node_of(nodes))
}

/// Parse the Word/UnicodeMath "linear" flavour: `√(x)`, `√[3](x)`, `( )`/`[ ]` grouping and `/`
/// between single operands as a fraction.
pub fn parse_linear(src: &str) -> Result<MathNode, MathError> {
    let mut parser = Parser::new(src, true)?;
    let nodes = parser.seq(&[], Mode::Normal)?;
    Ok(node_of(nodes))
}

/// Parse `src`: LaTeX when it contains a backslash command, the linear flavour otherwise.
pub fn parse(src: &str) -> Result<MathNode, MathError> {
    if has_latex_command(src) { parse_latex(src) } else { parse_linear(src) }
}

/// `true` when `src` contains a `\` followed by an ASCII letter (a LaTeX command).
fn has_latex_command(src: &str) -> bool {
    let mut chars = src.chars();
    while let Some(c) = chars.next() {
        if c == '\\' && matches!(chars.clone().next(), Some(n) if n.is_ascii_alphabetic()) {
            return true;
        }
    }
    false
}

/// Render `n` as canonical LaTeX. The result parses back to the same shape:
/// `to_latex(parse_latex(&to_latex(&n))?) == to_latex(&n)`.
pub fn to_latex(n: &MathNode) -> String {
    let mut out = String::new();
    write_latex(n, 0, &mut out);
    out
}

/// Render `n` as plain text: fractions as `a/b`, roots as `√(x)` or `³√(x)`, scripts as
/// `x_{i}`/`x^{2}`, big operators as their glyph plus limits, matrices as `(a,b;c,d)`.
pub fn to_plain(n: &MathNode) -> String {
    let mut out = String::new();
    write_plain(n, 0, &mut out);
    out
}

/// Collapse a statement list: a single node stays bare, several become a [`MathNode::Row`].
fn node_of(mut nodes: Vec<MathNode>) -> MathNode {
    if nodes.len() == 1
        && let Some(one) = nodes.pop()
    {
        return one;
    }
    MathNode::Row(nodes)
}

/// Which delimiters an environment draws (`.` is invisible).
fn env_delims(name: &str) -> Option<(char, char)> {
    match name {
        "matrix" => Some(('.', '.')),
        "pmatrix" => Some(('(', ')')),
        "bmatrix" => Some(('[', ']')),
        "Bmatrix" => Some(('{', '}')),
        "vmatrix" | "Vmatrix" => Some(('|', '|')),
        "cases" => Some(('{', '.')),
        _ => None,
    }
}

/// The environment that draws the given delimiters.
fn matrix_env(left: char, right: char) -> &'static str {
    match (left, right) {
        ('(', ')') => "pmatrix",
        ('[', ']') => "bmatrix",
        ('{', '}') => "Bmatrix",
        ('|', '|') => "vmatrix",
        ('{', '.') => "cases",
        _ => "matrix",
    }
}

/// The accent command name for a kind.
fn accent_cmd(kind: AccentKind) -> &'static str {
    match kind {
        AccentKind::Hat => "hat",
        AccentKind::Bar => "bar",
        AccentKind::Tilde => "tilde",
        AccentKind::Dot => "dot",
        AccentKind::Ddot => "ddot",
        AccentKind::Vec => "vec",
        AccentKind::Underline => "underline",
        AccentKind::Overline => "overline",
    }
}

/// The accent an accent command name stands for.
fn accent_kind(name: &str) -> Option<AccentKind> {
    match name {
        "hat" | "widehat" => Some(AccentKind::Hat),
        "bar" => Some(AccentKind::Bar),
        "tilde" | "widetilde" => Some(AccentKind::Tilde),
        "dot" => Some(AccentKind::Dot),
        "ddot" => Some(AccentKind::Ddot),
        "vec" => Some(AccentKind::Vec),
        "underline" | "underbrace" => Some(AccentKind::Underline),
        "overline" | "overbrace" => Some(AccentKind::Overline),
        _ => None,
    }
}

/// Whether a node may sit on one side of a linear `/` (a token, `(…)`/`[…]`, or `√(…)`).
fn frac_operand(n: &MathNode) -> bool {
    match n {
        MathNode::Delim { left, .. } => *left == '(' || *left == '[',
        MathNode::Text(s, _) => !s.chars().all(char::is_whitespace),
        MathNode::Sqrt { .. } | MathNode::Script { .. } | MathNode::Big { .. } | MathNode::Accent { .. } | MathNode::Styled { .. } => true,
        MathNode::Row(_) | MathNode::Space(_) | MathNode::Frac { .. } | MathNode::Matrix { .. } => false,
    }
}

/// Whether a script base must be braced so the script attaches to the whole base.
fn script_base_needs_braces(base: &MathNode) -> bool {
    match base {
        MathNode::Script { .. } | MathNode::Big { .. } => true,
        MathNode::Row(nodes) => nodes.len() != 1,
        _ => false,
    }
}

/// Where a statement list stops.
#[derive(Clone, Copy, PartialEq, Eq)]
enum Mode {
    /// Only at the caller's terminator characters.
    Normal,
    /// Also at `\right`.
    Right,
    /// Also at `&`, `\\` and `\end`.
    Matrix,
}

struct Parser {
    cs: Vec<char>,
    i: usize,
    depth: usize,
    linear: bool,
}

impl Parser {
    fn new(src: &str, linear: bool) -> Result<Self, MathError> {
        let cs: Vec<char> = src.chars().collect();
        if cs.len() > MAX_INPUT {
            return Err(MathError::TooLong);
        }
        Ok(Parser { cs, i: 0, depth: 0, linear })
    }

    fn peek(&self) -> Option<char> {
        self.cs.get(self.i).copied()
    }

    fn peek_at(&self, k: usize) -> Option<char> {
        self.cs.get(self.i.saturating_add(k)).copied()
    }

    fn bump(&mut self) -> Option<char> {
        let c = self.peek();
        if c.is_some() {
            self.i = self.i.saturating_add(1);
        }
        c
    }

    fn at(&self, c: char) -> bool {
        self.peek() == Some(c)
    }

    fn checkpoint(&self) -> usize {
        self.i
    }

    fn rewind(&mut self, cp: usize) {
        if cp <= self.cs.len() {
            self.i = cp;
        }
    }

    fn enter(&mut self) -> Result<(), MathError> {
        if self.depth >= MAX_DEPTH {
            return Err(MathError::TooDeep);
        }
        self.depth = self.depth.saturating_add(1);
        Ok(())
    }

    fn leave(&mut self) {
        self.depth = self.depth.saturating_sub(1);
    }

    /// Whether `\name` is next, with no command letter directly after the name.
    fn at_cmd(&self, name: &str) -> bool {
        if self.peek() != Some('\\') {
            return false;
        }
        let mut k = self.i.saturating_add(1);
        for want in name.chars() {
            if self.cs.get(k).copied() != Some(want) {
                return false;
            }
            k = k.saturating_add(1);
        }
        !matches!(self.cs.get(k).copied(), Some(c) if c.is_ascii_alphanumeric())
    }

    /// Consume a run of ASCII letters (a command name; may be empty).
    fn read_name(&mut self) -> String {
        let start = self.i;
        while matches!(self.peek(), Some(c) if c.is_ascii_alphabetic()) {
            self.bump();
        }
        self.slice(start, self.i)
    }

    /// Consume characters up to and including `end`.
    fn read_until(&mut self, end: char) -> Result<String, MathError> {
        let mut s = String::new();
        while let Some(c) = self.peek() {
            self.bump();
            if c == end {
                return Ok(s);
            }
            s.push(c);
        }
        Err(MathError::UnexpectedEnd)
    }

    /// The characters in `from..to` as a string (out-of-range indices yield an empty string).
    fn slice(&self, from: usize, to: usize) -> String {
        self.cs.get(from..to).map(|part| part.iter().copied().collect::<String>()).unwrap_or_default()
    }

    /// Push `node`, merging a text run into a preceding run of the same style.
    fn push(&self, out: &mut Vec<MathNode>, node: MathNode) {
        if let MathNode::Text(s, style) = &node {
            let merged = match out.last_mut() {
                Some(MathNode::Text(prev, prev_style)) if prev_style == style => {
                    prev.push_str(s);
                    true
                }
                _ => false,
            };
            if merged {
                return;
            }
        }
        out.push(node);
    }

    /// A single literal: a run of letters/digits (italic) or one other character (Roman).
    fn literal(&mut self) -> MathNode {
        match self.peek() {
            Some(c) if c.is_alphanumeric() => {
                let start = self.i;
                while matches!(self.peek(), Some(ch) if ch.is_alphanumeric()) {
                    self.bump();
                }
                MathNode::Text(self.slice(start, self.i), MathStyle::Italic)
            }
            Some(c) => {
                self.bump();
                MathNode::Text(c.to_string(), MathStyle::Roman)
            }
            None => MathNode::Row(Vec::new()),
        }
    }

    fn seq(&mut self, stop: &[char], mode: Mode) -> Result<Vec<MathNode>, MathError> {
        let mut out: Vec<MathNode> = Vec::new();
        while let Some(c) = self.peek() {
            if stop.contains(&c) {
                break;
            }
            match c {
                '&' => {
                    if mode == Mode::Matrix {
                        break;
                    }
                    return Err(MathError::Unexpected('&'));
                }
                '}' => return Err(MathError::Unmatched('}')),
                '^' | '_' => self.scripts(&mut out)?,
                '\\' if !self.linear => {
                    if self.peek_at(1) == Some('\\') {
                        if mode == Mode::Matrix {
                            break;
                        }
                        return Err(MathError::Unexpected('\\'));
                    }
                    if self.at_cmd("right") {
                        if mode == Mode::Right {
                            break;
                        }
                        return Err(MathError::Unexpected('\\'));
                    }
                    if self.at_cmd("end") {
                        if mode == Mode::Matrix {
                            break;
                        }
                        return Err(MathError::Unexpected('\\'));
                    }
                    if let Some(node) = self.command()? {
                        self.push(&mut out, node);
                    }
                }
                '{' => {
                    let node = self.group('}')?;
                    self.push(&mut out, node);
                }
                _ if self.linear => match c {
                    '(' => {
                        let node = self.paren('(', ')')?;
                        self.push(&mut out, node);
                    }
                    '[' => {
                        let node = self.paren('[', ']')?;
                        self.push(&mut out, node);
                    }
                    ')' | ']' => return Err(MathError::Unexpected(c)),
                    '√' => {
                        let node = self.sqrt_linear();
                        self.push(&mut out, node);
                    }
                    '/' => self.slash(&mut out)?,
                    _ => {
                        let node = self.literal();
                        self.push(&mut out, node);
                    }
                },
                _ => {
                    let node = self.literal();
                    self.push(&mut out, node);
                }
            }
        }
        Ok(out)
    }

    /// Attach the `^`/`_` runs at the cursor to the last statement.
    fn scripts(&mut self, out: &mut Vec<MathNode>) -> Result<(), MathError> {
        let trigger = match self.peek() {
            Some(c) => c,
            None => return Ok(()),
        };
        let mut node = match out.pop() {
            Some(n) => n,
            None => return Err(MathError::Unexpected(trigger)),
        };
        loop {
            let sc = match self.peek() {
                Some('^') => '^',
                Some('_') => '_',
                _ => break,
            };
            self.bump();
            let arg = self.script_arg()?;
            if matches!(node, MathNode::Script { .. } | MathNode::Big { .. }) {
                set_script(&mut node, sc, arg)?;
            } else {
                let (sub, sup) = if sc == '_' { (Some(Box::new(arg)), None) } else { (None, Some(Box::new(arg))) };
                node = MathNode::Script { base: Box::new(node), sub, sup };
            }
        }
        out.push(node);
        Ok(())
    }

    /// One script argument: a braced group, a linear bracket group, or a single token.
    fn script_arg(&mut self) -> Result<MathNode, MathError> {
        match self.peek() {
            None => Err(MathError::UnexpectedEnd),
            Some('{') => self.group('}'),
            Some('(') if self.linear => self.paren_body(')'),
            Some('[') if self.linear => self.paren_body(']'),
            Some('√') if self.linear => Ok(self.sqrt_linear()),
            Some('\\') if !self.linear => match self.command()? {
                Some(node) => Ok(node),
                None => Err(MathError::Unexpected('\\')),
            },
            Some(_) => Ok(self.literal()),
        }
    }

    /// A `{ … }` group (the opening brace is consumed).
    fn group(&mut self, close: char) -> Result<MathNode, MathError> {
        self.bump();
        self.enter()?;
        let res = self.seq(&[close], Mode::Normal);
        self.leave();
        let nodes = res?;
        self.expect_close(close)?;
        Ok(node_of(nodes))
    }

    /// A linear `( … )` or `[ … ]`, kept as a visible [`MathNode::Delim`].
    fn paren(&mut self, open: char, close: char) -> Result<MathNode, MathError> {
        let body = self.paren_body(close)?;
        Ok(MathNode::Delim { left: open, right: close, body: Box::new(body) })
    }

    /// The contents of a linear bracket group, delimiters dropped (they are argument syntax).
    fn paren_body(&mut self, close: char) -> Result<MathNode, MathError> {
        self.bump();
        self.enter()?;
        let res = self.seq(&[close], Mode::Normal);
        self.leave();
        let nodes = res?;
        self.expect_close(close)?;
        Ok(node_of(nodes))
    }

    fn expect_close(&mut self, c: char) -> Result<(), MathError> {
        match self.peek() {
            Some(x) if x == c => {
                self.bump();
                Ok(())
            }
            Some(_) => Err(MathError::Unmatched(c)),
            None => Err(MathError::UnexpectedEnd),
        }
    }

    /// `√(x)` or `√[3](x)`; a bare `√` stays literal (and the cursor never moves backwards over
    /// a character the caller has not seen).
    fn sqrt_linear(&mut self) -> MathNode {
        self.bump();
        let after = self.checkpoint();
        let mut index: Option<MathNode> = None;
        if self.at('[') {
            match self.paren_body(']') {
                Ok(node) => index = Some(node),
                Err(_) => {
                    self.rewind(after);
                    return MathNode::Text("√".to_string(), MathStyle::Roman);
                }
            }
        }
        if self.at('(') {
            match self.paren_body(')') {
                Ok(body) => return MathNode::Sqrt { index: index.map(Box::new), body: Box::new(body) },
                Err(_) => {
                    self.rewind(after);
                    return MathNode::Text("√".to_string(), MathStyle::Roman);
                }
            }
        }
        self.rewind(after);
        MathNode::Text("√".to_string(), MathStyle::Roman)
    }

    /// A `/` after an operand: a fraction when the right side is another single operand.
    fn slash(&mut self, out: &mut Vec<MathNode>) -> Result<(), MathError> {
        let left_ok = matches!(out.last(), Some(n) if frac_operand(n));
        self.bump();
        if !left_ok {
            self.push(out, MathNode::Text("/".to_string(), MathStyle::Roman));
            return Ok(());
        }
        let after_slash = self.checkpoint();
        let mut done = false;
        if let Ok(Some(rhs)) = self.frac_rhs()
            && frac_operand(&rhs)
            && let Some(num) = out.pop()
        {
            self.push(out, MathNode::Frac { num: Box::new(num), den: Box::new(rhs) });
            done = true;
        }
        if !done {
            self.rewind(after_slash);
            self.push(out, MathNode::Text("/".to_string(), MathStyle::Roman));
        }
        Ok(())
    }

    /// One right-hand operand for a linear `/`.
    fn frac_rhs(&mut self) -> Result<Option<MathNode>, MathError> {
        match self.peek() {
            None => Ok(None),
            Some('{') => Ok(Some(self.group('}')?)),
            Some('(') => Ok(Some(self.paren('(', ')')?)),
            Some('[') => Ok(Some(self.paren('[', ']')?)),
            Some('√') => Ok(Some(self.sqrt_linear())),
            Some(c) if c.is_alphanumeric() => Ok(Some(self.literal())),
            Some(_) => Ok(None),
        }
    }

    /// A `\` command. `Ok(None)` for the switches (`\displaystyle`) that draw nothing.
    fn command(&mut self) -> Result<Option<MathNode>, MathError> {
        self.bump();
        match self.peek() {
            None => Err(MathError::UnexpectedEnd),
            Some(c) if !c.is_ascii_alphabetic() => {
                self.bump();
                let node = match c {
                    ',' => MathNode::Space(0.167),
                    ':' => MathNode::Space(0.222),
                    ';' => MathNode::Space(0.278),
                    '!' => MathNode::Space(-0.167),
                    '{' => MathNode::Text("{".to_string(), MathStyle::Roman),
                    '}' => MathNode::Text("}".to_string(), MathStyle::Roman),
                    '|' => MathNode::Text("|".to_string(), MathStyle::Roman),
                    '%' => MathNode::Text("%".to_string(), MathStyle::Roman),
                    '&' => MathNode::Text("&".to_string(), MathStyle::Roman),
                    '#' => MathNode::Text("#".to_string(), MathStyle::Roman),
                    '_' => MathNode::Text("_".to_string(), MathStyle::Roman),
                    '$' => MathNode::Text("$".to_string(), MathStyle::Roman),
                    c if c.is_whitespace() => MathNode::Space(0.333),
                    other => return Err(MathError::Unexpected(other)),
                };
                Ok(Some(node))
            }
            Some(_) => {
                let name = self.read_name();
                self.named(&name)
            }
        }
    }

    /// Dispatch a named command.
    fn named(&mut self, name: &str) -> Result<Option<MathNode>, MathError> {
        match name {
            "" => Err(MathError::Unexpected('\\')),
            "frac" | "dfrac" | "tfrac" => {
                let num = self.required_arg()?;
                let den = self.required_arg()?;
                Ok(Some(MathNode::Frac { num: Box::new(num), den: Box::new(den) }))
            }
            "sqrt" => {
                let index = self.bracket_arg()?;
                let body = self.required_arg()?;
                Ok(Some(MathNode::Sqrt { index: index.map(Box::new), body: Box::new(body) }))
            }
            "begin" => Ok(Some(self.environment()?)),
            "left" => Ok(Some(self.left_right()?)),
            "text" | "operatorname" => Ok(Some(self.verbatim_text()?)),
            // `\mathbb` and `\mathcal` are approximated by the alphabets we have.
            "mathrm" | "mathbb" => Ok(Some(self.styled(MathStyle::Roman)?)),
            "mathbf" | "mathcal" => Ok(Some(self.styled(MathStyle::Bold)?)),
            "mathit" => Ok(Some(self.styled(MathStyle::Italic)?)),
            "displaystyle" | "textstyle" => Ok(None),
            "quad" => Ok(Some(MathNode::Space(1.0))),
            "qquad" => Ok(Some(MathNode::Space(2.0))),
            other => {
                if let Some(kind) = accent_kind(other) {
                    let body = self.required_arg()?;
                    return Ok(Some(MathNode::Accent { kind, body: Box::new(body) }));
                }
                if let Some((_, _, limits)) = BIGS.iter().find(|(cmd, _, _)| *cmd == other) {
                    return Ok(Some(MathNode::Big { op: other.to_string(), sub: None, sup: None, limits: *limits }));
                }
                if let Some(glyph) = cmd_symbol(other) {
                    return Ok(Some(MathNode::Text(glyph.to_string(), MathStyle::Roman)));
                }
                Err(MathError::UnknownCommand(other.to_string()))
            }
        }
    }

    fn styled(&mut self, style: MathStyle) -> Result<MathNode, MathError> {
        let body = self.required_arg()?;
        Ok(MathNode::Styled { style, body: Box::new(body) })
    }

    /// A mandatorily present argument: a group, a linear bracket group, or one token.
    fn required_arg(&mut self) -> Result<MathNode, MathError> {
        match self.peek() {
            None => Err(MathError::UnexpectedEnd),
            Some('{') => self.group('}'),
            Some('(') if self.linear => self.paren_body(')'),
            Some('[') if self.linear => self.paren_body(']'),
            Some('\\') if !self.linear => match self.command()? {
                Some(node) => Ok(node),
                None => Err(MathError::Unexpected('\\')),
            },
            Some(_) => Ok(self.literal()),
        }
    }

    /// An optional `[ … ]` argument (the index of `\sqrt`).
    fn bracket_arg(&mut self) -> Result<Option<MathNode>, MathError> {
        if !self.at('[') {
            return Ok(None);
        }
        self.bump();
        self.enter()?;
        let res = self.seq(&[']'], Mode::Normal);
        self.leave();
        let nodes = res?;
        self.expect_close(']')?;
        Ok(Some(node_of(nodes)))
    }

    /// The body of `\text`, `\operatorname`: read verbatim, spaces and all (nesting counted).
    fn verbatim_text(&mut self) -> Result<MathNode, MathError> {
        match self.peek() {
            None => return Err(MathError::UnexpectedEnd),
            Some('{') => {
                self.bump();
            }
            Some(c) => return Err(MathError::Unexpected(c)),
        }
        self.enter()?;
        let mut text = String::new();
        let mut open = 1usize;
        let mut closed = false;
        while let Some(c) = self.peek() {
            self.bump();
            if c == '}' {
                open = open.saturating_sub(1);
                if open == 0 {
                    closed = true;
                    break;
                }
                text.push(c);
                continue;
            }
            if c == '{' {
                open = open.saturating_add(1);
            }
            text.push(c);
        }
        self.leave();
        if !closed {
            return Err(MathError::UnexpectedEnd);
        }
        Ok(MathNode::Text(text, MathStyle::Roman))
    }

    /// `\begin{name} … \end{name}`.
    fn environment(&mut self) -> Result<MathNode, MathError> {
        match self.peek() {
            None => return Err(MathError::UnexpectedEnd),
            Some('{') => {
                self.bump();
            }
            Some(c) => return Err(MathError::Unexpected(c)),
        }
        let name = self.read_until('}')?;
        let (left, right) = match env_delims(&name) {
            Some(delims) => delims,
            None => return Err(MathError::UnknownCommand(name)),
        };
        self.enter()?;
        let res = self.matrix_rows(&name);
        self.leave();
        Ok(MathNode::Matrix { rows: res?, left, right })
    }

    /// Cells (`&`) and rows (`\\`) up to `\end{name}`.
    fn matrix_rows(&mut self, name: &str) -> Result<Vec<Vec<MathNode>>, MathError> {
        let mut rows: Vec<Vec<MathNode>> = Vec::new();
        let mut row: Vec<MathNode> = Vec::new();
        loop {
            let cell = self.seq(&[], Mode::Matrix)?;
            row.push(node_of(cell));
            match self.peek() {
                Some('&') => {
                    self.bump();
                }
                Some('\\') if self.peek_at(1) == Some('\\') => {
                    self.bump();
                    self.bump();
                    rows.push(std::mem::take(&mut row));
                }
                Some('\\') if self.at_cmd("end") => {
                    self.bump();
                    let _ = self.read_name();
                    match self.peek() {
                        None => return Err(MathError::UnexpectedEnd),
                        Some('{') => {
                            self.bump();
                        }
                        Some(c) => return Err(MathError::Unexpected(c)),
                    }
                    let closing = self.read_until('}')?;
                    if closing != name {
                        return Err(MathError::UnknownCommand(closing));
                    }
                    rows.push(row);
                    return Ok(rows);
                }
                Some(c) => return Err(MathError::Unexpected(c)),
                None => return Err(MathError::UnexpectedEnd),
            }
        }
    }

    /// `\left<delim> … \right<delim>`.
    fn left_right(&mut self) -> Result<MathNode, MathError> {
        let left = self.delim_token()?;
        self.enter()?;
        let res = self.seq(&[], Mode::Right);
        self.leave();
        let nodes = res?;
        if !self.at_cmd("right") {
            return Err(MathError::UnexpectedEnd);
        }
        self.bump();
        let _ = self.read_name();
        let right = self.delim_token()?;
        Ok(MathNode::Delim { left, right, body: Box::new(node_of(nodes)) })
    }

    /// The delimiter after `\left`/`\right`: one character or a delimiter command.
    fn delim_token(&mut self) -> Result<char, MathError> {
        match self.peek() {
            None => Err(MathError::UnexpectedEnd),
            Some('\\') => {
                self.bump();
                match self.peek() {
                    None => Err(MathError::UnexpectedEnd),
                    Some('{') => {
                        self.bump();
                        Ok('{')
                    }
                    Some('}') => {
                        self.bump();
                        Ok('}')
                    }
                    Some('|') => {
                        self.bump();
                        Ok('|')
                    }
                    Some(c) if c.is_ascii_alphabetic() => {
                        let name = self.read_name();
                        match name.as_str() {
                            "langle" => Ok('⟨'),
                            "rangle" => Ok('⟩'),
                            "lfloor" => Ok('⌊'),
                            "rfloor" => Ok('⌋'),
                            "lceil" => Ok('⌈'),
                            "rceil" => Ok('⌉'),
                            "vert" | "lvert" | "rvert" | "mid" => Ok('|'),
                            "Vert" | "lVert" | "rVert" => Ok('‖'),
                            "backslash" => Ok('\\'),
                            other => Err(MathError::UnknownCommand(other.to_string())),
                        }
                    }
                    Some(c) => {
                        self.bump();
                        Err(MathError::Unexpected(c))
                    }
                }
            }
            Some(c) if matches!(c, '(' | ')' | '[' | ']' | '{' | '}' | '|' | '.' | '⟨' | '⟩' | '⌊' | '⌋' | '⌈' | '⌉' | '‖') => {
                self.bump();
                Ok(c)
            }
            Some(c) => Err(MathError::Unexpected(c)),
        }
    }
}

/// Store `arg` into the sub/sup slot of a script-like node; a filled slot is an error.
fn set_script(node: &mut MathNode, sc: char, arg: MathNode) -> Result<(), MathError> {
    match node {
        MathNode::Script { sub, sup, .. } | MathNode::Big { sub, sup, .. } => {
            if sc == '^' {
                if sup.is_some() {
                    return Err(MathError::Unexpected('^'));
                }
                *sup = Some(Box::new(arg));
            } else {
                if sub.is_some() {
                    return Err(MathError::Unexpected('_'));
                }
                *sub = Some(Box::new(arg));
            }
            Ok(())
        }
        _ => Ok(()),
    }
}

fn write_latex(n: &MathNode, depth: usize, out: &mut String) {
    if depth > MAX_WRITE_DEPTH {
        return;
    }
    match n {
        MathNode::Row(nodes) => {
            for node in nodes {
                write_latex(node, depth + 1, out);
            }
        }
        MathNode::Text(s, style) => write_text_latex(s, *style, out),
        MathNode::Space(ems) => out.push_str(space_latex(*ems)),
        MathNode::Frac { num, den } => {
            out.push_str("\\frac{");
            write_latex(num, depth + 1, out);
            out.push_str("}{");
            write_latex(den, depth + 1, out);
            out.push('}');
        }
        MathNode::Sqrt { index, body } => {
            out.push_str("\\sqrt");
            if let Some(idx) = index {
                out.push('[');
                write_latex(idx, depth + 1, out);
                out.push(']');
            }
            out.push('{');
            write_latex(body, depth + 1, out);
            out.push('}');
        }
        MathNode::Script { base, sub, sup } => {
            let brace = script_base_needs_braces(base);
            if brace {
                out.push('{');
            }
            write_latex(base, depth + 1, out);
            if brace {
                out.push('}');
            }
            if let Some(sub) = sub {
                out.push_str("_{");
                write_latex(sub, depth + 1, out);
                out.push('}');
            }
            if let Some(sup) = sup {
                out.push_str("^{");
                write_latex(sup, depth + 1, out);
                out.push('}');
            }
        }
        MathNode::Big { op, sub, sup, .. } => {
            write_big_latex(op, out);
            if let Some(sub) = sub {
                out.push_str("_{");
                write_latex(sub, depth + 1, out);
                out.push('}');
            }
            if let Some(sup) = sup {
                out.push_str("^{");
                write_latex(sup, depth + 1, out);
                out.push('}');
            }
        }
        MathNode::Delim { left, right, body } => {
            out.push_str("\\left");
            out.push_str(&delim_latex(*left));
            write_latex(body, depth + 1, out);
            out.push_str("\\right");
            out.push_str(&delim_latex(*right));
        }
        MathNode::Accent { kind, body } => {
            out.push('\\');
            out.push_str(accent_cmd(*kind));
            out.push('{');
            write_latex(body, depth + 1, out);
            out.push('}');
        }
        MathNode::Matrix { rows, left, right } => {
            let env = matrix_env(*left, *right);
            out.push_str("\\begin{");
            out.push_str(env);
            out.push('}');
            for (r, row) in rows.iter().enumerate() {
                if r > 0 {
                    out.push_str("\\\\");
                }
                for (c, cell) in row.iter().enumerate() {
                    if c > 0 {
                        out.push('&');
                    }
                    write_latex(cell, depth + 1, out);
                }
            }
            out.push_str("\\end{");
            out.push_str(env);
            out.push('}');
        }
        MathNode::Styled { style, body } => {
            out.push_str(style_cmd(*style));
            out.push('{');
            write_latex(body, depth + 1, out);
            out.push('}');
        }
    }
}

fn style_cmd(style: MathStyle) -> &'static str {
    match style {
        MathStyle::Italic => "\\mathit",
        MathStyle::Roman => "\\mathrm",
        // Bold italic has no own command; bold is the closest re-parsable rendering.
        MathStyle::Bold | MathStyle::BoldItalic => "\\mathbf",
    }
}

fn write_big_latex(op: &str, out: &mut String) {
    if BIGS.iter().any(|(cmd, _, _)| *cmd == op) {
        out.push('\\');
        out.push_str(op);
        return;
    }
    let mut chars = op.chars();
    if let (Some(only), None) = (chars.next(), chars.next())
        && let Some(cmd) = symbol_cmd(only)
    {
        out.push('\\');
        out.push_str(cmd);
        return;
    }
    out.push_str("\\text{");
    out.push_str(op);
    out.push('}');
}

fn write_text_latex(s: &str, style: MathStyle, out: &mut String) {
    match style {
        MathStyle::Italic => out.push_str(s),
        MathStyle::Roman => {
            // Words (and anything holding a space) go through `\text`; other runs (operators,
            // Greek letters, raw math characters) become named commands or raw characters.
            if s.chars().any(|c| c.is_ascii_alphanumeric() || c.is_whitespace()) {
                out.push_str("\\text{");
                out.push_str(s);
                out.push('}');
            } else {
                for c in s.chars() {
                    push_char_latex(c, out);
                }
            }
        }
        MathStyle::Bold => {
            out.push_str("\\mathbf{");
            out.push_str(s);
            out.push('}');
        }
        MathStyle::BoldItalic => {
            out.push_str("\\mathbf{\\mathit{");
            out.push_str(s);
            out.push_str("}}");
        }
    }
}

fn push_char_latex(c: char, out: &mut String) {
    if let Some(cmd) = symbol_cmd(c) {
        out.push('\\');
        out.push_str(cmd);
    } else if matches!(c, '{' | '}' | '%' | '&' | '#' | '_' | '$' | '|') {
        out.push('\\');
        out.push(c);
    } else if matches!(c, '~' | '^') {
        out.push_str("\\text{");
        out.push(c);
        out.push('}');
    } else {
        out.push(c);
    }
}

/// A delimiter character as it is written after `\left`/`\right`.
fn delim_latex(c: char) -> String {
    let cmd = match c {
        '(' => "(",
        ')' => ")",
        '[' => "[",
        ']' => "]",
        '{' => "\\{",
        '}' => "\\}",
        '|' => "|",
        '.' => ".",
        '⟨' => "\\langle",
        '⟩' => "\\rangle",
        '⌊' => "\\lfloor",
        '⌋' => "\\rfloor",
        '⌈' => "\\lceil",
        '⌉' => "\\rceil",
        '‖' => "\\Vert",
        '\\' => "\\backslash",
        other => return other.to_string(),
    };
    cmd.to_string()
}

fn space_latex(ems: f32) -> &'static str {
    if (ems - 0.167).abs() < 1e-4 {
        "\\,"
    } else if (ems - 0.222).abs() < 1e-4 {
        "\\:"
    } else if (ems - 0.278).abs() < 1e-4 {
        "\\;"
    } else if (ems + 0.167).abs() < 1e-4 {
        "\\!"
    } else if (ems - 0.333).abs() < 1e-4 {
        "\\ "
    } else if (ems - 1.0).abs() < 1e-4 {
        "\\quad"
    } else if (ems - 2.0).abs() < 1e-4 {
        "\\qquad"
    } else {
        "\\,"
    }
}

fn write_plain(n: &MathNode, depth: usize, out: &mut String) {
    if depth > MAX_WRITE_DEPTH {
        return;
    }
    match n {
        MathNode::Row(nodes) => {
            for node in nodes {
                write_plain(node, depth + 1, out);
            }
        }
        MathNode::Text(s, _) => out.push_str(s),
        MathNode::Space(_) => out.push(' '),
        MathNode::Frac { num, den } => {
            write_plain_side(num, depth + 1, out);
            out.push('/');
            write_plain_side(den, depth + 1, out);
        }
        MathNode::Sqrt { index, body } => {
            if let Some(idx) = index {
                let text = plain_text(idx, depth + 1);
                out.push_str(&superscript(&text));
            }
            out.push('√');
            out.push('(');
            write_plain(body, depth + 1, out);
            out.push(')');
        }
        MathNode::Script { base, sub, sup } => {
            write_plain(base, depth + 1, out);
            if let Some(sub) = sub {
                out.push_str("_{");
                write_plain(sub, depth + 1, out);
                out.push('}');
            }
            if let Some(sup) = sup {
                out.push_str("^{");
                write_plain(sup, depth + 1, out);
                out.push('}');
            }
        }
        MathNode::Big { op, sub, sup, .. } => {
            out.push_str(&big_plain(op));
            if let Some(sub) = sub {
                out.push_str("_{");
                write_plain(sub, depth + 1, out);
                out.push('}');
            }
            if let Some(sup) = sup {
                out.push_str("^{");
                write_plain(sup, depth + 1, out);
                out.push('}');
            }
        }
        MathNode::Delim { left, right, body } => {
            if *left != '.' {
                out.push(*left);
            }
            write_plain(body, depth + 1, out);
            if *right != '.' {
                out.push(*right);
            }
        }
        MathNode::Accent { body, .. } => write_plain(body, depth + 1, out),
        MathNode::Matrix { rows, left, right } => {
            if *left != '.' {
                out.push(*left);
            }
            for (r, row) in rows.iter().enumerate() {
                if r > 0 {
                    out.push(';');
                }
                for (c, cell) in row.iter().enumerate() {
                    if c > 0 {
                        out.push(',');
                    }
                    write_plain(cell, depth + 1, out);
                }
            }
            if *right != '.' {
                out.push(*right);
            }
        }
        MathNode::Styled { body, .. } => write_plain(body, depth + 1, out),
    }
}

/// One side of a plain-text fraction: parenthesised unless it is a single atom.
fn write_plain_side(n: &MathNode, depth: usize, out: &mut String) {
    if matches!(n, MathNode::Row(nodes) if nodes.len() != 1) {
        out.push('(');
        write_plain(n, depth + 1, out);
        out.push(')');
    } else {
        write_plain(n, depth + 1, out);
    }
}

fn plain_text(n: &MathNode, depth: usize) -> String {
    let mut out = String::new();
    write_plain(n, depth, &mut out);
    out
}

/// The visible characters of a big operator (`"sum"` to `"∑"`).
fn big_plain(op: &str) -> String {
    match cmd_symbol(op) {
        Some(glyph) => glyph.to_string(),
        None => op.to_string(),
    }
}

/// Superscript digits for a root index (`"3"` to `"³"`); other text is parenthesised.
fn superscript(s: &str) -> String {
    let mut out = String::new();
    for c in s.chars() {
        let mapped = match c {
            '0' => '⁰',
            '1' => '¹',
            '2' => '²',
            '3' => '³',
            '4' => '⁴',
            '5' => '⁵',
            '6' => '⁶',
            '7' => '⁷',
            '8' => '⁸',
            '9' => '⁹',
            '+' => '⁺',
            '-' => '⁻',
            '(' => '⁽',
            ')' => '⁾',
            'n' => 'ⁿ',
            'i' => 'ⁱ',
            _ => return format!("({s})"),
        };
        out.push(mapped);
    }
    out
}
