//! Shell completion script generation.
//!
//! The scripts are rendered at runtime from the same JSON metadata that drives
//! `--help` (`cli-commands.json` / `cli-help.json` in `turbotokens-cli-parser`), so
//! completions stay in sync with the CLI automatically.

use serde_json::{Map, Value};

use crate::{
    Result,
    cli::{CompletionShell, CompletionsArgs},
};

const COMMANDS_JSON: &str = include_str!("../../../turbotokens-cli-parser/src/cli-commands.json");
const HELP_JSON: &str = include_str!("../../../turbotokens-cli-parser/src/cli-help.json");

#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
enum ValueKind {
    #[default]
    None,
    Required,
    Optional,
}

#[derive(Clone, Default)]
struct Flag {
    short: Option<String>,
    long: Option<String>,
    value: ValueKind,
    description: String,
    choices: Vec<String>,
}

impl Flag {
    fn names(&self) -> Vec<&str> {
        [&self.short, &self.long]
            .into_iter()
            .flatten()
            .map(String::as_str)
            .collect()
    }
}

#[derive(Default)]
struct Page {
    path: Vec<String>,
    flags: Vec<Flag>,
    subcommands: Vec<(String, String)>,
    /// Positional alternatives extracted from usage strings like
    /// `turbotokens daemon [run|start|stop|status]`.
    positionals: Vec<String>,
}

struct Spec {
    pages: Vec<Page>,
    /// Flags whose value comes from a fixed choice set: (flag names, choices).
    hint_flags: Vec<(Vec<String>, Vec<String>)>,
    /// Flag names that consume the following word as their value.
    value_flags: Vec<String>,
}

pub(super) fn run(args: CompletionsArgs) -> Result<()> {
    let spec = build_spec();
    let script = match args.shell {
        CompletionShell::Bash => render_bash(&spec),
        CompletionShell::Zsh => render_zsh(&spec),
        CompletionShell::Fish => render_fish(&spec),
    };
    println!("{script}");
    Ok(())
}

fn build_spec() -> Spec {
    let commands: Value = serde_json::from_str(COMMANDS_JSON).expect("parse cli-commands.json");
    let help: Value = serde_json::from_str(HELP_JSON).expect("parse cli-help.json");
    let Value::Object(commands) = commands else {
        panic!("cli-commands.json must be an object");
    };
    let Value::Object(help) = help else {
        panic!("cli-help.json must be an object");
    };
    let combined = commands
        .get("combinedOptions")
        .and_then(Value::as_object)
        .cloned()
        .unwrap_or_default();

    let mut pages = Vec::new();
    let mut root = Page::default();
    for command in array_field(&commands["root"], "commands") {
        root.subcommands.push((
            string_field(command, "name"),
            string_field(command, "description"),
        ));
    }

    let mut raw_pages = Vec::new();
    for page in commands["pages"]
        .as_array()
        .expect("pages must be an array")
    {
        let mut parsed = Page {
            path: array_field(page, "path")
                .iter()
                .map(|part| part.as_str().expect("page path part").to_string())
                .collect(),
            ..Page::default()
        };
        if let Some(options) = page.get("options").and_then(Value::as_str) {
            resolve_option_group(options, &combined, &help, &mut parsed.flags);
        }
        if let Some(subcommands) = page.get("commands").and_then(Value::as_array) {
            for command in subcommands {
                parsed.subcommands.push((
                    string_field(command, "name"),
                    string_field(command, "description"),
                ));
            }
        }
        if let Some(usage) = page.get("usage").and_then(Value::as_str) {
            parsed.positionals = usage_positionals(usage);
        }
        raw_pages.push(parsed);
    }

    // The root invocation defaults to the command bracketed in its usage
    // (`turbotokens [daily] <OPTIONS>`), so reuse that page's flags at the root.
    let mut root_flags = Vec::new();
    for usage in array_field(&commands["root"], "usage") {
        let Some(usage) = usage.as_str() else {
            continue;
        };
        for default in usage_default_commands(usage) {
            if let Some(page) = raw_pages
                .iter()
                .find(|page| page.path == [default.as_str()])
            {
                root_flags = page.flags.clone();
            }
        }
    }
    root.flags = root_flags;
    pages.push(root);
    pages.extend(raw_pages);

    let mut hint_flags: Vec<(Vec<String>, Vec<String>)> = Vec::new();
    let mut value_flags = Vec::new();
    for page in &pages {
        for flag in &page.flags {
            let flag_names = flag.names();
            if !flag.choices.is_empty()
                && !hint_flags
                    .iter()
                    .any(|(names, _)| names.iter().any(|name| flag_names.contains(&name.as_str())))
            {
                hint_flags.push((
                    flag_names.iter().map(|name| (*name).to_string()).collect(),
                    flag.choices.clone(),
                ));
            }
            if flag.value == ValueKind::Required {
                for name in flag_names {
                    if !value_flags.contains(&name.to_string()) {
                        value_flags.push(name.to_string());
                    }
                }
            }
        }
    }
    Spec {
        pages,
        hint_flags,
        value_flags,
    }
}

