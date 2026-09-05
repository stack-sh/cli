//! Deterministic shell-completion and manual-page generation.

use std::fmt::Write as _;

use super::{
    CHECK_HELP, COMPLETIONS_HELP, CONFIG_GET_HELP, CONFIG_HELP, CONFIG_PATH_HELP, DOCTOR_HELP,
    FORMAT_HELP, GENERAL_HELP, HELP_HELP, ICONS_HELP, ICONS_IMPORT_HELP, ICONS_LIST_HELP,
    INIT_HELP, LSP_HELP, MANPAGE_HELP, RENDER_HELP, UPDATE_HELP, VERSION_HELP,
};

pub(crate) const TOP_LEVEL_NAMES: &[&str] = &[
    "init",
    "check",
    "fmt",
    "render",
    "update",
    "lsp",
    "doctor",
    "config",
    "icons",
    "completions",
    "manpage",
    "help",
    "version",
];

const GLOBAL_OPTIONS: &[&str] = &["-h", "--help", "-v", "-V", "--version"];
const PROVIDERS: &[&str] = &["aws", "gcp", "azure", "simple-icons"];
const SHELLS: &[&str] = &["bash", "zsh", "fish"];
const TEMPLATES: &[&str] = &[
    "hello-stack",
    "application-and-data",
    "groups-and-layout",
    "commerce-platform",
    "aws-serverless-checkout",
    "gcp-data-service",
    "azure-event-platform",
    "github-delivery-workflow",
    "mixed-provider-platform",
];

#[derive(Clone, Copy)]
struct CommandSpec {
    context: &'static str,
    description: &'static str,
    options: &'static [&'static str],
    values: &'static [&'static str],
}

const COMMANDS: &[CommandSpec] = &[
    CommandSpec {
        context: "init",
        description: "Create a Stack file from a versioned starter template",
        options: &["--template", "-o", "--output", "--force", "-h", "--help"],
        values: &[],
    },
    CommandSpec {
        context: "check",
        description: "Validate a Stack source file without modifying it",
        options: &["--json", "-h", "--help"],
        values: &[],
    },
    CommandSpec {
        context: "fmt",
        description: "Format a file in place or read from standard input",
        options: &["--check", "--json", "-h", "--help"],
        values: &["-"],
    },
    CommandSpec {
        context: "render",
        description: "Render standalone SVG to standard output or a file",
        options: &[
            "--provider-pack",
            "-o",
            "--notice",
            "--json",
            "-h",
            "--help",
        ],
        values: &[],
    },
    CommandSpec {
        context: "update",
        description: "Check for or install a verified direct-install update",
        options: &["--check", "--version", "-h", "--help"],
        values: &[],
    },
    CommandSpec {
        context: "lsp",
        description: "Run the Stack language server over standard input and output",
        options: &["-h", "--help"],
        values: &[],
    },
    CommandSpec {
        context: "doctor",
        description: "Diagnose CLI configuration and provider icon packs",
        options: &["--provider-pack", "-h", "--help"],
        values: &[],
    },
    CommandSpec {
        context: "config",
        description: "Inspect effective read-only configuration",
        options: &["-h", "--help"],
        values: &["path", "get", "help"],
    },
    CommandSpec {
        context: "config path",
        description: "Print the resolved config.yaml path",
        options: &["-h", "--help"],
        values: &[],
    },
    CommandSpec {
        context: "config get",
        description: "Print one effective configuration value",
        options: &["-h", "--help"],
        values: &["default_icons_path"],
    },
    CommandSpec {
        context: "icons",
        description: "List catalogs and import audited provider icon archives",
        options: &["-h", "--help"],
        values: &["list", "import", "help"],
    },
    CommandSpec {
        context: "icons list",
        description: "List searchable asset-free provider catalog metadata",
        options: &["-h", "--help"],
        values: PROVIDERS,
    },
    CommandSpec {
        context: "icons import",
        description: "Download and import an audited provider icon archive",
        options: &["--accept-terms", "-o", "-h", "--help"],
        values: PROVIDERS,
    },
    CommandSpec {
        context: "completions",
        description: "Generate bash, zsh, or fish completion source",
        options: &["-h", "--help"],
        values: SHELLS,
    },
    CommandSpec {
        context: "manpage",
        description: "Print the offline Stack CLI manual page",
        options: &["-h", "--help"],
        values: &[],
    },
    CommandSpec {
        context: "help",
        description: "Print this message or the help of a subcommand",
        options: &["-h", "--help"],
        values: TOP_LEVEL_NAMES,
    },
    CommandSpec {
        context: "version",
        description: "Print version information",
        options: &["-h", "--help"],
        values: &[],
    },
];

