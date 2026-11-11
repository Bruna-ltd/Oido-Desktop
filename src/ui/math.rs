//! Math in replies (`$x^2$`, `$$\frac{a}{b}$$`, `\(…\)`, `\[…\]`), written out in Unicode before
//! the Markdown is drawn: `\alpha` is α, `x^2` is x², `\frac{a+b}{2}` is (a+b)/2. Nothing is
//! typeset, so stacked fractions and matrices read as a line, but most of what a chat says reads
//! as it should. Code is left alone, and a `$` only opens math as pandoc has it: `$5 and $10` is
//! money.

use std::borrow::Cow;
use std::iter::Peekable;
use std::str::Chars;

/// `markdown` with its math written out in Unicode; borrowed when it has none.
pub fn unicode(markdown: &str) -> Cow<'_, str> {
    if !markdown.contains('$') && !markdown.contains("\\(") && !markdown.contains("\\[") {
        return Cow::Borrowed(markdown);
    }
    let mut out = String::with_capacity(markdown.len());
    let mut fence: Option<&str> = None;
    // Text between fences, taken whole: display math often spans lines.
    let mut start = 0;
    let mut at = 0;
    for line in markdown.split_inclusive('\n') {
        let opener = line.trim_start();
        match fence {
            Some(f) => {
                if opener.starts_with(f) {
                    fence = None;
                }
                out.push_str(line);
            }
            None if opener.starts_with("```") || opener.starts_with("~~~") => {
                prose(&markdown[start..at], &mut out);
                fence = Some(&opener[..3]);
                out.push_str(line);
            }
            None => {}
        }
        at += line.len();
        if fence.is_some() || opener.starts_with("```") || opener.starts_with("~~~") {
            start = at;
        }
    }
    if fence.is_none() {
        prose(&markdown[start..], &mut out);
    }
    Cow::Owned(out)
}

/// Text outside code fences: code spans copied, math written out.
fn prose(text: &str, out: &mut String) {
    let mut rest = text;
    while let Some(at) = rest.find(['`', '$', '\\']) {
        out.push_str(&rest[..at]);
        rest = &rest[at..];
        if rest.starts_with('`') {
            // A code span: its run of backticks up to the same run again.
            let ticks = rest.len() - rest.trim_start_matches('`').len();
            let fence = &rest[..ticks];
            // Unmatched, the backticks are only backticks.
            let end = rest[ticks..]
                .find(fence)
                .map_or(ticks, |i| ticks + i + ticks);
            out.push_str(&rest[..end]);
            rest = &rest[end..];
        } else if let Some((tex, used)) = math(rest) {
            out.push_str(&escape(&latex(tex)));
            rest = &rest[used..];
        } else {
            // A `$` that opens nothing, or a `\` escaping the next character: both stay.
            let n = rest
                .chars()
                .take(2)
                .map(char::len_utf8)
                .sum::<usize>()
                .min(rest.len());
            let n = if rest.starts_with('\\') { n } else { 1 };
            out.push_str(&rest[..n]);
            rest = &rest[n..];
        }
    }
    out.push_str(rest);
}

/// Math at the start of `s`: its TeX and how many bytes it takes up with its delimiters.
fn math(s: &str) -> Option<(&str, usize)> {
    for (open, close) in [("$$", "$$"), ("\\[", "\\]"), ("\\(", "\\)")] {
        if let Some(body) = s.strip_prefix(open) {
            let end = body.find(close)?;
            let tex = &body[..end];
            return (!tex.trim().is_empty()).then_some((tex, open.len() + end + close.len()));
        }
    }
    // Pandoc's rule: `$` then no space, and a closing `$` with no space before it and no digit
    // after it.
    let body = s.strip_prefix('$')?;
    if body.starts_with(char::is_whitespace) || body.starts_with('$') {
        return None;
    }
    let mut from = 0;
    while let Some(i) = body[from..].find('$') {
        let end = from + i;
        let before = body[..end].chars().next_back();
        let after = body[end + 1..].chars().next();
        if body[..end].ends_with('\\') {
            from = end + 1;
            continue;
        }
        if body[..end].contains("\n\n") {
            return None;
        }
        if before.is_some_and(|c| !c.is_whitespace()) && !after.is_some_and(|c| c.is_ascii_digit())
        {
            return Some((&body[..end], end + 2));
        }
        return None;
    }
    None
}

