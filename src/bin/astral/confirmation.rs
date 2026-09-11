//! Shared bounded terminal confirmation. A pipe, EOF or partial answer is not approval.
use std::io::{self, BufRead, Read, Write};

pub(super) fn ask(
    input: &mut impl BufRead,
    output: &mut impl Write,
    prompt: &str,
) -> io::Result<bool> {
    write!(output, "{prompt} [y/N] ")?;
    output.flush()?;
    let mut answer = String::new();
    input.take(64).read_line(&mut answer)?;
    Ok(
        answer.ends_with('\n')
            && matches!(answer.trim().to_ascii_lowercase().as_str(), "y" | "yes"),
    )
}

#[cfg(test)]
mod tests {
    #[test]
    fn accepts_only_complete_bounded_affirmative_answers() {
        for (input, expected) in [
            ("y\n", true),
            (" YES \n", true),
            ("\n", false),
            ("no\n", false),
            ("yes", false),
            ("", false),
        ] {
            assert_eq!(
                super::ask(&mut input.as_bytes(), &mut Vec::new(), "Review?").unwrap(),
                expected
            );
        }
        assert!(
            !super::ask(
                &mut format!("yes{}\n", " ".repeat(80)).as_bytes(),
                &mut Vec::new(),
                "Review?"
            )
            .unwrap()
        );
    }
}
