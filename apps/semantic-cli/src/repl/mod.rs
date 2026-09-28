use std::{
    borrow::Cow,
    io::IsTerminal,
    path::PathBuf,
    sync::{Arc, Mutex},
    time::Instant,
};

use datafusion::sql::sqlparser::{keywords::Keyword, tokenizer::Token};
use rustyline::{
    Cmd, CompletionType, ConditionalEventHandler, Config, Context, Editor, Event, EventContext,
    EventHandler, Helper, KeyCode, KeyEvent, Modifiers, RepeatCount,
    completion::{Completer, Pair},
    error::ReadlineError,
    highlight::{CmdKind, Highlighter},
    hint::{Hinter, HistoryHinter},
    history::DefaultHistory,
    validate::{ValidationContext, ValidationResult, Validator},
};

use crate::{Compiler, Engine, OpenAiProvider, ReadMode, Result, config, run_ask, run_query};

mod command;
mod completion;
mod history;
mod lex;
mod progress;

pub(crate) use progress::Reporter as ProgressReporter;

type ReplEditor = Editor<EditorHelper, DefaultHistory>;

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum ReplMode {
    Sql,
    Ask,
}

impl ReplMode {
    fn prompt(self) -> &'static str {
        match self {
            Self::Sql => "sdb sql> ",
            Self::Ask => "sdb ask> ",
        }
    }

    fn toggle(self) -> Self {
        match self {
            Self::Sql => Self::Ask,
            Self::Ask => Self::Sql,
        }
    }

    fn name(self) -> &'static str {
        match self {
            Self::Sql => "SQL",
            Self::Ask => "Ask",
        }
    }

    fn complete(self, input: &str) -> bool {
        self == Self::Ask || lex::complete(input)
    }
}

struct Draft {
    text: String,
    cursor: usize,
}

struct SwitchMode {
    draft: Arc<Mutex<Option<Draft>>>,
}

