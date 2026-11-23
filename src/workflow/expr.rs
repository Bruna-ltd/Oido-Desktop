//! Expressions in a workflow: the conditions of `if`, `switch` and `while`, the list a `for_each`
//! goes over, the values of `set`, and `{{= … }}` in text. One small script engine (Rhai) for all of
//! them. It has no way to reach a file, the network or another program: anything like that is a
//! step, which the guardrails see. Every evaluation is bounded in operations, nesting, size and
//! time, and ends at once when the run is stopped.

use std::collections::HashMap;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, LazyLock, Mutex};
use std::time::{Duration, Instant};

use rhai::{Dynamic, Engine, EvalAltResult, ParseError};
use serde_json::Value;

use super::template::Scope;

/// Operations one evaluation may take.
const MAX_OPERATIONS: u64 = 1_000_000;
/// How long one evaluation may run.
const MAX_TIME: Duration = Duration::from_secs(10);
const MAX_CALL_LEVELS: usize = 32;
const MAX_EXPR_DEPTH: usize = 64;
const MAX_TEXT: usize = 1_000_000;
const MAX_ARRAY: usize = 100_000;
const MAX_MAP: usize = 10_000;
/// How big a compiled regular expression may be, in bytes.
const MAX_PATTERN: usize = 100_000;
/// The clock is looked at this often, in operations.
const CLOCK_EVERY: u64 = 1024;

/// How many compiled patterns `matches` keeps, so a loop does not compile its pattern each round.
const KEPT_PATTERNS: usize = 32;

const STOPPED: &str = "stopped";
const TOO_LONG: &str = "too long";

fn base() -> Engine {
    let mut engine = Engine::new();
    engine.set_max_operations(MAX_OPERATIONS);
    engine.set_max_call_levels(MAX_CALL_LEVELS);
    engine.set_max_expr_depths(MAX_EXPR_DEPTH, MAX_EXPR_DEPTH / 2);
    engine.set_max_string_size(MAX_TEXT);
    engine.set_max_array_size(MAX_ARRAY);
    engine.set_max_map_size(MAX_MAP);
    // A field that is not there is a mistake to report, not `()` to carry on with.
    engine.set_fail_on_invalid_map_property(true);
    engine.set_strict_variables(true);
    engine.disable_symbol("eval");
    engine.on_print(|_| {});
    engine.on_debug(|_, _, _| {});
    // Rhai's own `trim` cuts the string it is called on and gives back nothing, so
    // `out.stdout.trim()` is empty; this gives the trimmed text and leaves the value alone. What is
    // not text comes back as it is, so it can be put on any result.
    engine.register_fn("trimmed", |v: Dynamic| match v.clone().into_string() {
        Ok(text) => Dynamic::from(text.trim().to_owned()),
        Err(_) => v,
    });
    // Whether text fits a regular expression, which is how "matches the pattern" is written. The
    // pattern is bounded in size, and one that is not a pattern is an error to report.
    engine.register_fn(
        "matches",
        |text: rhai::ImmutableString,
         pattern: rhai::ImmutableString|
         -> Result<bool, Box<EvalAltResult>> {
            pattern_matches(&pattern, &text)
                .map_err(|e| format!("{pattern:?} is not a pattern: {e}").into())
        },
    );
    engine
}

/// Whether `text` fits `pattern`, compiling each pattern once.
fn pattern_matches(pattern: &str, text: &str) -> Result<bool, regex::Error> {
    static KEPT: LazyLock<Mutex<HashMap<String, regex::Regex>>> = LazyLock::new(Mutex::default);
    let known = KEPT.lock().ok().and_then(|kept| kept.get(pattern).cloned());
    let re = match known {
        Some(re) => re,
        None => {
            let re = regex::RegexBuilder::new(pattern)
                .size_limit(MAX_PATTERN)
                .build()?;
            if let Ok(mut kept) = KEPT.lock() {
                if kept.len() >= KEPT_PATTERNS {
                    kept.clear();
                }
                kept.insert(pattern.to_owned(), re.clone());
            }
            re
        }
    };
    Ok(re.is_match(text))
}