fn resolve_option_group(
    name: &str,
    combined: &Map<String, Value>,
    help: &Map<String, Value>,
    out: &mut Vec<Flag>,
) {
    if let Some(parts) = combined.get(name).and_then(Value::as_array) {
        for part in parts {
            resolve_option_group(
                part.as_str().expect("combined group name"),
                combined,
                help,
                out,
            );
        }
        return;
    }
    for entry in help
        .get(name)
        .unwrap_or_else(|| panic!("missing option group {name}"))
        .as_array()
        .unwrap_or_else(|| panic!("option group {name} must be an array"))
    {
        out.push(parse_flag(entry));
    }
}

fn parse_flag(entry: &Value) -> Flag {
    let flags = string_field(entry, "flags");
    let mut flag = Flag {
        description: string_field(entry, "description"),
        ..Flag::default()
    };
    for part in flags.split(',') {
        let mut tokens = part.split_whitespace();
        let Some(name) = tokens.next() else {
            continue;
        };
        if let Some(value) = tokens.next() {
            flag.value = if value.starts_with('<') {
                ValueKind::Required
            } else {
                ValueKind::Optional
            };
        }
        if let Some(long) = name.strip_prefix("--") {
            flag.long = Some(format!("--{long}"));
        } else if let Some(short) = name.strip_prefix('-') {
            flag.short = Some(format!("-{short}"));
        }
    }
    if let Some(choices) = entry.get("choices").and_then(Value::as_array) {
        flag.choices = choices
            .iter()
            .map(|choice| choice.as_str().expect("choice").to_string())
            .collect();
    } else if let Some(choices) = description_choices(&flag.description) {
        flag.choices = choices;
    }
    flag
}

/// Extract `(choices: a | b | c)` annotations from a flag description.
fn description_choices(description: &str) -> Option<Vec<String>> {
    let start = description.find("(choices: ")? + "(choices: ".len();
    let end = description[start..].find(')')? + start;
    let choices = description[start..end]
        .split('|')
        .map(str::trim)
        .filter(|choice| !choice.is_empty())
        .map(str::to_string)
        .collect::<Vec<_>>();
    (!choices.is_empty()).then_some(choices)
}

/// Extract `[a|b|c]` / `<a|b|c>` positional alternatives from a usage string.
fn usage_positionals(usage: &str) -> Vec<String> {
    let mut positionals = Vec::new();
    let bytes = usage.as_bytes();
    let mut index = 0;
    while index < bytes.len() {
        let open = match bytes[index] {
            b'[' => b']',
            b'<' => b'>',
            _ => {
                index += 1;
                continue;
            }
        };
        let Some(close) = usage[index + 1..].find(open as char) else {
            break;
        };
        let inner = &usage[index + 1..index + 1 + close];
        if inner.contains('|') {
            for part in inner.split('|') {
                let part = part.trim();
                if part
                    .chars()
                    .all(|ch| ch.is_ascii_lowercase() || ch.is_ascii_digit() || ch == '-')
                    && !part.is_empty()
                {
                    positionals.push(part.to_string());
                }
            }
        }
        index += close + 2;
    }
    positionals
}

