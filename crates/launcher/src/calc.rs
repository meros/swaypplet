//! The `=` calculator: arithmetic evaluated here, in the frame the key was
//! typed in.
//!
//! Elephant has a `calc` provider, but it answers over the socket after the
//! query's debounce and round trip. Arithmetic does not need a daemon: a
//! recursive-descent parser over `f64` answers in microseconds, so the result
//! row is on screen before the next frame. What it cannot read (units,
//! currencies) still goes to elephant's provider.
//!
//! Grammar, lowest precedence first:
//!
//! ```text
//! expr   = term (('+' | '-') term)*
//! term   = unary (('*' | '/' | '%') unary)*
//! unary  = ('-' | '+') unary | power
//! power  = atom ('^' unary)?            right-associative, binds above unary minus on its left
//! atom   = number | constant | function '(' expr ')' | '(' expr ')'
//! ```
//!
//! Nothing here evaluates code: the input is read as numbers and a closed
//! set of operators, functions and constants, and anything else is `None`.

/// The value of `input`, or `None` when it is not arithmetic this reads.
pub fn eval(input: &str) -> Option<f64> {
    let tokens = lex(input)?;
    let mut p = Parser { tokens, pos: 0 };
    let v = p.expr()?;
    (p.pos == p.tokens.len() && v.is_finite()).then_some(v)
}

/// Whether `input` is worth offering a result for without the `=` prefix:
/// it evaluates, and it has an operator or a function in it, so a bare
/// number or a word that happens to be a constant ("e") does not turn every
/// query into a calculator row.
pub fn looks_like_arithmetic(input: &str) -> bool {
    let Some(tokens) = lex(input) else {
        return false;
    };
    let has_op = tokens.iter().any(|t| {
        matches!(t, Tok::Op(_) | Tok::Ident(_)) && !matches!(t, Tok::Ident(c) if is_constant(c))
    });
    has_op && tokens.len() > 1 && eval(input).is_some()
}

/// `v` as a person would write it: an integer without a decimal point, else
/// at most 12 significant digits with the trailing zeros dropped.
pub fn format(v: f64) -> String {
    if v == 0.0 {
        return "0".into();
    }
    if v.fract() == 0.0 && v.abs() < 1e15 {
        return format!("{}", v as i64);
    }
    let magnitude = v.abs().log10().floor() as i32;
    if !(-6..15).contains(&magnitude) {
        let s = format!("{v:.11e}");
        let (mantissa, exp) = s.split_once('e').unwrap_or((&s, "0"));
        let mantissa = mantissa.trim_end_matches('0').trim_end_matches('.');
        return format!("{mantissa}e{exp}");
    }
    let decimals = (11 - magnitude).clamp(0, 15) as usize;
    let s = format!("{v:.decimals$}");
    s.trim_end_matches('0').trim_end_matches('.').to_string()
}

#[derive(Debug, Clone, PartialEq)]
enum Tok {
    Num(f64),
    Op(char),
    LParen,
    RParen,
    Ident(String),
}

fn is_constant(name: &str) -> bool {
    matches!(name, "pi" | "π" | "e" | "tau" | "τ")
}

