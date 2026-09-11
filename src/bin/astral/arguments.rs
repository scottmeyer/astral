//! Presentation options for the custom project/init parser. The literal `--`
//! belongs to the child argument boundary and is never crossed here.
use std::ffi::OsString;

pub(super) fn presentation(args: &[OsString]) -> (bool, Vec<OsString>) {
    let mut json = false;
    let mut filtered = Vec::with_capacity(args.len());
    let mut protected_value = false;
    let mut child_arguments = false;
    for arg in args {
        if child_arguments || protected_value {
            filtered.push(arg.clone());
            protected_value = false;
        } else if arg == "--" {
            child_arguments = true;
            filtered.push(arg.clone());
        } else if arg == "--json" {
            json = true;
        } else {
            protected_value = ["--root", "--work", "--resume"]
                .iter()
                .any(|option| arg == option);
            filtered.push(arg.clone());
        }
    }
    (json, filtered)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn args(values: &[&str]) -> Vec<OsString> {
        values.iter().map(OsString::from).collect()
    }

    #[test]
    fn keeps_child_json_and_option_values_literal() {
        let input = args(&[
            "--json", "project", "web", "--root", "--json", "--", "--json", "prompt",
        ]);
        let (json, filtered) = presentation(&input);
        assert!(json);
        assert_eq!(filtered, input[1..]);
        let child_only = args(&["project", "web", "--", "--json"]);
        assert_eq!(presentation(&child_only), (false, child_only));
    }

    #[test]
    fn accepts_json_on_either_side_of_custom_subcommands() {
        for input in [
            args(&["--json", "project", "--inspect"]),
            args(&["project", "--inspect", "--json"]),
        ] {
            assert_eq!(
                presentation(&input),
                (true, args(&["project", "--inspect"]))
            );
        }
        assert_eq!(
            presentation(&args(&["init", "--json", "--inspect"])),
            (true, args(&["init", "--inspect"]))
        );
    }
}