/// Extract `[name]` default-command brackets from a usage string.
fn usage_default_commands(usage: &str) -> Vec<String> {
    let mut defaults = Vec::new();
    let mut rest = usage;
    while let Some(start) = rest.find('[') {
        let Some(end) = rest[start..].find(']') else {
            break;
        };
        let inner = rest[start + 1..start + end].trim();
        if !inner.contains('|')
            && inner
                .chars()
                .all(|ch| ch.is_ascii_lowercase() || ch.is_ascii_digit() || ch == '-')
            && !inner.is_empty()
        {
            defaults.push(inner.to_string());
        }
        rest = &rest[start + end + 1..];
    }
    defaults
}

fn page_words(page: &Page) -> (String, String) {
    let subs = page
        .subcommands
        .iter()
        .map(|(name, _)| name.clone())
        .chain(page.positionals.iter().cloned())
        .collect::<Vec<_>>()
        .join(" ");
    let flags = page
        .flags
        .iter()
        .flat_map(Flag::names)
        .map(str::to_string)
        .collect::<Vec<_>>()
        .join(" ");
    (subs, flags)
}

fn render_bash(spec: &Spec) -> String {
    let mut out = String::from(
        "# bash completion for turbotokens\n\
         _turbotokens() {\n\
         \x20   local cur prev\n\
         \x20   cur=\"${COMP_WORDS[COMP_CWORD]}\"\n\
         \x20   prev=\"${COMP_WORDS[COMP_CWORD-1]}\"\n\
         \n\
         \x20   case \"$prev\" in\n",
    );
    for (names, choices) in &spec.hint_flags {
        out.push_str(&format!(
            "        {}) COMPREPLY=( $(compgen -W \"{}\" -- \"$cur\") ); return 0 ;;\n",
            names.join("|"),
            choices.join(" ")
        ));
    }
    out.push_str("    esac\n\n");
    if !spec.value_flags.is_empty() {
        out.push_str(&format!(
            "    case \"$prev\" in\n        {}) return 0 ;;\n    esac\n\n",
            spec.value_flags.join("|")
        ));
    }

    out.push_str(
        "    local -a positionals=()\n\
         \x20   local i=1 word\n\
         \x20   while (( i < COMP_CWORD )); do\n\
         \x20       word=\"${COMP_WORDS[i]}\"\n\
         \x20       case \"$word\" in\n\
         \x20           --?*=*) ;;\n",
    );
    if !spec.value_flags.is_empty() {
        out.push_str(&format!(
            "            {}) (( i += 1 )) ;;\n",
            spec.value_flags.join("|")
        ));
    }
    out.push_str(
        "            -*) ;;\n\
         \x20           *) positionals+=(\"$word\") ;;\n\
         \x20       esac\n\
         \x20       (( i += 1 ))\n\
         \x20   done\n\
         \x20   local path=\"${positionals[*]}\"\n\n\
         \x20   local subs=\"\" flags=\"\"\n\
         \x20   case \"$path\" in\n",
    );
    for page in &spec.pages {
        let (subs, flags) = page_words(page);
        if subs.is_empty() && flags.is_empty() {
            continue;
        }
        out.push_str(&format!(
            "        \"{}\") subs=\"{subs}\" flags=\"{flags}\" ;;\n",
            page.path.join(" ")
        ));
    }
    out.push_str(
        "    esac\n\n\
         \x20   COMPREPLY=( $(compgen -W \"$subs $flags\" -- \"$cur\") )\n\
         }\n\
         complete -F _turbotokens turbotokens\n",
    );
    out
}