fn lex(input: &str) -> Option<Vec<Tok>> {
    let mut out = Vec::new();
    let chars: Vec<char> = input.chars().collect();
    let mut i = 0;
    while i < chars.len() {
        let c = chars[i];
        match c {
            ' ' | '\t' => i += 1,
            '0'..='9' | '.' | ',' => {
                // Hex: 0x1f.
                if c == '0' && matches!(chars.get(i + 1), Some('x' | 'X')) {
                    let start = i + 2;
                    let mut j = start;
                    while j < chars.len() && chars[j].is_ascii_hexdigit() {
                        j += 1;
                    }
                    let digits: String = chars[start..j].iter().collect();
                    out.push(Tok::Num(i64::from_str_radix(&digits, 16).ok()? as f64));
                    i = j;
                    continue;
                }
                let start = i;
                while i < chars.len()
                    && (chars[i].is_ascii_digit() || matches!(chars[i], '.' | ',' | '_'))
                {
                    i += 1;
                }
                // An exponent: 1e3, 2.5E-4. Only when a digit follows, so
                // "2e" is 2 then the constant e, which multiplies below.
                if i < chars.len()
                    && matches!(chars[i], 'e' | 'E')
                    && (chars.get(i + 1).is_some_and(char::is_ascii_digit)
                        || (matches!(chars.get(i + 1), Some('-' | '+'))
                            && chars.get(i + 2).is_some_and(char::is_ascii_digit)))
                {
                    i += 2;
                    while i < chars.len() && chars[i].is_ascii_digit() {
                        i += 1;
                    }
                }
                // A comma is the Swedish decimal separator; there is no
                // thousands separator to confuse it with (`1_000` groups).
                let text: String = chars[start..i]
                    .iter()
                    .filter(|c| **c != '_')
                    .map(|c| if *c == ',' { '.' } else { *c })
                    .collect();
                out.push(Tok::Num(text.parse().ok()?));
            }
            '+' | '-' | '*' | '/' | '%' | '^' => {
                out.push(Tok::Op(c));
                i += 1;
            }
            '×' | 'x'
                if !out.is_empty()
                    && !matches!(out.last(), Some(Tok::Op(_) | Tok::LParen))
                    && !chars.get(i + 1).is_some_and(|n| n.is_alphabetic()) =>
            {
                out.push(Tok::Op('*'));
                i += 1;
            }
            '·' => {
                out.push(Tok::Op('*'));
                i += 1;
            }
            '÷' => {
                out.push(Tok::Op('/'));
                i += 1;
            }
            '−' => {
                out.push(Tok::Op('-'));
                i += 1;
            }
            '(' => {
                out.push(Tok::LParen);
                i += 1;
            }
            ')' => {
                out.push(Tok::RParen);
                i += 1;
            }
            c if c.is_alphabetic() => {
                let start = i;
                while i < chars.len() && chars[i].is_alphanumeric() {
                    i += 1;
                }
                let word: String = chars[start..i].iter().collect::<String>().to_lowercase();
                out.push(Tok::Ident(word));
            }
            _ => return None,
        }
    }
    (!out.is_empty()).then_some(out)
}

struct Parser {
    tokens: Vec<Tok>,
    pos: usize,
}

impl Parser {
    fn peek(&self) -> Option<&Tok> {
        self.tokens.get(self.pos)
    }

    fn next(&mut self) -> Option<Tok> {
        let t = self.tokens.get(self.pos).cloned();
        self.pos += 1;
        t
    }

    fn expr(&mut self) -> Option<f64> {
        let mut v = self.term()?;
        while let Some(Tok::Op(op @ ('+' | '-'))) = self.peek().cloned() {
            self.pos += 1;
            let r = self.term()?;
            v = if op == '+' { v + r } else { v - r };
        }
        Some(v)
    }

    fn term(&mut self) -> Option<f64> {
        let mut v = self.unary()?;
        loop {
            match self.peek().cloned() {
                Some(Tok::Op(op @ ('*' | '/' | '%'))) => {
                    self.pos += 1;
                    let r = self.unary()?;
                    v = match op {
                        '*' => v * r,
                        '/' => v / r,
                        _ => v.rem_euclid(r),
                    };
                }
                // Juxtaposition multiplies: 2pi, 3(4+1), (1+1)(2+2). Not two
                // numbers: "1 000" is a typo, not a thousand and not zero.
                Some(Tok::LParen | Tok::Ident(_)) => {
                    let r = self.unary()?;
                    v *= r;
                }
                _ => return Some(v),
            }
        }
    }

    fn unary(&mut self) -> Option<f64> {
        match self.peek() {
            Some(Tok::Op('-')) => {
                self.pos += 1;
                Some(-self.unary()?)
            }
            Some(Tok::Op('+')) => {
                self.pos += 1;
                self.unary()
            }
            _ => self.power(),
        }
    }

    fn power(&mut self) -> Option<f64> {
        let base = self.atom()?;
        if let Some(Tok::Op('^')) = self.peek() {
            self.pos += 1;
            let exp = self.unary()?;
            return Some(base.powf(exp));
        }
        Some(base)
    }