/// Backslash-escapes what Markdown would read as markup, so `a*b` stays a star.
fn escape(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    for c in s.chars() {
        if matches!(
            c,
            '\\' | '*' | '_' | '[' | ']' | '<' | '>' | '`' | '#' | '|' | '~'
        ) {
            out.push('\\');
        }
        out.push(c);
    }
    out
}

/// TeX written out in Unicode, as far as it can be.
fn latex(tex: &str) -> String {
    let mut p = Tex {
        s: tex.chars().peekable(),
    };
    let out = p.seq(None);
    out.split_whitespace()
        .collect::<Vec<_>>()
        .join(" ")
        .replace(" ,", ",")
        .replace(" ;", ";")
}

struct Tex<'a> {
    s: Peekable<Chars<'a>>,
}

impl Tex<'_> {
    /// Atoms up to `stop` (consumed) or the end.
    fn seq(&mut self, stop: Option<char>) -> String {
        let mut out = String::new();
        while let Some(&c) = self.s.peek() {
            if Some(c) == stop {
                self.s.next();
                break;
            }
            match c {
                '^' | '_' => {
                    self.s.next();
                    let arg = self.atom();
                    out.push_str(&script(&arg, c == '^'));
                }
                _ => out.push_str(&self.atom()),
            }
        }
        out
    }

    /// One character, `{group}` or `\command` with its arguments.
    fn atom(&mut self) -> String {
        match self.s.next() {
            None => String::new(),
            Some('{') => self.seq(Some('}')),
            Some('\\') => self.command(),
            Some('-') => "−".into(),
            // Matrix and aligned columns.
            Some('&') => ", ".into(),
            Some('*') => "∗".into(),
            Some('\'') => "′".into(),
            Some(c) => c.to_string(),
        }
    }

    fn command(&mut self) -> String {
        let mut name = String::new();
        while let Some(&c) = self.s.peek() {
            if !c.is_ascii_alphabetic() {
                break;
            }
            name.push(c);
            self.s.next();
        }
        if name.is_empty() {
            // `\{`, `\,`, `\\` and the like.
            return match self.s.next() {
                Some(',' | ';' | ':' | ' ') => " ".into(),
                Some('!') => String::new(),
                Some('\\') => "; ".into(),
                Some(c) => c.to_string(),
                None => String::new(),
            };
        }
        match name.as_str() {
            "frac" | "dfrac" | "tfrac" => {
                let (a, b) = (self.atom(), self.atom());
                format!("{}/{}", group(&a), group(&b))
            }
            "sqrt" => {
                let root = if self.s.peek() == Some(&'[') {
                    self.s.next();
                    Some(self.seq(Some(']')))
                } else {
                    None
                };
                let x = self.atom();
                let sign = match root.as_deref() {
                    Some("3") => "∛".to_owned(),
                    Some("4") => "∜".to_owned(),
                    Some(n) => format!("{}√", script(n, true)),
                    None => "√".to_owned(),
                };
                format!("{sign}{}", group(&x))
            }
            "text" | "mathrm" | "mathit" | "mathbf" | "mathsf" | "mathtt" | "textbf" | "textit"
            | "operatorname" | "boldsymbol" | "mbox" => self.atom(),
            "mathbb" => self.atom().chars().map(double_struck).collect(),
            // An environment's name: its rows and columns are written out as they come.
            "begin" | "end" => {
                self.atom();
                String::new()
            }
            "left" | "right" | "big" | "Big" | "bigg" | "Bigg" | "displaystyle" | "limits" => {
                String::new()
            }
            "quad" | "qquad" => "  ".into(),
            "hat" | "bar" | "overline" | "vec" | "dot" | "ddot" | "tilde" => {
                let mark = match name.as_str() {
                    "hat" => '\u{302}',
                    "bar" | "overline" => '\u{305}',
                    "vec" => '\u{20D7}',
                    "dot" => '\u{307}',
                    "ddot" => '\u{308}',
                    _ => '\u{303}',
                };
                let x = self.atom();
                if x.chars().count() == 1 {
                    format!("{x}{mark}")
                } else {
                    x
                }
            }
            _ => match symbol(&name) {
                Some(s) => s.into(),
                None => format!("\\{name}"),
            },
        }
    }
}

/// `x` as one operand: in brackets when it is more than a single term.
fn group(x: &str) -> String {
    let x = x.trim();
    if x.chars().count() > 1 && x.contains([' ', '+', '−', '-', '×', '·', '/', '=', ',']) {
        format!("({x})")
    } else {
        x.to_owned()
    }
}