pub(crate) fn completion(shell: &str) -> Result<String, String> {
    match shell {
        "bash" => Ok(bash_completion()),
        "zsh" => Ok(zsh_completion()),
        "fish" => Ok(fish_completion()),
        _ => Err(format!(
            "unsupported shell '{shell}'; supported shells: {}",
            SHELLS.join(", ")
        )),
    }
}

fn words(values: &[&str]) -> String {
    values.join(" ")
}

fn bash_completion() -> String {
    let mut output = String::from(
        "# Generated by `stack completions bash`; do not edit.\n\
_stack_completion() {\n\
  local current previous context words\n\
  current=\"${COMP_WORDS[COMP_CWORD]-}\"\n\
  previous=\"${COMP_WORDS[COMP_CWORD-1]-}\"\n\
  context=\"${COMP_WORDS[1]-}\"\n\
  if [[ ( \"$context\" == icons || \"$context\" == config ) && $COMP_CWORD -ge 3 ]]; then\n\
    context=\"$context ${COMP_WORDS[2]-}\"\n\
  fi\n\
  case \"$previous\" in\n\
    --template) words=\"",
    );
    output.push_str(&words(TEMPLATES));
    output.push_str(
        "\" ;;\n\
    --provider-pack|-o|--output|--notice) compopt -o default 2>/dev/null || true; return ;;\n\
    --version) COMPREPLY=(); return ;;\n\
    *) words=\"\" ;;\n\
  esac\n\
  if [[ -z \"$words\" ]]; then\n\
    if (( COMP_CWORD == 1 )); then\n\
      words=\"",
    );
    let top_level = TOP_LEVEL_NAMES
        .iter()
        .chain(GLOBAL_OPTIONS.iter())
        .copied()
        .collect::<Vec<_>>();
    output.push_str(&words(&top_level));
    output.push_str("\"\n    else\n      case \"$context\" in\n");
    for command in COMMANDS {
        let candidates = command
            .values
            .iter()
            .chain(command.options.iter())
            .copied()
            .collect::<Vec<_>>();
        let _ = writeln!(
            output,
            "        \"{}\") words=\"{}\" ;;",
            command.context,
            words(&candidates)
        );
    }
    output.push_str(
        "        *) words=\"\" ;;\n\
      esac\n\
    fi\n\
  fi\n\
  COMPREPLY=( $(compgen -W \"$words\" -- \"$current\") )\n\
  if (( ${#COMPREPLY[@]} == 0 )); then\n\
    compopt -o default 2>/dev/null || true\n\
  fi\n\
}\n\
complete -F _stack_completion stack\n",
    );
    output
}

fn zsh_completion() -> String {
    let mut output = String::from(
        "#compdef stack\n# Generated by `stack completions zsh`; do not edit.\n\
_stack() {\n\
  local context=\"${words[2]-}\"\n\
  if [[ ( \"$context\" == icons || \"$context\" == config ) && $CURRENT -ge 4 ]]; then\n\
    context=\"$context ${words[3]-}\"\n\
  fi\n\
  if (( CURRENT == 2 )); then\n\
    local -a commands\n\
    commands=(\n",
    );
    for command_name in TOP_LEVEL_NAMES {
        let description = COMMANDS
            .iter()
            .find(|command| command.context == *command_name)
            .map_or("Stack CLI command", |command| command.description);
        let _ = writeln!(output, "      '{}:{}'", command_name, description);
    }
    for option in GLOBAL_OPTIONS {
        let _ = writeln!(output, "      '{}:Global option'", option);
    }
    output.push_str(
        "    )\n\
    _describe 'command' commands\n\
    return\n\
  fi\n\
  local -a candidates\n\
  case \"$context\" in\n",
    );
    for command in COMMANDS {
        let candidates = command
            .values
            .iter()
            .chain(command.options.iter())
            .copied()
            .collect::<Vec<_>>();
        let quoted = candidates
            .iter()
            .map(|candidate| format!("'{candidate}'"))
            .collect::<Vec<_>>()
            .join(" ");
        let _ = writeln!(
            output,
            "    \"{}\") candidates=({}) ;;",
            command.context, quoted
        );
    }
    output.push_str(
        "    *) candidates=() ;;\n\
  esac\n\
  compadd -- $candidates\n\
}\n\
compdef _stack stack\n",
    );
    output
}

