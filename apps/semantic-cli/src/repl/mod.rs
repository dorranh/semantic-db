use std::{borrow::Cow, io::IsTerminal, path::PathBuf, time::Instant};

use datafusion::sql::sqlparser::{keywords::Keyword, tokenizer::Token};
use rustyline::{
    CompletionType, Config, Context, Editor, Helper,
    completion::{Completer, Pair},
    error::ReadlineError,
    highlight::{CmdKind, Highlighter},
    hint::{Hinter, HistoryHinter},
    history::DefaultHistory,
    validate::{ValidationContext, ValidationResult, Validator},
};

use crate::{
    Compiler, Engine, OpenAiProvider, Result, config, parse_assignment, run_ask, run_query,
};

mod command;
mod completion;
mod history;
mod lex;

type ReplEditor = Editor<EditorHelper, DefaultHistory>;

pub(super) async fn run(
    engine: &mut Engine,
    no_color: bool,
    no_history: bool,
    history_file: Option<PathBuf>,
) -> Result<()> {
    let color = colors_enabled(
        no_color,
        std::io::stdout().is_terminal(),
        std::env::var_os("NO_COLOR").is_some_and(|s| !s.is_empty()),
        std::env::var("TERM").is_ok_and(|term| {
            ["dumb", "cons25", "emacs"]
                .iter()
                .any(|name| term.eq_ignore_ascii_case(name))
        }),
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
    let mut helper = EditorHelper::new(color);
    helper.catalog.refresh(engine);
    editor.set_helper(Some(helper));
    let mut history = history::Storage::new(no_history, history_file);
    history.load(&mut editor);
    println!(
        "{} — {} relation(s)\nEnd SQL with ; · Tab completes · Ctrl-R searches · .help for commands",
        paint("Semantic DB", "1;36", color),
        engine.catalog().relations().count()
    );
    println!("History: {}", history.description());
    let mut compiler = None;
    loop {
        match editor.readline("semantic> ") {
            Ok(line) => {
                let trimmed = line.trim();
                if trimmed.is_empty()
                    || (!trimmed.starts_with('.')
                        && lex::lex(trimmed).tokens.iter().all(lex::Lexeme::whitespace))
                {
                    continue;
                }
                history.record(&mut editor, trimmed);
                if matches!(trimmed, ".quit" | ".exit") {
                    break;
                }
                let started = Instant::now();
                let result = if trimmed.starts_with('.') {
                    command::run(engine, &mut compiler, trimmed, color).await
                } else {
                    run_query(engine, trimmed).await
                };
                let elapsed = started.elapsed();
                match result {
                    Ok(()) => {
                        if trimmed.starts_with(".view") {
                            editor.helper_mut().unwrap().catalog.refresh(engine);
                        }
                        // Metadata/help commands don't need execution summaries.
                        if !trimmed.starts_with('.')
                            || [".view", ".ask", ".plan", ".ask-views", ".plan-views"]
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

struct EditorHelper {
    catalog: completion::Catalog,
    hinter: HistoryHinter,
    color: bool,
}

impl EditorHelper {
    fn new(color: bool) -> Self {
        Self {
            catalog: completion::Catalog::default(),
            hinter: HistoryHinter::new(),
            color,
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
            if &input[leading..end] == ".view"
                && let Some(eq) = input[end..].find('=')
            {
                let sql = end + eq + 1;
                result.push_str(&input[end..sql]);
                result.push_str(&self.highlight_sql(&input[sql..]));
                return result;
            }
            result.push_str(&input[end..]);
            return result;
        }
        self.highlight_sql(input)
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
        let (start, candidates) = self.catalog.complete(line, pos);
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
        if self.color {
            self.hinter.hint(line, pos, ctx)
        } else {
            None
        }
    }
}

impl Validator for EditorHelper {
    fn validate(&self, ctx: &mut ValidationContext<'_>) -> rustyline::Result<ValidationResult> {
        Ok(if lex::complete(ctx.input()) {
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
        Cow::Owned(paint(prompt, "1;36", self.color))
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
            ".view v=SELECT 1;",
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
    async fn views_refresh_completion() {
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