/// The engine that checks expressions without running them; it is the same for every check.
static CHECKER: LazyLock<Engine> = LazyLock::new(base);

/// What a run evaluates expressions with. One per run: it knows when the run is stopped.
pub struct Evaluator {
    engine: Engine,
    limit: Duration,
    deadline: Arc<Mutex<Instant>>,
}

impl Evaluator {
    pub fn new(cancel: &Arc<AtomicBool>) -> Evaluator {
        Self::with_limit(cancel, MAX_TIME)
    }

    fn with_limit(cancel: &Arc<AtomicBool>, limit: Duration) -> Evaluator {
        let deadline = Arc::new(Mutex::new(Instant::now() + limit));
        let mut engine = base();
        let stop = Arc::clone(cancel);
        let until = Arc::clone(&deadline);
        engine.on_progress(move |ops| {
            if stop.load(Ordering::Relaxed) {
                return Some(STOPPED.into());
            }
            if ops.is_multiple_of(CLOCK_EVERY) && until.lock().is_ok_and(|d| Instant::now() > *d) {
                return Some(TOO_LONG.into());
            }
            None
        });
        Evaluator {
            engine,
            limit,
            deadline,
        }
    }

    /// The value of the expression `src` over `scope`, as JSON.
    pub fn eval(&self, scope: &Scope, src: &str) -> Result<Value, String> {
        self.go(scope, src, |engine, vars| {
            engine.eval_expression_with_scope(vars, src)
        })
    }

    /// What the script `src` gives (its last value) over `scope`, as JSON. Statements, loops and
    /// functions are allowed; reaching outside the values is not.
    pub fn run(&self, scope: &Scope, src: &str) -> Result<Value, String> {
        self.go(scope, src, |engine, vars| {
            engine.eval_with_scope::<Dynamic>(vars, src)
        })
    }

    fn go(
        &self,
        scope: &Scope,
        src: &str,
        work: impl FnOnce(&Engine, &mut rhai::Scope) -> Result<Dynamic, Box<EvalAltResult>>,
    ) -> Result<Value, String> {
        if let Ok(mut d) = self.deadline.lock() {
            *d = Instant::now() + self.limit;
        }
        // Only what `src` names is made into Rhai values: a value it does not name cannot be
        // reached (there is no `eval`, and variables are strict), and a big earlier result would
        // otherwise be converted again for every condition in a loop.
        let named = names_in(src);
        let mut vars = rhai::Scope::new();
        for (name, value) in scope.iter().filter(|(n, _)| named.contains(n.as_str())) {
            let dynamic = rhai::serde::to_dynamic(value).map_err(|e| e.to_string())?;
            vars.push_dynamic(name.as_str(), dynamic);
        }
        let result = work(&self.engine, &mut vars).map_err(|e| with_hint(src, failure(&e)))?;
        rhai::serde::from_dynamic(&result)
            .map_err(|_| format!("`{}` gave something a workflow cannot keep", one_line(src)))
    }

    /// A condition: the answer must be true or false. A number or text is not guessed at.
    pub fn eval_bool(&self, scope: &Scope, src: &str) -> Result<bool, String> {
        match self.eval(scope, src)? {
            Value::Bool(b) => Ok(b),
            other => Err(format!(
                "`{}` gave {}, not true or false",
                one_line(src),
                super::template::as_text(&other)
            )),
        }
    }
}

/// Every word in `src` that could be a variable's name.
fn names_in(src: &str) -> std::collections::HashSet<&str> {
    src.split(|c: char| !(c.is_alphanumeric() || c == '_'))
        .filter(|w| !w.is_empty())
        .collect()
}

fn one_line(src: &str) -> String {
    let line = src.lines().next().unwrap_or_default().trim();
    let (cut, more) = crate::runs::clip(line, 60);
    if more || src.lines().count() > 1 {
        format!("{cut}…")
    } else {
        cut
    }
}