fn fish_completion() -> String {
    let mut output = String::from(
        "# Generated by `stack completions fish`; do not edit.\n\
function __stack_needs_command\n\
    set -l tokens (commandline -opc)\n\
    test (count $tokens) -eq 1\n\
end\n",
    );
    for command_name in TOP_LEVEL_NAMES {
        let description = COMMANDS
            .iter()
            .find(|command| command.context == *command_name)
            .map_or("Stack CLI command", |command| command.description);
        let _ = writeln!(
            output,
            "complete -c stack -n __stack_needs_command -a '{}' -d '{}'",
            command_name, description
        );
    }
    for option in GLOBAL_OPTIONS {
        if let Some(long) = option.strip_prefix("--") {
            let _ = writeln!(
                output,
                "complete -c stack -n __stack_needs_command -l '{}'",
                long
            );
        } else if option.len() == 2 && option.starts_with('-') {
            let _ = writeln!(
                output,
                "complete -c stack -n __stack_needs_command -s '{}'",
                &option[1..]
            );
        }
    }
    for command in COMMANDS {
        let condition = if command.context.contains(' ') {
            let mut parts = command.context.split_whitespace();
            let parent = parts.next().unwrap_or_default();
            let child = parts.next().unwrap_or_default();
            format!("__fish_seen_subcommand_from {parent}; and __fish_seen_subcommand_from {child}")
        } else {
            format!("__fish_seen_subcommand_from {}", command.context)
        };
        for value in command.values {
            let _ = writeln!(
                output,
                "complete -c stack -n '{}' -a '{}'",
                condition, value
            );
        }
        for option in command.options {
            if let Some(long) = option.strip_prefix("--") {
                let requires_value = matches!(
                    *option,
                    "--template" | "--output" | "--provider-pack" | "--notice" | "--version"
                );
                let _ = writeln!(
                    output,
                    "complete -c stack -n '{}' -l '{}'{}",
                    condition,
                    long,
                    if requires_value { " -r" } else { "" }
                );
            } else if option.len() == 2 && option.starts_with('-') {
                let _ = writeln!(
                    output,
                    "complete -c stack -n '{}' -s '{}'",
                    condition,
                    &option[1..]
                );
            }
        }
    }
    output
}

fn roff_escape(line: &str) -> String {
    let escaped = line.replace('\\', "\\e").replace('-', "\\-");
    if escaped.starts_with('.') || escaped.starts_with('\'') {
        format!("\\&{escaped}")
    } else {
        escaped
    }
}

