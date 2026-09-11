//! Presentation options for the custom project/init parser. The literal `--`
//! belongs to the child argument boundary and is never crossed here.
use std::ffi::OsString;

#[derive(Debug, Default, PartialEq, Eq)]
pub(super) struct Presentation {
    pub json: bool,
    pub plain: bool,
    pub pick: bool,
}

pub(super) fn presentation(args: &[OsString]) -> (Presentation, Vec<OsString>) {
    let mut options = Presentation::default();
    let mut filtered = Vec::with_capacity(args.len());
    let mut protected_value = false;
    let mut child_arguments = false;
    let mut command = None;
    for arg in args {
        if child_arguments || protected_value {
            filtered.push(arg.clone());
            protected_value = false;
        } else if arg == "--" {
            child_arguments = true;
            filtered.push(arg.clone());
        } else if arg == "--json" {
            options.json = true;
        } else if arg == "--plain" {
            options.plain = true;
        } else if arg == "--pick" && command == Some("project") {
            options.pick = true;
        } else {
            if command.is_none() && !arg.as_encoded_bytes().starts_with(b"-") {
                command = Some(if arg == "project" { "project" } else { "other" });
            }
            protected_value = ["--root", "--work", "--resume"]
                .iter()
                .any(|option| arg == option);
            filtered.push(arg.clone());
        }
    }
    (options, filtered)
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
        let (options, filtered) = presentation(&input);
        assert!(options.json);
        assert_eq!(filtered, input[1..]);
        let child_only = args(&["project", "web", "--", "--json"]);
        assert_eq!(
            presentation(&child_only),
            (Presentation::default(), child_only)
        );
    }

    #[test]
    fn accepts_json_on_either_side_of_custom_subcommands() {
        for input in [
            args(&["--json", "project", "--inspect"]),
            args(&["project", "--inspect", "--json"]),
        ] {
            assert_eq!(
                presentation(&input),
                (
                    Presentation {
                        json: true,
                        ..Presentation::default()
                    },
                    args(&["project", "--inspect"])
                )
            );
        }
        assert_eq!(
            presentation(&args(&["init", "--json", "--inspect"])),
            (
                Presentation {
                    json: true,
                    ..Presentation::default()
                },
                args(&["init", "--inspect"])
            )
        );
    }

    #[test]
    fn picker_and_plain_preserve_values_and_child_arguments() {
        let (options, filtered) = presentation(&args(&[
            "--root", "--pick", "project", "web", "--pick", "--work", "--plain", "--plain", "--",
            "--pick", "--plain", "--json", "a prompt",
        ]));
        assert_eq!(
            options,
            Presentation {
                json: false,
                plain: true,
                pick: true
            }
        );
        assert_eq!(
            filtered,
            args(&[
                "--root", "--pick", "project", "web", "--work", "--plain", "--", "--pick",
                "--plain", "--json", "a prompt",
            ])
        );
        assert!(!presentation(&args(&["init", "--pick"])).0.pick);
        assert!(!presentation(&args(&["context", "list", "--pick"])).0.pick);
    }
}