/// `x` raised or lowered: in Unicode's superscripts or subscripts when it has them all, else
/// `^(x)` or `_(x)`.
fn script(x: &str, up: bool) -> String {
    let small: Option<String> = x
        .chars()
        .map(|c| if up { superscript(c) } else { subscript(c) })
        .collect();
    match small {
        Some(s) if !s.is_empty() => s,
        _ => format!("{}{}", if up { '^' } else { '_' }, group(x)),
    }
}

fn superscript(c: char) -> Option<char> {
    const FROM: &str = "0123456789+−-=()niabcdefghjklmoprstuvwxyzT′";
    const TO: &str = "⁰¹²³⁴⁵⁶⁷⁸⁹⁺⁻⁻⁼⁽⁾ⁿⁱᵃᵇᶜᵈᵉᶠᵍʰʲᵏˡᵐᵒᵖʳˢᵗᵘᵛʷˣʸᶻᵀ′";
    FROM.chars()
        .position(|f| f == c)
        .and_then(|i| TO.chars().nth(i))
}

fn subscript(c: char) -> Option<char> {
    const FROM: &str = "0123456789+−-=()aehijklmnoprstuvx";
    const TO: &str = "₀₁₂₃₄₅₆₇₈₉₊₋₋₌₍₎ₐₑₕᵢⱼₖₗₘₙₒₚᵣₛₜᵤᵥₓ";
    FROM.chars()
        .position(|f| f == c)
        .and_then(|i| TO.chars().nth(i))
}

fn double_struck(c: char) -> char {
    match c {
        'R' => 'ℝ',
        'N' => 'ℕ',
        'Z' => 'ℤ',
        'Q' => 'ℚ',
        'C' => 'ℂ',
        'P' => 'ℙ',
        'E' => '𝔼',
        c => c,
    }
}