fn failure(e: &EvalAltResult) -> String {
    match e {
        EvalAltResult::ErrorTerminated(why, _) if why.to_string() == STOPPED => {
            "Stopped.".to_owned()
        }
        EvalAltResult::ErrorTerminated(..) | EvalAltResult::ErrorTooManyOperations(_) => {
            "The expression ran too long.".to_owned()
        }
        other => other.to_string(),
    }
}

/// `why` a script or expression failed, with what to write instead when `src` has a habit from
/// JavaScript, which is what models reach for first.
fn with_hint(src: &str, why: String) -> String {
    let mut hints: Vec<String> = Vec::new();
    if let Some(name) = why
        .split("Undefined variable: ")
        .nth(1)
        .and_then(|rest| rest.split([' ', '\n']).next())
        .filter(|n| !n.is_empty() && !src.contains(&format!("let {n}")))
        && src.contains(&format!("{name} = "))
    {
        hints.push(format!("declare it first: `let {name} = …`"));
    }
    let habits = [
        (
            "=>",
            "a function is written `|x| x.name == \"a\"`, not `x => …`",
        ),
        ("?.", "there is no `?.`; test with `if x != () { … }`"),
        (".length", "the size is `.len()`, not `.length`"),
        ("===", "equal is `==`"),
        ("null", "nothing is `()`, not `null`"),
    ];
    hints.extend(
        habits
            .iter()
            .filter(|(habit, _)| src.contains(habit))
            .map(|(_, say)| (*say).to_owned()),
    );
    if hints.is_empty() {
        why
    } else {
        format!("{why} (Rhai, not JavaScript: {})", hints.join("; "))
    }
}

/// Whether `src` reads as a script and names only `visible` values (and what it declares).
/// Nothing is run.
pub fn check_script(visible: &[&str], src: &str) -> Result<(), String> {
    if src.trim().is_empty() {
        return Err("it is empty".to_owned());
    }
    let engine = &*CHECKER;
    let mut vars = rhai::Scope::new();
    for name in visible {
        vars.push_dynamic(*name, Dynamic::UNIT);
    }
    engine
        .compile_with_scope(&vars, src)
        .map(|_| ())
        .map_err(|e: ParseError| with_hint(src, e.to_string()))
}

/// Whether `src` reads as an expression and names only `visible` values. Nothing is run.
pub fn check(visible: &[&str], src: &str) -> Result<(), String> {
    if src.trim().is_empty() {
        return Err("it is empty".to_owned());
    }
    let engine = &*CHECKER;
    let mut vars = rhai::Scope::new();
    for name in visible {
        vars.push_dynamic(*name, Dynamic::UNIT);
    }
    engine
        .compile_expression_with_scope(&vars, src)
        .map(|_| ())
        .map_err(|e: ParseError| with_hint(src, e.to_string()))
}

#[cfg(test)]
#[allow(clippy::unwrap_used)]
mod tests {
    use serde_json::json;

    use super::*;

    fn scope() -> Scope {
        crate::workflow::template::scope_of([
            (
                "inputs".to_owned(),
                json!({"n": 3, "who": "ann", "tags": ["a", "b"]}),
            ),
            (
                "files".to_owned(),
                json!({"exit_code": 0, "stdout": "a.txt\nb.txt"}),
            ),
        ])
    }

    fn ev() -> Evaluator {
        Evaluator::new(&Arc::new(AtomicBool::new(false)))
    }

    #[test]
    fn only_the_values_named_are_handed_over_and_a_missing_one_is_still_an_error() {
        let e = ev();
        let s = scope();
        assert_eq!(e.eval(&s, "`${files.exit_code}!`").unwrap(), json!("0!"));
        assert_eq!(
            e.run(&s, "let n = 0; for t in inputs.tags { n += 1 } n")
                .unwrap(),
            json!(2)
        );
        let why = e.eval(&s, "nothing_here + 1").unwrap_err();
        assert!(why.contains("nothing_here"), "{why}");
    }