    fn atom(&mut self) -> Option<f64> {
        match self.next()? {
            Tok::Num(n) => Some(n),
            Tok::LParen => {
                let v = self.expr()?;
                (self.next()? == Tok::RParen).then_some(v)
            }
            Tok::Ident(name) => {
                if let Some(c) = constant(&name) {
                    return Some(c);
                }
                let f = function(&name)?;
                // A function takes a parenthesised argument, or the atom
                // right after it: sqrt(2), sqrt 2.
                let arg = if self.peek() == Some(&Tok::LParen) {
                    self.atom()?
                } else {
                    self.power()?
                };
                Some(f(arg))
            }
            _ => None,
        }
    }
}

fn constant(name: &str) -> Option<f64> {
    Some(match name {
        "pi" | "π" => std::f64::consts::PI,
        "e" => std::f64::consts::E,
        "tau" | "τ" => std::f64::consts::TAU,
        _ => return None,
    })
}

fn function(name: &str) -> Option<fn(f64) -> f64> {
    Some(match name {
        "sqrt" => f64::sqrt,
        "cbrt" => f64::cbrt,
        "abs" => f64::abs,
        "floor" => f64::floor,
        "ceil" => f64::ceil,
        "round" => f64::round,
        "exp" => f64::exp,
        "ln" => f64::ln,
        "log" | "lg" => f64::log10,
        "log2" => f64::log2,
        "sin" => f64::sin,
        "cos" => f64::cos,
        "tan" => f64::tan,
        "asin" => f64::asin,
        "acos" => f64::acos,
        "atan" => f64::atan,
        _ => return None,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn close(input: &str, want: f64) {
        let got = eval(input).unwrap_or_else(|| panic!("{input} did not evaluate"));
        assert!(
            (got - want).abs() < 1e-9 * want.abs().max(1.0),
            "{input} = {got}, want {want}"
        );
    }

    #[test]
    fn precedence_and_associativity() {
        close("1 + 2 * 3", 7.0);
        close("(1 + 2) * 3", 9.0);
        close("2 ^ 3 ^ 2", 512.0);
        close("-2 ^ 2", -4.0);
        close("2 ^ -1", 0.5);
        close("10 - 4 - 3", 3.0);
        close("100 / 10 / 5", 2.0);
        close("7 % 3", 1.0);
        close("-7 % 3", 2.0);
    }

    #[test]
    fn numbers_as_people_type_them() {
        close("1,5 * 2", 3.0);
        close("1_000 + 1", 1001.0);
        close("1e3 / 4", 250.0);
        close("2.5E-1", 0.25);
        close("0xff + 1", 256.0);
        close("3 × 4", 12.0);
        close("12 ÷ 4", 3.0);
        close("5 − 2", 3.0);
    }

    #[test]
    fn functions_constants_and_juxtaposition() {
        close("sqrt(16)", 4.0);
        close("sqrt 16 + 1", 5.0);
        close("2pi", std::f64::consts::TAU);
        close("3(4 + 1)", 15.0);
        close("(1 + 1)(2 + 2)", 8.0);
        close("ln(e)", 1.0);
        close("log 1000", 3.0);
        close("cos(0)", 1.0);
    }

    #[test]
    fn what_is_not_arithmetic_is_none() {
        for bad in [
            "1 000", "", "firefox", "1 +", "(1 + 2", "1 + 2)", "rm -rf", "sqrt", "1 / 0", "2 $ 3",
            "x",
        ] {
            assert_eq!(eval(bad), None, "{bad}");
        }
    }

    #[test]
    fn only_expressions_are_offered_without_the_prefix() {
        assert!(looks_like_arithmetic("2+2"));
        assert!(looks_like_arithmetic("sqrt 2"));
        assert!(!looks_like_arithmetic("42"));
        assert!(!looks_like_arithmetic("e"));
        assert!(!looks_like_arithmetic("pi"));
        assert!(!looks_like_arithmetic("code"));
        assert!(!looks_like_arithmetic("vlc"));
    }

    #[test]
    fn results_read_like_numbers() {
        assert_eq!(format(4.0), "4");
        assert_eq!(format(-12.0), "-12");
        assert_eq!(format(0.1 + 0.2), "0.3");
        assert_eq!(format(1.0 / 3.0), "0.333333333333");
        assert_eq!(format(2f64.sqrt()), "1.41421356237");
        assert_eq!(format(1e20), "1e20");
        assert_eq!(format(1.5e-9), "1.5e-9");
    }
}
