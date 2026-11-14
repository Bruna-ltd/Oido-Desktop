//! A condition said the way a person says it, "[result] [is / contains / …] [value]", and the
//! expression it stands for. Only the editor uses this; a condition that is not of this shape
//! stays an expression the person can edit as one. Pure.

use serde_json::Value;

/// How the result on the left is compared with what is on the right.
#[derive(Clone, Copy, PartialEq, Eq, Debug, Default)]
pub enum Op {
    #[default]
    Is,
    IsNot,
    Contains,
    StartsWith,
    IsEmpty,
    IsNotEmpty,
    MoreThan,
    LessThan,
}

/// The operators, in the order a list shows them.
pub const OPS: [Op; 8] = [
    Op::Is,
    Op::IsNot,
    Op::Contains,
    Op::StartsWith,
    Op::IsEmpty,
    Op::IsNotEmpty,
    Op::MoreThan,
    Op::LessThan,
];

impl Op {
    pub fn label(self) -> &'static str {
        match self {
            Op::Is => "is",
            Op::IsNot => "is not",
            Op::Contains => "contains",
            Op::StartsWith => "starts with",
            Op::IsEmpty => "is empty",
            Op::IsNotEmpty => "is not empty",
            Op::MoreThan => "is more than",
            Op::LessThan => "is less than",
        }
    }

    pub fn from_label(label: &str) -> Option<Op> {
        OPS.into_iter().find(|o| o.label() == label)
    }

    /// Whether there is a value on the right.
    pub fn takes_value(self) -> bool {
        !matches!(self, Op::IsEmpty | Op::IsNotEmpty)
    }
}

/// A number or true/false stays as it is; anything else is text.
fn literal(value: &str) -> String {
    let v = value.trim();
    if v.parse::<f64>().is_ok() || v == "true" || v == "false" {
        v.to_owned()
    } else {
        Value::String(value.to_owned()).to_string()
    }
}

/// What a literal says, if it is one this module writes.
fn unliteral(src: &str) -> Option<String> {
    let s = src.trim();
    if s.starts_with('"') {
        return serde_json::from_str::<String>(s).ok();
    }
    (s.parse::<f64>().is_ok() || s == "true" || s == "false").then(|| s.to_owned())
}

/// A name of a result, with fields: `files.stdout`, `inputs.kind`.
fn is_path(s: &str) -> bool {
    !s.is_empty()
        && !s.starts_with('.')
        && s.chars()
            .all(|c| c.is_alphanumeric() || c == '_' || c == '.')
}

/// The expression for `left op right`, or none while there is no result on the left. `trimmed()`
/// is on the left so text from a command, which ends in a new line, compares as it reads.
pub fn build(left: &str, op: Op, right: &str) -> Option<String> {
    let left = left.trim();
    if left.is_empty() {
        return None;
    }
    let l = format!("{left}.trimmed()");
    let lit = literal(right);
    Some(match op {
        Op::Is => format!("{l} == {lit}"),
        Op::IsNot => format!("{l} != {lit}"),
        Op::Contains => format!("{l}.contains({lit})"),
        Op::StartsWith => format!("{l}.starts_with({lit})"),
        Op::IsEmpty => format!("{l} == \"\""),
        Op::IsNotEmpty => format!("{l} != \"\""),
        Op::MoreThan => format!("{l} > {lit}"),
        Op::LessThan => format!("{l} < {lit}"),
    })
}

/// Reads an expression back into its parts, if it is one [`build`] writes.
pub fn parse(src: &str) -> Option<(String, Op, String)> {
    let src = src.trim();
    let (left, rest) = src.split_once(".trimmed()")?;
    if !is_path(left) {
        return None;
    }
    let rest = rest.trim();
    for (op, call) in [
        (Op::Contains, ".contains("),
        (Op::StartsWith, ".starts_with("),
    ] {
        if let Some(arg) = rest.strip_prefix(call).and_then(|r| r.strip_suffix(')')) {
            return Some((left.to_owned(), op, unliteral(arg)?));
        }
    }
    for (op, sign) in [
        (Op::Is, "=="),
        (Op::IsNot, "!="),
        (Op::MoreThan, ">"),
        (Op::LessThan, "<"),
    ] {
        if let Some(arg) = rest.strip_prefix(sign) {
            let arg = unliteral(arg)?;
            return Some(match (op, arg.as_str()) {
                (Op::Is, "") => (left.to_owned(), Op::IsEmpty, arg),
                (Op::IsNot, "") => (left.to_owned(), Op::IsNotEmpty, arg),
                _ => (left.to_owned(), op, arg),
            });
        }
    }
    None
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn every_operator_writes_an_expression_that_reads_back_the_same() {
        for op in OPS {
            let right = if op.takes_value() { "pdf" } else { "" };
            let src = build("files.stdout", op, right).unwrap();
            assert_eq!(
                parse(&src),
                Some(("files.stdout".to_owned(), op, right.to_owned())),
                "{src}"
            );
        }
    }

    #[test]
    fn numbers_and_truth_stay_bare_and_text_is_quoted_whatever_it_holds() {
        assert_eq!(
            build("job.exit_code", Op::Is, "0").unwrap(),
            "job.exit_code.trimmed() == 0"
        );
        assert_eq!(
            build("x", Op::Is, "say \"hi\"").unwrap(),
            r#"x.trimmed() == "say \"hi\"""#
        );
        assert_eq!(
            parse(r#"x.trimmed() == "say \"hi\"""#),
            Some(("x".into(), Op::Is, "say \"hi\"".into()))
        );
        assert_eq!(build("x", Op::Is, "true").unwrap(), "x.trimmed() == true");
    }

    #[test]
    fn nothing_is_written_without_a_result_and_what_is_not_this_shape_is_left_alone() {
        assert_eq!(build("  ", Op::Is, "a"), None);
        for src in [
            "files.stdout == \"a\"",
            "a.trimmed() == b",
            "a.len() > 2 && b",
            "(a).trimmed() == 1",
            "",
        ] {
            assert_eq!(parse(src), None, "{src:?}");
        }
    }
}