/// A command that stands for one symbol or word.
fn symbol(name: &str) -> Option<&'static str> {
    Some(match name {
        "alpha" => "α",
        "beta" => "β",
        "gamma" => "γ",
        "delta" => "δ",
        "epsilon" | "varepsilon" => "ε",
        "zeta" => "ζ",
        "eta" => "η",
        "theta" | "vartheta" => "θ",
        "iota" => "ι",
        "kappa" => "κ",
        "lambda" => "λ",
        "mu" => "μ",
        "nu" => "ν",
        "xi" => "ξ",
        "pi" => "π",
        "rho" | "varrho" => "ρ",
        "sigma" => "σ",
        "tau" => "τ",
        "upsilon" => "υ",
        "phi" | "varphi" => "φ",
        "chi" => "χ",
        "psi" => "ψ",
        "omega" => "ω",
        "Gamma" => "Γ",
        "Delta" => "Δ",
        "Theta" => "Θ",
        "Lambda" => "Λ",
        "Xi" => "Ξ",
        "Pi" => "Π",
        "Sigma" => "Σ",
        "Phi" => "Φ",
        "Psi" => "Ψ",
        "Omega" => "Ω",
        "times" => "×",
        "cdot" => "·",
        "div" => "÷",
        "pm" => "±",
        "mp" => "∓",
        "le" | "leq" => "≤",
        "ge" | "geq" => "≥",
        "ne" | "neq" => "≠",
        "approx" => "≈",
        "equiv" => "≡",
        "sim" => "∼",
        "simeq" => "≃",
        "propto" => "∝",
        "ll" => "≪",
        "gg" => "≫",
        "infty" => "∞",
        "partial" => "∂",
        "nabla" => "∇",
        "sum" => "∑",
        "prod" => "∏",
        "int" => "∫",
        "iint" => "∬",
        "oint" => "∮",
        "in" => "∈",
        "notin" => "∉",
        "ni" => "∋",
        "subset" => "⊂",
        "subseteq" => "⊆",
        "supset" => "⊃",
        "supseteq" => "⊇",
        "cup" => "∪",
        "cap" => "∩",
        "setminus" => "∖",
        "emptyset" | "varnothing" => "∅",
        "forall" => "∀",
        "exists" => "∃",
        "neg" | "lnot" => "¬",
        "land" | "wedge" => "∧",
        "lor" | "vee" => "∨",
        "oplus" => "⊕",
        "otimes" => "⊗",
        "to" | "rightarrow" => "→",
        "leftarrow" | "gets" => "←",
        "Rightarrow" | "implies" => "⇒",
        "Leftarrow" => "⇐",
        "iff" | "Leftrightarrow" => "⇔",
        "leftrightarrow" => "↔",
        "mapsto" => "↦",
        "uparrow" => "↑",
        "downarrow" => "↓",
        "ldots" | "dots" => "…",
        "cdots" => "⋯",
        "vdots" => "⋮",
        "circ" => "∘",
        "degree" => "°",
        "angle" => "∠",
        "perp" => "⊥",
        "parallel" => "∥",
        "mid" => "∣",
        "ell" => "ℓ",
        "hbar" => "ℏ",
        "langle" => "⟨",
        "rangle" => "⟩",
        "lceil" => "⌈",
        "rceil" => "⌉",
        "lfloor" => "⌊",
        "rfloor" => "⌋",
        "lvert" | "rvert" | "vert" => "|",
        "lVert" | "rVert" | "Vert" => "‖",
        "prime" => "′",
        "star" => "⋆",
        "dagger" => "†",
        "therefore" => "∴",
        "because" => "∵",
        "sin" => "sin ",
        "cos" => "cos ",
        "tan" => "tan ",
        "log" => "log ",
        "ln" => "ln ",
        "exp" => "exp ",
        "lim" => "lim",
        "max" => "max",
        "min" => "min",
        "det" => "det",
        _ => return None,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn tex_reads_as_unicode() {
        for (tex, want) in [
            (r"x^2 + y^2 = r^2", "x² + y² = r²"),
            (r"\frac{a+b}{2}", "(a+b)/2"),
            (r"\frac{1}{n}", "1/n"),
            (r"\sqrt{x+1} \le \sqrt[3]{y}", "√(x+1) ≤ ∛y"),
            (r"\sum_{i=1}^{n} i", "∑ᵢ₌₁ⁿ i"),
            (r"\alpha \to \infty", "α → ∞"),
            (r"x_{max}", "xₘₐₓ"),
            (r"x_{ab}", "x_ab"),
            (r"e^{i\pi} = -1", "e^iπ = −1"),
            (
                r"\begin{pmatrix} a & b \\ c & d \end{pmatrix}",
                "a, b; c, d",
            ),
            (r"\mathbb{R}^n", "ℝⁿ"),
            (r"\text{speed} = \frac{d}{t}", "speed = d/t"),
            (r"\hat{x}", "x\u{302}"),
            (r"\unknown{x}", r"\unknownx"),
        ] {
            assert_eq!(latex(tex), want, "{tex}");
        }
    }

    #[test]
    fn math_is_found_in_its_delimiters_and_nowhere_else() {
        let md = "Area $\\pi r^2$, then\n\n$$E = mc^2$$\n\nor \\(a \\ne b\\) and \\[x_1\\].";
        assert_eq!(
            unicode(md),
            "Area π r², then\n\nE = mc²\n\nor a ≠ b and x₁."
        );
        // Money, escaped dollars, code spans and fences stay as written.
        for md in [
            "It costs $5 and $10.",
            "Price: $ 5 $",
            r"Escaped \$x\$ stays.",
            "Code `$x^2$` stays.",
            "```\n$x^2$\n```\n",
            "~~~tex\n\\frac{a}{b}\n~~~\n",
        ] {
            assert_eq!(unicode(md), md, "{md}");
        }
        assert!(matches!(unicode("no math here"), Cow::Borrowed(_)));
        // Display math over several lines, and an unmatched backtick before math.
        assert_eq!(
            unicode("Energy:\n$$\nE = mc^2\n$$\nDone."),
            "Energy:\nE = mc²\nDone."
        );
        assert_eq!(unicode("a ` b $x^2$"), "a ` b x²");
        // Math between two fences is still found.
        assert_eq!(
            unicode("```\n$a$\n```\n$x^2$\n```\n$b$\n```\n"),
            "```\n$a$\n```\nx²\n```\n$b$\n```\n"
        );
        // A `$` does not open math across a blank line.
        assert_eq!(unicode("$a\n\nb$"), "$a\n\nb$");
    }

    #[test]
    fn written_out_math_is_not_read_as_markup() {
        assert_eq!(unicode("$a<b$"), r"a\<b");
        assert_eq!(unicode("$x_{ab}$"), r"x\_ab");
        assert_eq!(unicode("$|x|$"), r"\|x\|");
    }

    #[test]
    fn math_still_arriving_shows_as_written_until_it_closes() {
        assert_eq!(unicode("Area $\\pi r"), "Area $\\pi r");
        assert_eq!(unicode("$$E = mc"), "$$E = mc");
    }
}
