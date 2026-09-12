//! Shared, lossless token positions for editing incomplete SQL.
use datafusion::sql::sqlparser::{
    dialect::GenericDialect,
    tokenizer::{Location, Token, Tokenizer, Whitespace},
};

#[derive(Debug, Clone)]
pub(super) struct Lexeme {
    pub token: Token,
    pub start: usize,
    pub end: usize,
}

impl Lexeme {
    pub fn keyword(&self, word: &str) -> bool {
        matches!(&self.token, Token::Word(w) if w.quote_style.is_none() && w.value.eq_ignore_ascii_case(word))
    }

    pub fn name(&self) -> Option<String> {
        match &self.token {
            Token::Word(w) => Some(if w.quote_style.is_some() {
                w.value.clone()
            } else {
                w.value.to_lowercase()
            }),
            _ => None,
        }
    }

    pub fn whitespace(&self) -> bool {
        matches!(self.token, Token::Whitespace(_))
    }

    pub fn comment(&self) -> bool {
        matches!(
            self.token,
            Token::Whitespace(
                Whitespace::SingleLineComment { .. } | Whitespace::MultiLineComment(_)
            )
        )
    }

    pub fn string(&self) -> bool {
        // Token's Display normalizes literal prefixes; quoted identifiers are Words.
        !matches!(self.token, Token::Word(_) | Token::Whitespace(_))
            && self.token.to_string().contains(['\'', '"'])
            || matches!(
                self.token,
                Token::DollarQuotedString(_) | Token::DoubleQuotedString(_)
            )
    }
}

pub(super) struct Lexed {
    pub tokens: Vec<Lexeme>,
    /// An unfinished quote/comment (or invalid lexical suffix) remains unmodified.
    pub unfinished: Option<usize>,
}

pub(super) fn lex(sql: &str) -> Lexed {
    let mut lines = vec![vec![]];
    for (offset, ch) in sql.char_indices() {
        lines.last_mut().unwrap().push(offset);
        if ch == '\n' {
            lines.push(vec![]);
        }
    }
    lines.last_mut().unwrap().push(sql.len());
    let offset = |loc: Location| {
        lines
            .get(loc.line.saturating_sub(1) as usize)
            .and_then(|line| line.get(loc.column.saturating_sub(1) as usize))
            .copied()
            .unwrap_or(sql.len())
    };
    let mut tokens = Vec::new();
    let result =
        Tokenizer::new(&GenericDialect {}, sql).tokenize_with_location_into_buf(&mut tokens);
    // Tokenizer errors point inside the unfinished token; the last emitted span
    // is the reliable boundary, including for dollar quotes and nested comments.
    let unfinished = result
        .err()
        .map(|_| tokens.last().map_or(0, |token| offset(token.span.end)));
    Lexed {
        tokens: tokens
            .into_iter()
            .map(|token| Lexeme {
                start: offset(token.span.start),
                end: offset(token.span.end),
                token: token.token,
            })
            .collect(),
        unfinished,
    }
}

pub(super) fn complete(input: &str) -> bool {
    let input = input.trim();
    if input.is_empty() || input.starts_with('.') {
        return true;
    }
    let lexed = lex(input);
    lexed.unfinished.is_none()
        && lexed
            .tokens
            .iter()
            .rev()
            .find(|t| !t.whitespace())
            .is_none_or(|t| t.token == Token::SemiColon)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn statement_framing() {
        for sql in [
            "",
            ".view v=SELECT 1",
            "SELECT 1; -- trailing",
            "SELECT 'it''s;'; /* hi */",
            "SELECT $$;$$;",
            "SELECT (;",
            "SELECT 1; SELECT 2;",
            "-- comment",
        ] {
            assert!(complete(sql), "{sql}");
        }
        for sql in [
            "SELECT 1",
            "SELECT ';'",
            "SELECT 'abc;",
            "SELECT 1 -- ;",
            "SELECT 1; /* unfinished",
            "SELECT $tag$;$tag$",
            "SELECT 1; SELECT 2",
        ] {
            assert!(!complete(sql), "{sql}");
        }
    }

    #[test]
    fn spans_preserve_unicode_and_newlines() {
        let sql = "SELECT \"café\",\n '你好';";
        let lexed = lex(sql);
        assert!(lexed.unfinished.is_none());
        assert_eq!(
            lexed
                .tokens
                .iter()
                .map(|t| &sql[t.start..t.end])
                .collect::<String>(),
            sql
        );
    }
}
