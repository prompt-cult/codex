/// The one error type the reader ever returns. A message only: the
/// caller (`tb-run`) prints it to standard error and exits non-zero.
/// See `AGENTS.md` — Error message standard — for the shape every
/// message here follows: name the constraint, the offending value, and
/// the expectation.
pub struct ParseError {
    pub message: String,
}

impl std::fmt::Display for ParseError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(&self.message)
    }
}

impl std::fmt::Debug for ParseError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(&self.message)
    }
}

/// Build a `ParseError` from the three-part shape most messages take:
/// what was checked, what it actually was, what it was expected to be.
pub fn fail(constraint: &str, actual: impl std::fmt::Display, expected: impl std::fmt::Display) -> ParseError {
    ParseError { message: format!("{constraint} is {actual}; expected {expected}") }
}

/// Build a `ParseError` from an already fully-formed message, for the
/// validator failure shapes that `IR.md` fixes verbatim and that do not
/// fit the `fail()` triple.
pub fn fail_msg(message: String) -> ParseError {
    ParseError { message }
}