    #[test]
    fn values_go_in_and_out_as_json_with_their_types() {
        let e = ev();
        let s = scope();
        assert_eq!(e.eval(&s, "inputs.n + 1").unwrap(), json!(4));
        assert_eq!(e.eval(&s, "inputs.n * 1.5").unwrap(), json!(4.5));
        assert_eq!(e.eval(&s, "inputs.who + \"!\"").unwrap(), json!("ann!"));
        assert_eq!(e.eval(&s, "inputs.tags.len()").unwrap(), json!(2));
        assert_eq!(
            e.eval(&s, "files.stdout.split(\"\\n\")").unwrap(),
            json!(["a.txt", "b.txt"])
        );
        assert_eq!(
            e.eval(&s, "#{a: 1, b: [2]}").unwrap(),
            json!({"a": 1, "b": [2]})
        );
        assert_eq!(e.eval(&s, "()").unwrap(), Value::Null);
        assert_eq!(
            e.eval(&s, "files.exit_code == 0 && inputs.n > 2").unwrap(),
            json!(true)
        );
    }

    #[test]
    fn a_condition_must_be_true_or_false_and_a_missing_field_is_an_error() {
        let e = ev();
        let s = scope();
        assert!(e.eval_bool(&s, "inputs.n > 2").unwrap());
        let why = e.eval_bool(&s, "inputs.n").unwrap_err();
        assert!(why.contains("not true or false"), "{why}");
        let why = e.eval(&s, "inputs.nothing").unwrap_err();
        assert!(why.to_lowercase().contains("nothing"), "{why}");
        let why = e.eval(&s, "ghost + 1").unwrap_err();
        assert!(why.contains("ghost"), "{why}");
    }

    #[test]
    fn nothing_outside_the_values_can_be_reached() {
        let e = ev();
        let s = scope();
        assert!(e.eval(&s, "eval(\"1 + 1\")").is_err());
        assert!(e.eval(&s, "import \"x\" as y; 1").is_err());
        assert!(
            e.eval(&s, "print(\"hi\")").is_ok(),
            "print is silent, not fatal"
        );
    }

    #[test]
    fn a_runaway_expression_is_stopped_by_its_size_cap() {
        let e = ev();
        let s = scope();
        let why = e
            .eval(&s, "let x = 0; while true { x += 1; } x")
            .unwrap_err();
        assert!(!why.is_empty());
        // Statements are not expressions; the cap is for what an expression can still do.
        let big = e.eval(&s, "\"ab\".pad(2000000, \"x\")").unwrap_err();
        assert!(
            big.to_lowercase().contains("size") || big.contains("too"),
            "{big}"
        );
    }

    #[test]
    fn stopping_the_run_ends_an_evaluation_in_progress() {
        let cancel = Arc::new(AtomicBool::new(true));
        let e = Evaluator::new(&cancel);
        let why = e.eval(&scope(), "inputs.n + 1").unwrap_err();
        assert_eq!(why, "Stopped.");
    }

    #[test]
    fn the_clock_ends_a_long_evaluation() {
        let e = Evaluator::with_limit(&Arc::new(AtomicBool::new(false)), Duration::ZERO);
        let long = format!("[{}].len()", vec!["1"; 5000].join(","));
        let why = e.eval(&scope(), &long).unwrap_err();
        assert!(why.contains("too long"), "{why}");
    }

    #[test]
    fn a_failure_says_what_to_write_instead_of_a_habit_from_javascript() {
        let told = check_script(&[], "items = []\nitems.push(1)\nitems.length").unwrap_err();
        assert!(
            told.contains("let items = …") && told.contains(".len()"),
            "{told}"
        );
        let told = check_script(&["xs"], "xs.find(h => h.a == 1)").unwrap_err();
        assert!(told.contains("|x|"), "{told}");
        // Nothing is added when no habit shows.
        let told = check_script(&[], "1 +").unwrap_err();
        assert!(!told.contains("Rhai, not"), "{told}");
    }

