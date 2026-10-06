use std::path::Path;

/// How to open a file at a line in the user's editor.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct EditorCommand {
    pub program: String,
    pub args: Vec<String>,
    /// The editor runs in this terminal, so the viewer must step aside until it exits.
    pub in_terminal: bool,
}

/// Builds the command for `editor` (the value of `$VISUAL` or `$EDITOR`, which may carry
/// its own arguments, like `code --wait`), or the system opener when none is set.
pub fn editor_command(editor: Option<&str>, file: &Path, line: usize) -> EditorCommand {
    let file = file.display().to_string();
    let mut words = editor.unwrap_or("").split_whitespace().map(str::to_string);
    let Some(program) = words.next() else {
        let opener = if cfg!(target_os = "macos") {
            "open"
        } else {
            "xdg-open"
        };
        return EditorCommand {
            program: opener.to_string(),
            args: vec![file],
            in_terminal: false,
        };
    };
    let mut args: Vec<String> = words.collect();
    let name = Path::new(&program)
        .file_name()
        .map(|name| name.to_string_lossy().into_owned())
        .unwrap_or_default();

    let in_terminal = matches!(
        name.as_str(),
        "vim" | "nvim" | "vi" | "nano" | "emacs" | "micro" | "hx" | "helix" | "kak"
    );
    match name.as_str() {
        "code" | "code-insiders" | "cursor" | "windsurf" | "codium" => {
            args.extend(["-g".to_string(), format!("{file}:{line}")])
        }
        "subl" | "zed" | "hx" | "helix" => args.push(format!("{file}:{line}")),
        "vim" | "nvim" | "vi" | "nano" | "emacs" | "micro" | "kak" => {
            args.extend([format!("+{line}"), file])
        }
        "xed" | "idea" | "pycharm" | "webstorm" | "fleet" | "mate" => {
            args.extend(["--line".to_string(), line.to_string(), file])
        }
        _ => args.push(file),
    }
    EditorCommand {
        program,
        args,
        in_terminal,
    }
}