impl ConditionalEventHandler for SwitchMode {
    fn handle(&self, _: &Event, _: RepeatCount, _: bool, ctx: &EventContext) -> Option<Cmd> {
        *self.draft.lock().expect("mode switch lock") = Some(Draft {
            text: ctx.line().to_owned(),
            cursor: ctx.pos(),
        });
        Some(Cmd::AcceptLine)
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum InputKind {
    Command,
    Sql,
    Ask,
}

fn input_kind(mode: ReplMode, input: &str) -> InputKind {
    if input.starts_with('.') {
        InputKind::Command
    } else if mode == ReplMode::Ask {
        InputKind::Ask
    } else {
        InputKind::Sql
    }
}

fn history_entry(kind: InputKind, input: &str) -> String {
    if kind == InputKind::Ask {
        format!(".ask {input}")
    } else {
        input.to_owned()
    }
}

pub(super) async fn run(
    engine: &mut Engine,
    project_path: Option<PathBuf>,
    no_color: bool,
    no_history: bool,
    history_file: Option<PathBuf>,
    read_mode: ReadMode,
) -> Result<()> {
    let dumb = std::env::var("TERM").is_ok_and(|term| {
        ["dumb", "cons25", "emacs"]
            .iter()
            .any(|name| term.eq_ignore_ascii_case(name))
    });
    let color = colors_enabled(
        no_color,
        std::io::stdout().is_terminal(),
        std::env::var_os("NO_COLOR").is_some_and(|s| !s.is_empty()),
        dumb,
    );
    let config = Config::builder()
        .max_history_size(1_000)?
        .history_ignore_dups(true)?
        .completion_type(CompletionType::List)
        .color_mode(if color {
            rustyline::ColorMode::Enabled
        } else {
            rustyline::ColorMode::Disabled
        })
        .build();
    let mut editor = ReplEditor::with_config(config)?;
    let draft = Arc::new(Mutex::new(None));
    editor.bind_sequence(
        KeyEvent(KeyCode::BackTab, Modifiers::NONE),
        EventHandler::Conditional(Box::new(SwitchMode {
            draft: Arc::clone(&draft),
        })),
    );
    let mut mode = ReplMode::Sql;
    let mut helper = EditorHelper::new(color);
    helper.catalog.refresh(engine);
    editor.set_helper(Some(helper));
    let mut history = history::Storage::new(no_history, history_file);
    history.load(&mut editor);
    welcome(
        project_path.as_deref(),
        engine.catalog().relations().count(),
        &history.description(),
        color,
        dumb,
    );
    let progress = ProgressReporter::new(color && std::io::stderr().is_terminal());
    let mut compiler = None;
    let mut initial: Option<Draft> = None;
    loop {
        let prompt = mode.prompt();
        let next = if let Some(draft) = initial.take() {
            editor.readline_with_initial(
                prompt,
                (&draft.text[..draft.cursor], &draft.text[draft.cursor..]),
            )
        } else {
            editor.readline(prompt)
        };
        if let Some(restored) = draft.lock().expect("mode switch lock").take() {
            mode = mode.toggle();
            editor.helper_mut().expect("REPL helper").mode = mode;
            initial = Some(restored);
            continue;
        }
        match next {
            Ok(line) => {
                let trimmed = line.trim();
                if trimmed.is_empty()
                    || (mode == ReplMode::Sql
                        && !trimmed.starts_with('.')
                        && lex::lex(trimmed).tokens.iter().all(lex::Lexeme::whitespace))
                {
                    continue;
                }
                let kind = input_kind(mode, trimmed);
                history.record(&mut editor, &history_entry(kind, trimmed));
                if matches!(trimmed, ".quit" | ".exit") {
                    break;
                }
                let started = Instant::now();
                let result = match kind {
                    InputKind::Command => {
                        command::run(
                            engine,
                            &mut compiler,
                            &mut mode,
                            trimmed,
                            &read_mode,
                            &progress,
                        )
                        .await
                    }
                    InputKind::Ask => {
                        async {
                            if compiler.is_none() {
                                compiler = Some(Compiler::new(config::provider()?));
                            }
                            run_ask(
                                engine,
                                compiler.as_ref().expect("compiler initialized"),
                                trimmed,
                                false,
                                &read_mode,
                                Some(&progress),
                            )
                            .await
                        }
                        .await
                    }
                    InputKind::Sql => {
                        if semantic_engine::is_write_statement(trimmed) {
                            crate::run_write(
                                engine,
                                trimmed,
                                semantic_engine::is_write_explanation(trimmed),
                            )
                            .await
                        } else {
                            run_query(engine, trimmed, &read_mode).await
                        }
                    }
                };
                editor.helper_mut().expect("REPL helper").mode = mode;
                let elapsed = started.elapsed();
                match result {
                    Ok(()) => {
                        // Metadata/help commands don't need execution summaries.
                        if kind != InputKind::Command
                            || [".ask", ".plan"]
                                .contains(&trimmed.split_whitespace().next().unwrap_or(""))
                        {
                            println!(
                                "{} in {:.3}s",
                                paint("Completed", "32", color),
                                elapsed.as_secs_f64()
                            );
                        }
                    }
                    Err(error) => eprintln!(
                        "{}: {error}\nFailed after {:.3}s",
                        paint("Error", "1;31", color && std::io::stderr().is_terminal()),
                        elapsed.as_secs_f64()
                    ),
                }
            }
            Err(ReadlineError::Interrupted) => {}
            Err(ReadlineError::Eof) => break,
            Err(error) => return Err(error.into()),
        }
    }
    Ok(())
}

fn colors_enabled(no_color: bool, terminal: bool, env_no_color: bool, dumb: bool) -> bool {
    terminal && !no_color && !env_no_color && !dumb
}

fn paint(text: &str, style: &str, enabled: bool) -> String {
    if enabled {
        format!("\x1b[{style}m{text}\x1b[0m")
    } else {
        text.to_owned()
    }
}

fn welcome(
    project: Option<&std::path::Path>,
    relations: usize,
    history: &str,
    color: bool,
    dumb: bool,
) {
    if dumb {
        println!("  o o\n o [=]  {}", paint("SemanticDB", "1;36", color));
    } else {
        println!("  • •\n ○ ▤   {}", paint("SemanticDB", "1;36", color));
    }
    println!(
        "Project: {} · {} relation(s)",
        project.map_or_else(|| "none".to_owned(), |path| path.display().to_string()),
        relations
    );
    println!("History: {history}");
    println!("SQL mode · Shift-Tab switches SQL / Ask · .help lists commands");
}

struct EditorHelper {
    catalog: completion::Catalog,
    hinter: HistoryHinter,
    color: bool,
    mode: ReplMode,
}

impl EditorHelper {
    fn new(color: bool) -> Self {
        Self {
            catalog: completion::Catalog::default(),
            hinter: HistoryHinter::new(),
            color,
            mode: ReplMode::Sql,
        }
    }

    fn highlighted(&self, input: &str) -> String {
        if !self.color {
            return input.into();
        }
        let leading = input.len() - input.trim_start().len();
        if input[leading..].starts_with('.') {
            let end = input[leading..]
                .find(char::is_whitespace)
                .map_or(input.len(), |i| leading + i);
            let mut result = input[..leading].to_owned();
            result.push_str(&paint(&input[leading..end], "1;36", true));
            result.push_str(&input[end..]);
            return result;
        }
        if self.mode == ReplMode::Ask {
            return input.into();
        }
        self.highlight_sql(input)
    }

    fn candidates(&self, line: &str, pos: usize) -> (usize, Vec<String>) {
        if self.mode == ReplMode::Ask && !line.trim_start().starts_with('.') {
            (pos, Vec::new())
        } else {
            self.catalog.complete(line, pos)
        }
    }

    fn highlight_sql(&self, input: &str) -> String {
        let lexed = lex::lex(input);
        let mut result = String::new();
        let mut last = 0;
        for token in lexed.tokens {
            result.push_str(&input[last..token.start]);
            let text = &input[token.start..token.end];
            let style = if token.comment() {
                "2"
            } else if token.string() {
                "32"
            } else {
                match &token.token {
                    Token::Word(w)
                        if w.keyword != Keyword::NoKeyword && w.quote_style.is_none() =>
                    {
                        "1;34"
                    }
                    Token::Number(_, _) => "35",
                    _ => "",
                }
            };
            if style.is_empty() {
                result.push_str(text);
            } else {
                result.push_str(&paint(text, style, self.color));
            }
            last = token.end;
        }
        result.push_str(&input[last..]);
        result
    }
}

impl Helper for EditorHelper {}

impl Completer for EditorHelper {
    type Candidate = Pair;
    fn update(
        &self,
        line: &mut rustyline::line_buffer::LineBuffer,
        start: usize,
        elected: &str,
        changes: &mut rustyline::Changeset,
    ) {
        let end = completion::replacement_end(line.as_str(), start, line.pos());
        line.replace(start..end, elected, changes);
    }
    fn complete(
        &self,
        line: &str,
        pos: usize,
        _: &Context<'_>,
    ) -> rustyline::Result<(usize, Vec<Pair>)> {
        let (start, candidates) = self.candidates(line, pos);
        Ok((
            start,
            candidates
                .into_iter()
                .map(|s| Pair {
                    display: s.clone(),
                    replacement: s,
                })
                .collect(),
        ))
    }
}

impl Hinter for EditorHelper {
    type Hint = String;
    fn hint(&self, line: &str, pos: usize, ctx: &Context<'_>) -> Option<String> {
        if self.color && (self.mode == ReplMode::Sql || line.trim_start().starts_with('.')) {
            self.hinter.hint(line, pos, ctx)
        } else {
            None
        }
    }
}

impl Validator for EditorHelper {
    fn validate(&self, ctx: &mut ValidationContext<'_>) -> rustyline::Result<ValidationResult> {
        Ok(if self.mode.complete(ctx.input()) {
            ValidationResult::Valid(None)
        } else {
            ValidationResult::Incomplete
        })
    }
}

impl Highlighter for EditorHelper {
    fn highlight<'l>(&self, line: &'l str, _: usize) -> Cow<'l, str> {
        if self.color {
            Cow::Owned(self.highlighted(line))
        } else {
            Cow::Borrowed(line)
        }
    }
    fn highlight_prompt<'b, 's: 'b, 'p: 'b>(&'s self, prompt: &'p str, _: bool) -> Cow<'b, str> {
        let style = if self.mode == ReplMode::Ask {
            "1;35"
        } else {
            "1;36"
        };
        Cow::Owned(paint(prompt, style, self.color))
    }
    fn highlight_hint<'h>(&self, hint: &'h str) -> Cow<'h, str> {
        // Without styling, ghost text is indistinguishable from editable input.
        Cow::Owned(if self.color {
            paint(hint, "2", true)
        } else {
            String::new()
        })
    }
    fn highlight_char(&self, _: &str, _: usize, _: CmdKind) -> bool {
        self.color
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn modes_route_validate_complete_and_record_history() {
        assert_eq!(input_kind(ReplMode::Sql, "SELECT 1;"), InputKind::Sql);
        assert_eq!(input_kind(ReplMode::Ask, "show wells"), InputKind::Ask);
        assert_eq!(
            input_kind(ReplMode::Ask, ".plan show wells"),
            InputKind::Command
        );
        assert!(!ReplMode::Sql.complete("SELECT 1"));
        assert!(ReplMode::Ask.complete("show wells"));
        assert_eq!(
            history_entry(InputKind::Ask, "show wells"),
            ".ask show wells"
        );
        assert_eq!(history_entry(InputKind::Sql, "SELECT 1;"), "SELECT 1;");

        let mut helper = EditorHelper::new(true);
        assert!(helper.candidates("SEL", 3).1.contains(&"SELECT".to_owned()));
        helper.mode = ReplMode::Ask;
        assert!(helper.candidates("SELECT ", 7).1.is_empty());
        assert_eq!(helper.highlighted("SELECT 1"), "SELECT 1");
        assert!(helper.candidates(".mo", 3).1.contains(&".mode".to_owned()));
    }

    #[test]
    fn color_policy_and_lossless_highlighting() {
        assert!(colors_enabled(false, true, false, false));
        for flags in [
            (true, true, false, false),
            (false, false, false, false),
            (false, true, true, false),
            (false, true, false, true),
        ] {
            assert!(!colors_enabled(flags.0, flags.1, flags.2, flags.3));
        }
        let helper = EditorHelper::new(true);
        for input in [
            "SELECT \"café\", 'it''s fine', 42; -- hi",
            ".ask SELECT isn't SQL",
            ".ask show wells",
            "SELECT 'unfinished",
            "SELECT 1\r\nFROM wells;",
        ] {
            let colored = helper.highlighted(input);
            let mut plain = String::new();
            let mut escape = false;
            for ch in colored.chars() {
                if ch == '\x1b' {
                    escape = true;
                } else if escape {
                    if ch == 'm' {
                        escape = false;
                    }
                } else {
                    plain.push(ch);
                }
            }
            assert_eq!(plain, input);
            assert_eq!(EditorHelper::new(false).highlighted(input), input);
        }
    }

    #[tokio::test]
    async fn configured_views_are_available_to_completion() {
        let mut engine = Engine::new();
        let mut catalog = completion::Catalog::default();
        engine
            .create_view("new_view", "SELECT 1 AS answer")
            .await
            .unwrap();
        catalog.refresh(&engine);
        assert_eq!(catalog.complete(".schema new", 11).1, ["new_view"]);
        assert_eq!(
            catalog.complete("SELECT new_view. FROM new_view", 16).1,
            ["answer"]
        );
    }
}