    #[test]
    fn a_check_reads_the_expression_and_the_names_it_uses_without_running_it() {
        assert_eq!(check(&["inputs", "files"], "files.exit_code == 0"), Ok(()));
        assert!(
            check(&["inputs"], "files.exit_code == 0")
                .unwrap_err()
                .contains("files")
        );
        assert!(check(&["inputs"], "inputs.n +").is_err());
        assert!(check(&["inputs"], "  ").unwrap_err().contains("empty"));
        assert!(
            check(&["inputs"], "let x = 1; x").is_err(),
            "a statement is not an expression"
        );
    }

    #[test]
    fn a_script_may_use_statements_loops_and_functions_but_still_only_the_values() {
        let e = ev();
        let s = scope();
        let script = "let total = 0; for t in inputs.tags { total += t.len(); } fn twice(x) { x * 2 } twice(total) + inputs.n";
        assert_eq!(e.run(&s, script).unwrap(), json!(7));
        assert_eq!(
            e.run(
                &s,
                "#{names: files.stdout.split(\"\\n\"), ok: files.exit_code == 0}"
            )
            .unwrap(),
            json!({"names": ["a.txt", "b.txt"], "ok": true})
        );
        assert!(e.run(&s, "import \"x\" as y; 1").is_err());
        assert!(e.run(&s, "ghost + 1").unwrap_err().contains("ghost"));
        let why = e.run(&s, "let x = 0; loop { x += 1; }").unwrap_err();
        assert!(why.contains("too long"), "{why}");
        assert_eq!(check_script(&["inputs"], "let a = inputs.n; a + 1"), Ok(()));
        assert!(check_script(&["inputs"], "let a = nope; a").is_err());
        assert!(check_script(&["inputs"], "let a = ").is_err());
        assert!(check_script(&["inputs"], "").unwrap_err().contains("empty"));
    }

    #[test]
    fn trimmed_gives_the_text_without_its_ends_where_rhais_own_trim_gives_nothing() {
        let e = ev();
        let s = crate::workflow::template::scope_of([(
            "out".to_owned(),
            json!({"stdout": "  hi there\n"}),
        )]);
        assert_eq!(
            e.eval(&s, "out.stdout.trimmed()").unwrap(),
            json!("hi there")
        );
        assert_eq!(
            e.eval(&s, "out.stdout.trimmed().split(\" \")").unwrap(),
            json!(["hi", "there"])
        );
        assert_eq!(
            e.eval(&s, "out.stdout").unwrap(),
            json!("  hi there\n"),
            "the value is untouched"
        );
        assert_eq!(
            e.eval(&s, "out.stdout.trim()").unwrap(),
            Value::Null,
            "which is why `trimmed` is here"
        );
        assert_eq!(
            e.run(&s, "let t = out.stdout.trimmed(); t.len()").unwrap(),
            json!(8)
        );
        let n = crate::workflow::template::scope_of([("n".to_owned(), json!(3))]);
        assert_eq!(e.eval(&n, "n.trimmed() == 3").unwrap(), json!(true));
        assert_eq!(
            e.eval(&s, "out.stdout.trimmed().contains(\"hi\")").unwrap(),
            json!(true)
        );
    }

    #[test]
    fn matches_tests_text_against_a_pattern_and_a_bad_pattern_is_an_error() {
        let e = ev();
        let s = crate::workflow::template::scope_of([("n".to_owned(), json!("order-1042"))]);
        assert_eq!(
            e.eval(&s, r#"n.matches("^order-\\d+$")"#).unwrap(),
            json!(true)
        );
        assert_eq!(e.eval(&s, r#"n.matches("^x")"#).unwrap(), json!(false));
        assert!(
            e.eval(&s, r#"n.matches("(")"#)
                .unwrap_err()
                .contains("not a pattern")
        );
    }
}