fn render_zsh(spec: &Spec) -> String {
    let mut out = String::from(
        "#compdef turbotokens\n\
         # zsh completion for turbotokens\n\
         _turbotokens() {\n\
         \x20   local cur prev\n\
         \x20   cur=\"${words[CURRENT]}\"\n\
         \x20   prev=\"${words[CURRENT-1]}\"\n\
         \n\
         \x20   case \"$prev\" in\n",
    );
    for (names, choices) in &spec.hint_flags {
        out.push_str(&format!(
            "        ({})\n            compadd -- {}\n            return\n            ;;\n",
            names.join("|"),
            choices.join(" ")
        ));
    }
    out.push_str("    esac\n\n");
    if !spec.value_flags.is_empty() {
        out.push_str(&format!(
            "    case \"$prev\" in\n        ({}) return ;;\n    esac\n\n",
            spec.value_flags.join("|")
        ));
    }

    out.push_str(
        "    local -a positionals\n\
         \x20   local i=2 word\n\
         \x20   while (( i < CURRENT )); do\n\
         \x20       word=\"${words[i]}\"\n\
         \x20       case \"$word\" in\n\
         \x20           (--?*=*) ;;\n",
    );
    if !spec.value_flags.is_empty() {
        out.push_str(&format!(
            "            ({}) (( i += 1 )) ;;\n",
            spec.value_flags.join("|")
        ));
    }
    out.push_str(
        "            (-*) ;;\n\
         \x20           (*) positionals+=(\"$word\") ;;\n\
         \x20       esac\n\
         \x20       (( i += 1 ))\n\
         \x20   done\n\
         \x20   local path=\"${(j: :)positionals}\"\n\n\
         \x20   local -a subs flags\n\
         \x20   case \"$path\" in\n",
    );
    for page in &spec.pages {
        let (subs, flags) = page_words(page);
        if subs.is_empty() && flags.is_empty() {
            continue;
        }
        let mut entry = format!("        (\"{}\")\n", page.path.join(" "));
        if !subs.is_empty() {
            entry.push_str(&format!("            subs=({subs})\n"));
        }
        if !flags.is_empty() {
            entry.push_str(&format!("            flags=({flags})\n"));
        }
        entry.push_str("            ;;\n");
        out.push_str(&entry);
    }
    out.push_str(
        "    esac\n\n\
         \x20   (( ${#subs[@]} )) && compadd -- \"${subs[@]}\"\n\
         \x20   (( ${#flags[@]} )) && compadd -- \"${flags[@]}\"\n\
         }\n\
         compdef _turbotokens turbotokens\n",
    );
    out
}