pub(crate) fn manpage() -> String {
    let mut output = format!(
        ".TH STACK 1 \"\" \"Stack CLI {}\" \"Stack CLI Manual\"\n\
.SH NAME\n\
stack \\- Stack diagram toolchain\n\
.SH SYNOPSIS\n\
.B stack\n\
.RI \"<COMMAND> [OPTIONS]\"\n\
.SH DESCRIPTION\n\
Stack validates, formats, renders, and develops Stack architecture diagrams.\n\
.SH COMMAND REFERENCE\n",
        env!("CARGO_PKG_VERSION")
    );
    for (title, help) in [
        ("stack", GENERAL_HELP),
        ("stack init", INIT_HELP),
        ("stack check", CHECK_HELP),
        ("stack fmt", FORMAT_HELP),
        ("stack render", RENDER_HELP),
        ("stack update", UPDATE_HELP),
        ("stack lsp", LSP_HELP),
        ("stack doctor", DOCTOR_HELP),
        ("stack config", CONFIG_HELP),
        ("stack config path", CONFIG_PATH_HELP),
        ("stack config get", CONFIG_GET_HELP),
        ("stack icons", ICONS_HELP),
        ("stack icons list", ICONS_LIST_HELP),
        ("stack icons import", ICONS_IMPORT_HELP),
        ("stack completions", COMPLETIONS_HELP),
        ("stack manpage", MANPAGE_HELP),
        ("stack help", HELP_HELP),
        ("stack version", VERSION_HELP),
    ] {
        let _ = writeln!(output, ".SS \"{}\"", roff_escape(title));
        output.push_str(".nf\n");
        for line in help.lines() {
            let _ = writeln!(output, "{}", roff_escape(line));
        }
        output.push_str(".fi\n");
    }
    output.push_str(
        ".SH FILES\n\
Configuration uses $XDG_CONFIG_HOME/stack or $HOME/.config/stack.\n\
.SH SEE ALSO\n\
.BR man (1),\n\
https://stack-diagram.com/docs/\n",
    );
    output
}

#[cfg(test)]
mod tests {
    use super::*;

    fn help_for(context: &str) -> Option<&'static str> {
        match context {
            "init" => Some(INIT_HELP),
            "check" => Some(CHECK_HELP),
            "fmt" => Some(FORMAT_HELP),
            "render" => Some(RENDER_HELP),
            "update" => Some(UPDATE_HELP),
            "lsp" => Some(LSP_HELP),
            "doctor" => Some(DOCTOR_HELP),
            "config" => Some(CONFIG_HELP),
            "config path" => Some(CONFIG_PATH_HELP),
            "config get" => Some(CONFIG_GET_HELP),
            "icons" => Some(ICONS_HELP),
            "icons list" => Some(ICONS_LIST_HELP),
            "icons import" => Some(ICONS_IMPORT_HELP),
            "completions" => Some(COMPLETIONS_HELP),
            "manpage" => Some(MANPAGE_HELP),
            "help" => Some(HELP_HELP),
            "version" => Some(VERSION_HELP),
            _ => None,
        }
    }

    #[test]
    fn command_inventory_is_present_in_help_and_generators() {
        for command in COMMANDS {
            let help = help_for(command.context);
            assert!(help.is_some(), "missing help page for {}", command.context);
            let Some(help) = help else {
                continue;
            };
            for option in command.options {
                assert!(
                    help.contains(option),
                    "{} help is missing option {option}",
                    command.context
                );
            }
            for value in command.values {
                assert!(
                    help.contains(value),
                    "{} help is missing value {value}",
                    command.context
                );
            }
        }
        for command_name in TOP_LEVEL_NAMES {
            let command = COMMANDS
                .iter()
                .find(|command| command.context == *command_name);
            assert!(command.is_some(), "missing metadata for {command_name}");
            let Some(command) = command else {
                continue;
            };
            assert!(GENERAL_HELP.contains(command_name));
            assert!(GENERAL_HELP.contains(command.description));
        }
        for shell in SHELLS {
            let generated = completion(shell);
            assert!(generated.is_ok(), "generation failed for {shell}");
            let generated = generated.unwrap_or_default();
            assert!(generated.ends_with('\n'));
            for command_name in TOP_LEVEL_NAMES {
                assert!(generated.contains(command_name));
            }
        }
    }

    #[test]
    fn unsupported_shell_is_rejected() {
        assert_eq!(
            completion("powershell"),
            Err("unsupported shell 'powershell'; supported shells: bash, zsh, fish".to_owned())
        );
    }

    #[test]
    fn manual_is_deterministic_and_contains_every_help_page() {
        assert_eq!(manpage(), manpage());
        let manual = manpage();
        assert!(manual.starts_with(".TH STACK 1"));
        assert!(manual.contains(".SS \"stack doctor\""));
        assert!(manual.contains(".SS \"stack config get\""));
        assert!(manual.contains(".SS \"stack icons import\""));
        assert!(manual.contains("Stack CLI Manual"));
    }
}
