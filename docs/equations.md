# Equations (LaTeX)

WordCraft typesets equations from a LaTeX subset. An equation is an inline object
(`wordcraft_doc::para::InlineObject::Equation`) whose `linear` field holds its **source** — LaTeX, or
Word's linear (UnicodeMath-flavoured) form — and whose `display` flag says whether it sits in the
text flow or on a line of its own.

The pipeline is:

| Step | Crate | What happens |
|---|---|---|
| Parse | `doc::math` | `parse(src)` → `MathNode` tree (LaTeX when the source has a `\` command, else linear) |
| Typeset | `layout::math` | `typeset_source(src, size, bold, italic)` → `MathBox` (positioned glyphs + rules, baseline-relative, y up) |
| Place | `layout::para` | one atomic cluster (`adv`/`obj_h`/`obj_d`) plus a `MathItem` in `ParaLayout.maths` |
| Draw | `layout::display` | `Draw::Glyphs` batches (grouped by face/size) and `Draw::Line` rules, shared by the rasteriser, the PDF exporter and thumbnails |
| Round-trip | `docx` | structured OMML (`m:f`, `m:rad`, `m:sSup`/`m:sSub`/`m:sSubSup`, `m:nary`, `m:d`, `m:acc`, `m:bar`, `m:m`) both ways |

## Commands

| Command | Params | Notes |
|---|---|---|
| `insert.equation` | `{"latex"?: string, "linear"?: string, "display"?: bool}` | `Alt+=`. Source = `latex`, else `linear`, else `a^2+b^2=c^2`. Rejects a source that does not parse (> 8192 chars too) instead of inserting it |
| `equation.source` | — | `{"latex": string, "display": bool}` for the equation at the caret |

The Insert › Symbols **Equation** button opens the equation dialog: a LaTeX field, a *Display
equation (own line)* checkbox, a live preview, and Insert/Replace. When the caret is on an
equation the dialog loads its source and replaces it. Programmatic callers never open dialogs —
they call `insert.equation` directly.

## Supported LaTeX

Grouping with `{}`, single-token fallback (`x^2`, `x_ij`), and `^`/`_` in either order (one
`Script` node; a repeated `^` or `_` is an error).

- **Fractions and radicals** — `\frac`, `\dfrac`, `\tfrac`, `\sqrt`, `\sqrt[3]`
- **Big operators** — `\sum`, `\prod`, `\coprod`, `\bigcup`, `\bigcap`, `\int`, `\iint`, `\iiint`,
  `\oint`, `\lim`, `\max`, `\min`, `\sup`, `\inf`, `\log`, `\ln`, `\sin`, `\cos`, `\tan`, `\exp`,
  `\det`, `\gcd`. The `\sum` family stacks its limits; the integral family sets them to the side
- **Delimiters** — `\left( \right)`, `[`, `\{`, `|`, `.` (invisible), `\langle`, `\rangle`,
  `\lfloor`, `\rfloor`, `\lceil`, `\rceil`, `\vert`, `\Vert`
- **Accents** — `\hat`, `\widehat`, `\bar`, `\overline`, `\tilde`, `\widetilde`, `\dot`, `\ddot`,
  `\vec`, `\underline`, `\overbrace`, `\underbrace`
- **Environments** — `\begin{matrix|pmatrix|bmatrix|Bmatrix|vmatrix|Vmatrix|cases} … \end{…}`
  with `&` between cells and `\\` between rows
- **Styles** — `\text`, `\mathrm`, `\mathbf`, `\mathit`, `\mathbb`/`\mathcal` (approximated by the
  Roman and Bold alphabets we have), `\operatorname`, `\displaystyle`, `\textstyle`
- **Spacing** — `\,` `\:` `\;` `\!` `\ ` `\quad` `\qquad`
- **Symbols** — 107 named commands (`\alpha`, `\times`, `\le`, `\infty`, …) plus raw Unicode math
  characters (`±`, `×`, `≤`, `√`, `∑`, `∫`, `α`, …)

Anything else is an error, and the caller falls back to the source text: an unknown `\command`,
an unmatched brace, an unfinished argument. `parse` never panics on any input (length and nesting
are capped).

## Word's linear form

Sources without a `\` command are read as Word's linear form: `^`/`_` scripts, `√(x)`,
`√[3](x)`, `(`/`)` grouping, and `a/b` or `(…)/(…)` as a fraction when each side is a single
token or a bracket group. `x=(-b±√(b^2-4ac))/2a` is read as a fraction.

## Export

| Format | Equation |
|---|---|
| DOCX | structured OMML; Word opens and edits it |
| Markdown | `$…$` inline, a `$$` block for a display equation |
| HTML | `<span class="math">\(…\)</span>`, `<div class="math">\[…\]</div>` |
| TXT, RTF, ODT, PDF text | readable plain text from `math::to_plain` (`x=(-b±√(b²-4ac))/2a`) |

## Limits

There is no interactive equation toolbar (Word's Equation tab) and no MathML or ink input;
`insert.inkEquation` and `draw.inkToMath` are still unimplemented. A font without the OpenType
MATH table is fine — sizes, shifts and rules come from the typesetter's own constants, and
delimiters and radicals are stretched by scaling their glyphs.