fn render_fish(spec: &Spec) -> String {
    let mut out = String::from(
        "# fish completion for turbotokens\n\
         function __fish_turbotokens_positionals\n\
         \x20   set -l raw (commandline -opc)\n\
         \x20   set -e raw[1]\n\
         \x20   set -l tokens\n\
         \x20   set -l skip 0\n\
         \x20   for token in $raw\n\
         \x20       if test $skip -eq 1\n\
         \x20           set skip 0\n\
         \x20           continue\n\
         \x20       end\n\
         \x20       switch $token\n\
         \x20           case '--?*=*'\n",
    );
    if !spec.value_flags.is_empty() {
        let value_cases = spec
            .value_flags
            .iter()
            .map(|flag| format!("'{flag}'"))
            .collect::<Vec<_>>()
            .join(" ");
        out.push_str(&format!(
            "            case {value_cases}\n                set skip 1\n"
        ));
    }
    out.push_str(
        "            case '-*'\n\
         \x20           case '*'\n\
         \x20               set -a tokens $token\n\
         \x20       end\n\
         \x20   end\n\
         \x20   string join ' ' $tokens\n\
         end\n\
         \n\
         function __fish_turbotokens_at\n\
         \x20   set -l actual (__fish_turbotokens_positionals)\n\
         \x20   set -l expected (string join ' ' $argv)\n\
         \x20   test \"$actual\" = \"$expected\"\n\
         end\n\n",
    );

    for page in &spec.pages {
        let condition = fish_condition(&page.path);
        for (name, description) in &page.subcommands {
            out.push_str(&format!(
                "complete -c turbotokens -n '{condition}' -a '{}' -d '{}'\n",
                fish_escape(name),
                fish_escape(description)
            ));
        }
        if !page.positionals.is_empty() {
            out.push_str(&format!(
                "complete -c turbotokens -n '{condition}' -a '{}'\n",
                fish_escape(&page.positionals.join(" "))
            ));
        }
        for flag in &page.flags {
            let mut line = format!("complete -c turbotokens -n '{condition}'");
            if let Some(short) = &flag.short {
                line.push_str(&format!(" -s {}", &short[1..]));
            }
            if let Some(long) = &flag.long {
                line.push_str(&format!(" -l {}", &long[2..]));
            }
            if !flag.choices.is_empty() {
                line.push_str(&format!(" -a '{}'", fish_escape(&flag.choices.join(" "))));
            }
            if flag.value == ValueKind::Required {
                line.push_str(" -r");
            }
            line.push_str(&format!(" -d '{}'", fish_escape(&flag.description)));
            line.push('\n');
            out.push_str(&line);
        }
    }
    out
}

fn fish_condition(path: &[String]) -> String {
    if path.is_empty() {
        return "__fish_turbotokens_at".to_string();
    }
    format!("__fish_turbotokens_at {}", path.join(" "))
}

fn fish_escape(value: &str) -> String {
    value.replace('\\', "\\\\").replace('\'', "\\'")
}

fn array_field<'a>(object: &'a Value, field: &str) -> &'a [Value] {
    object
        .get(field)
        .and_then(Value::as_array)
        .unwrap_or_else(|| panic!("missing array field {field}"))
}

fn string_field(value: &Value, field: &str) -> String {
    value
        .get(field)
        .and_then(Value::as_str)
        .unwrap_or_else(|| panic!("missing string field {field}"))
        .to_string()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_flag_forms() {
        let flag = parse_flag(&serde_json::json!({
            "flags": "-i, --instances",
            "description": "breakdown"
        }));
        assert_eq!(flag.short.as_deref(), Some("-i"));
        assert_eq!(flag.long.as_deref(), Some("--instances"));
        assert_eq!(flag.value, ValueKind::None);

        let flag = parse_flag(&serde_json::json!({
            "flags": "-m, --mode [mode]",
            "description": "mode",
            "choices": ["auto", "calculate", "display"]
        }));
        assert_eq!(flag.value, ValueKind::Optional);
        assert_eq!(flag.choices, ["auto", "calculate", "display"]);

        let flag = parse_flag(&serde_json::json!({
            "flags": "--interval <interval>",
            "description": "poll"
        }));
        assert_eq!(flag.value, ValueKind::Required);
    }

    #[test]
    fn extracts_choices_from_description() {
        assert_eq!(
            description_choices("Agent to watch (choices: claude | codex | grok)"),
            Some(vec![
                "claude".to_string(),
                "codex".to_string(),
                "grok".to_string(),
            ])
        );
        assert_eq!(description_choices("no choices here"), None);
    }

    #[test]
    fn extracts_usage_positionals_and_defaults() {
        assert_eq!(
            usage_positionals("turbotokens daemon [run|start|stop|status] <OPTIONS>"),
            ["run", "start", "stop", "status"]
        );
        assert_eq!(
            usage_positionals("turbotokens completions <bash|zsh|fish>"),
            ["bash", "zsh", "fish"]
        );
        assert_eq!(
            usage_default_commands("turbotokens [daily] <OPTIONS>"),
            ["daily"]
        );
    }

    #[test]
    fn bash_script_covers_commands_flags_and_hints() {
        let spec = build_spec();
        let script = render_bash(&spec);
        for needle in [
            "complete -F _turbotokens turbotokens",
            "daily",
            "statusline",
            "openclaw",
            "--visual-burn-rate",
            "--cost-source",
            "auto calculate display",
            "run start stop status",
            "bash zsh fish",
            "--since",
        ] {
            assert!(script.contains(needle), "bash script missing {needle}");
        }
    }

    #[test]
    fn zsh_script_covers_commands_flags_and_hints() {
        let spec = build_spec();
        let script = render_zsh(&spec);
        for needle in [
            "#compdef turbotokens",
            "compdef _turbotokens turbotokens",
            "compadd -- auto calculate display",
            "subs=(daily monthly weekly session blocks statusline limits)",
            "--start-of-week",
        ] {
            assert!(script.contains(needle), "zsh script missing {needle}");
        }
    }

    #[test]
    fn fish_script_covers_commands_flags_and_hints() {
        let spec = build_spec();
        let script = render_fish(&spec);
        for needle in [
            "complete -c turbotokens",
            "__fish_turbotokens_at claude daily",
            "-l mode -a 'auto calculate display'",
            "-l agent -a 'claude codex grok'",
            "-a 'run start stop status'",
            "-a 'bash zsh fish'",
        ] {
            assert!(script.contains(needle), "fish script missing {needle}");
        }
    }

    #[test]
    #[ignore = "requires fish on PATH; run with --ignored"]
    fn fish_completes_empty_and_partial_contexts_without_errors() {
        use std::process::Command;
        use turbotokens_test_support::Fixture;

        let fixture = Fixture::new();
        let script = fixture.write_file("turbotokens.fish", render_fish(&build_spec()));
        let home = fixture.create_dir_all("home");
        let empty = fixture.create_dir_all("empty");
        let syntax = Command::new("fish")
            .args(["--no-config", "-n"])
            .arg(&script)
            .output()
            .expect("run fish syntax check");
        assert!(syntax.status.success());
        assert!(syntax.stderr.is_empty());

        for (line, expected) in [
            ("turbotokens ", vec!["claude", "codex", "daily", "daemon"]),
            (
                "turbotokens claude ",
                vec![
                    "blocks",
                    "daily",
                    "limits",
                    "monthly",
                    "session",
                    "statusline",
                    "weekly",
                ],
            ),
            ("turbotokens claude da", vec!["daily"]),
            ("turbotokens claude daily --mo", vec!["--mode", "--mode="]),
            (
                "turbotokens claude daily --mode=",
                vec!["--mode=auto", "--mode=calculate", "--mode=display"],
            ),
        ] {
            let output = Command::new("fish")
                .args([
                    "--no-config",
                    "-c",
                    "source $argv[1]; complete --do-complete $argv[2]",
                ])
                .arg(&script)
                .arg(line)
                .env("HOME", &home)
                .env("XDG_CONFIG_HOME", &home)
                .env("XDG_CACHE_HOME", &home)
                .current_dir(&empty)
                .output()
                .expect("run fish completion");
            assert!(output.status.success(), "fish failed for {line:?}");
            assert!(
                output.stderr.is_empty(),
                "{line:?}: {}",
                String::from_utf8_lossy(&output.stderr)
            );
            let stdout = String::from_utf8(output.stdout).unwrap();
            let candidates = stdout
                .lines()
                .map(|line| line.split('\t').next().unwrap())
                .collect::<Vec<_>>();
            if line == "turbotokens " {
                for candidate in expected {
                    assert!(
                        candidates.contains(&candidate),
                        "{line:?}: missing {candidate}"
                    );
                }
            } else {
                assert_eq!(candidates, expected, "{line:?}");
            }
        }
    }
}
