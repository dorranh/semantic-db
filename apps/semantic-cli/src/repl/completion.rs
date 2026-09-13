//! Best-effort name resolution for incomplete queries. Never plans or executes SQL.
use std::collections::{BTreeMap, BTreeSet};

use datafusion::sql::sqlparser::{
    ast::{Expr, Ident, SelectItem, SelectItemQualifiedWildcardKind, SetExpr, Statement},
    dialect::GenericDialect,
    keywords::{RESERVED_FOR_IDENTIFIER, RESERVED_FOR_TABLE_ALIAS},
    parser::Parser,
    tokenizer::Token,
};
use semantic_engine::Engine;

use super::lex::{Lexeme, lex};

pub(super) const COMMANDS: &[&str] = &[
    ".help",
    ".tables",
    ".schema",
    ".view",
    ".ask",
    ".plan",
    ".ask-views",
    ".plan-views",
    ".cache-status",
    ".cache-refresh",
    ".cache-invalidate",
    ".cache-bypass",
    ".quit",
    ".exit",
];
const KEYWORDS: &[&str] = &[
    "SELECT",
    "FROM",
    "WHERE",
    "JOIN",
    "LEFT JOIN",
    "RIGHT JOIN",
    "INNER JOIN",
    "FULL JOIN",
    "CROSS JOIN",
    "ON",
    "USING",
    "AS",
    "WITH",
    "DISTINCT",
    "AND",
    "OR",
    "NOT",
    "NULL",
    "IS",
    "IN",
    "BETWEEN",
    "LIKE",
    "GROUP BY",
    "ORDER BY",
    "HAVING",
    "LIMIT",
    "OFFSET",
    "ASC",
    "DESC",
    "UNION",
    "ALL",
    "CASE",
    "WHEN",
    "THEN",
    "ELSE",
    "END",
    "EXISTS",
    "EXPLAIN",
    "VALUES",
    "TRUE",
    "FALSE",
];

type Relations = BTreeMap<String, Vec<String>>;

#[derive(Default)]
pub(super) struct Catalog {
    relations: Relations,
}

impl Catalog {
    pub fn refresh(&mut self, engine: &Engine) {
        self.relations = engine
            .catalog()
            .relations()
            .map(|r| {
                (
                    r.name.clone(),
                    r.schema.fields().iter().map(|f| f.name().clone()).collect(),
                )
            })
            .collect();
    }

    pub fn complete(&self, input: &str, pos: usize) -> (usize, Vec<String>) {
        if !input.is_char_boundary(pos) {
            return (pos, vec![]);
        }
        let trimmed = input.trim_start();
        let leading = input.len() - trimmed.len();
        if pos < leading {
            return (pos, vec![]);
        }
        if trimmed.starts_with('.') {
            let end = trimmed.find(char::is_whitespace).unwrap_or(trimmed.len()) + leading;
            if pos <= end {
                return (
                    leading,
                    filter(
                        COMMANDS.iter().map(|s| s.to_string()),
                        &input[leading.min(pos)..pos],
                    ),
                );
            }
            match &input[leading..end] {
                ".schema" | ".cache-refresh" => {
                    let start = end + input[end..].len() - input[end..].trim_start().len();
                    if pos < start {
                        return (pos, vec![]);
                    }
                    return (
                        start,
                        filter(self.relations.keys().cloned(), &input[start..pos]),
                    );
                }
                ".cache-bypass" => {
                    let start = end + input[end..].len() - input[end..].trim_start().len();
                    if pos < start {
                        return (pos, vec![]);
                    }
                    return (
                        start,
                        filter(["on".to_owned(), "off".to_owned()], &input[start..pos]),
                    );
                }
                ".view" => {
                    if let Some(equal) = input[end..].find('=') {
                        let start = end + equal + 1;
                        if pos >= start {
                            let (offset, items) = self.sql(&input[start..], pos - start);
                            return (start + offset, items);
                        }
                    }
                }
                _ => {}
            }
            return (pos, vec![]);
        }
        self.sql(input, pos)
    }

    fn sql(&self, sql: &str, pos: usize) -> (usize, Vec<String>) {
        let mut lexed = lex(sql);
        if let Some(tail) = lexed.unfinished.filter(|&i| i < pos)
            && sql[tail..].starts_with('"')
            && !sql[tail + 1..pos].contains('"')
        {
            // Repair only the editing copy so FROM bindings after an unfinished
            // quoted prefix remain visible. Never change submitted SQL.
            let mut repaired = sql.to_owned();
            repaired.insert(pos, '"');
            lexed = lex(&repaired);
            for token in &mut lexed.tokens {
                token.start -= usize::from(token.start > pos);
                token.end -= usize::from(token.end > pos);
            }
            lexed.unfinished = lexed.unfinished.map(|n| n - usize::from(n > pos));
        }
        if lexed.tokens.iter().any(|t| {
            t.start < pos
                && (pos < t.end
                    || (pos == t.end
                        && t.comment()
                        && !sql[t.start..t.end].ends_with('\n')
                        && !sql[t.start..t.end].ends_with("*/")))
                && (t.comment() || t.string())
        }) {
            return (pos, vec![]);
        }

        let mut start = pos;
        let mut quoted = false;
        if let Some(t) = lexed
            .tokens
            .iter()
            .find(|t| t.start < pos && pos <= t.end && matches!(t.token, Token::Word(_)))
        {
            start = t.start;
            quoted = matches!(&t.token, Token::Word(w) if w.quote_style.is_some());
        } else if let Some(tail) = lexed.unfinished.filter(|&i| i < pos) {
            // A partially typed quoted identifier is still completable.
            if sql[tail..].starts_with('"') && !sql[tail + 1..pos].contains('"') {
                start = tail;
                quoted = true;
            } else {
                return (pos, vec![]);
            }
        }
        let prefix = &sql[start..pos];
        let tokens: Vec<_> = lexed
            .tokens
            .into_iter()
            .filter(|t| !t.whitespace())
            .collect();
        let mut analyzer = Analyzer {
            tokens: &tokens,
            catalog: &self.relations,
            scopes: vec![],
        };
        analyzer.query(0, tokens.len(), &Relations::new(), &Bindings::default(), 0);
        let scope = analyzer
            .scopes
            .iter()
            .filter(|s| s.start <= pos && pos <= s.end)
            .min_by_key(|s| s.end - s.start);
        let before: Vec<_> = tokens.iter().filter(|t| t.end <= start).collect();
        if before.last().is_some_and(|t| t.token == Token::Period) {
            let name = before.iter().rev().nth(1).and_then(|t| t.name());
            let columns = name
                .as_ref()
                .and_then(|name| scope.and_then(|s| s.sources.get(name)));
            return (
                start,
                filter_identifiers(columns.into_iter().flatten().cloned(), prefix, quoted),
            );
        }
        let relations = scope.map(|s| &s.ctes);
        let relation_position = before
            .last()
            .is_some_and(|t| t.keyword("FROM") || t.keyword("JOIN"))
            || (before.last().is_some_and(|t| t.token == Token::Comma)
                && scope.is_some_and(|s| s.from_start <= start && start <= s.from_end));
        let mut items: BTreeSet<String> = self.relations.keys().cloned().collect();
        if let Some(ctes) = relations {
            items.extend(ctes.keys().cloned());
        }
        if relation_position {
            return (start, filter_identifiers(items, prefix, quoted));
        }
        let mut candidates = filter_identifiers(items, prefix, quoted);
        if !quoted {
            candidates.extend(filter(KEYWORDS.iter().map(|s| s.to_string()), prefix));
        }
        if let Some(scope) = scope.filter(|s| s.reliable) {
            let mut counts = BTreeMap::<&str, usize>::new();
            for cols in scope.unqualified.values() {
                for col in cols.iter().collect::<BTreeSet<_>>() {
                    *counts.entry(col).or_default() += 1;
                }
            }
            for (alias, columns) in &scope.unqualified {
                for col in columns {
                    if matches_prefix(col, prefix, quoted) {
                        candidates.push(if counts[col.as_str()] > 1 {
                            format!("{}.{}", quote(alias), quote(col))
                        } else {
                            quote(col)
                        });
                    }
                }
            }
        }
        candidates.sort();
        candidates.dedup();
        (start, candidates)
    }
}

fn filter(items: impl IntoIterator<Item = String>, prefix: &str) -> Vec<String> {
    items
        .into_iter()
        .filter(|s| s.to_lowercase().starts_with(&prefix.to_lowercase()))
        .collect()
}

fn matches_prefix(name: &str, prefix: &str, quoted: bool) -> bool {
    if quoted {
        let prefix = prefix
            .strip_prefix('"')
            .unwrap_or(prefix)
            .trim_end_matches('"')
            .replace("\"\"", "\"");
        name.starts_with(&prefix)
    } else {
        name.to_lowercase().starts_with(&prefix.to_lowercase())
    }
}

fn filter_identifiers(
    items: impl IntoIterator<Item = String>,
    prefix: &str,
    quoted: bool,
) -> Vec<String> {
    let mut seen = BTreeSet::new();
    items
        .into_iter()
        .filter(|s| matches_prefix(s, prefix, quoted))
        .map(|s| {
            if quoted {
                format!("\"{}\"", s.replace('"', "\"\""))
            } else {
                quote(&s)
            }
        })
        .filter(|s| seen.insert(s.clone()))
        .collect()
}

fn ident_name(ident: &Ident) -> String {
    if ident.quote_style.is_some() {
        ident.value.clone()
    } else {
        ident.value.to_lowercase()
    }
}

fn projection_item(text: &str) -> Option<SelectItem> {
    let mut statements = Parser::parse_sql(&GenericDialect {}, &format!("SELECT {text}")).ok()?;
    if statements.len() != 1 {
        return None;
    }
    let Statement::Query(query) = statements.remove(0) else {
        return None;
    };
    let SetExpr::Select(mut select) = *query.body else {
        return None;
    };
    if select.projection.len() != 1 || !select.from.is_empty() {
        return None;
    }
    Some(select.projection.remove(0))
}

fn quote(name: &str) -> String {
    if name == name.to_lowercase()
        && matches!(projection_item(name), Some(SelectItem::UnnamedExpr(Expr::Identifier(ident))) if ident.quote_style.is_none() && ident.value == name)
        && !lex(name).tokens.iter().any(
            |t| matches!(&t.token, Token::Word(w) if RESERVED_FOR_TABLE_ALIAS.contains(&w.keyword)),
        )
    {
        name.to_owned()
    } else {
        format!("\"{}\"", name.replace('"', "\"\""))
    }
}

/// Replace the rest of an identifier too when completing in its middle.
pub(super) fn replacement_end(input: &str, start: usize, pos: usize) -> usize {
    if input[start..].starts_with('.') {
        return input[start..]
            .find(char::is_whitespace)
            .map_or(input.len(), |end| start + end);
    }
    lex(input)
        .tokens
        .iter()
        .find(|t| t.start == start && pos <= t.end && matches!(t.token, Token::Word(_)))
        .map_or(pos, |t| t.end)
}

#[derive(Default)]
struct Bindings {
    qualified: Relations,
    unqualified: Relations,
}

struct Scope {
    start: usize,
    end: usize,
    from_start: usize,
    from_end: usize,
    sources: Relations,
    unqualified: Relations,
    ctes: Relations,
    reliable: bool,
}

struct Analyzer<'a> {
    tokens: &'a [Lexeme],
    catalog: &'a Relations,
    scopes: Vec<Scope>,
}

impl Analyzer<'_> {
    fn is(&self, i: usize, word: &str) -> bool {
        self.tokens.get(i).is_some_and(|t| t.keyword(word))
    }
    fn token(&self, i: usize, token: Token) -> bool {
        self.tokens.get(i).is_some_and(|t| t.token == token)
    }
    fn name(&self, i: usize) -> Option<String> {
        self.tokens.get(i).and_then(Lexeme::name)
    }

    fn close(&self, open: usize, end: usize) -> usize {
        let mut depth = 0;
        for i in open..end {
            if self.token(i, Token::LParen) {
                depth += 1;
            }
            if self.token(i, Token::RParen) {
                depth -= 1;
                if depth == 0 {
                    return i;
                }
            }
        }
        end
    }

    fn top(&self, start: usize, end: usize) -> Vec<usize> {
        let mut result = vec![];
        let mut i = start;
        while i < end {
            result.push(i);
            i = if self.token(i, Token::LParen) {
                self.close(i, end).saturating_add(1)
            } else {
                i + 1
            };
        }
        result
    }

    fn query(
        &mut self,
        start: usize,
        end: usize,
        inherited: &Relations,
        outer: &Bindings,
        depth: usize,
    ) -> Vec<String> {
        // Completion must remain bounded even while editing deeply nested input.
        if start >= end || depth > 32 {
            return vec![];
        }
        let mut ctes = inherited.clone();
        let mut i = start;
        if self.is(i, "EXPLAIN") {
            i += 1;
        }
        if self.is(i, "WITH") {
            i += 1;
            if self.is(i, "RECURSIVE") {
                i += 1;
            }
            loop {
                let Some(name) = self.name(i) else {
                    break;
                };
                i += 1;
                let mut explicit = None;
                if self.token(i, Token::LParen) {
                    let close = self.close(i, end);
                    explicit = Some(
                        (i + 1..close)
                            .filter_map(|n| self.name(n))
                            .collect::<Vec<_>>(),
                    );
                    i = close + 1;
                }
                if !self.is(i, "AS") {
                    break;
                }
                i += 1;
                if self.is(i, "NOT") {
                    i += 1;
                }
                if self.is(i, "MATERIALIZED") {
                    i += 1;
                }
                if !self.token(i, Token::LParen) {
                    break;
                }
                let close = self.close(i, end);
                // Explicit recursive CTE columns can be known before its body.
                if let Some(cols) = &explicit {
                    ctes.insert(name.clone(), cols.clone());
                }
                let cols = self.query(i + 1, close, &ctes, outer, depth + 1);
                ctes.insert(name, explicit.unwrap_or(cols));
                i = close + 1;
                if !self.token(i, Token::Comma) {
                    break;
                }
                i += 1;
            }
        }
        let top = self.top(i, end);
        // Each set-operation branch has independent FROM bindings.
        if let Some(&split) = top
            .iter()
            .find(|&&n| self.is(n, "UNION") || self.is(n, "INTERSECT") || self.is(n, "EXCEPT"))
        {
            let cols = self.query(i, split, &ctes, outer, depth + 1);
            let next = split
                + 1
                + usize::from(self.is(split + 1, "ALL") || self.is(split + 1, "DISTINCT"));
            self.query(next, end, &ctes, outer, depth + 1);
            return cols;
        }
        let Some(&select) = top.iter().find(|&&n| self.is(n, "SELECT")) else {
            return vec![];
        };
        let from = top.iter().copied().find(|&n| self.is(n, "FROM"));
        let from_end = from
            .map(|f| {
                top.iter()
                    .copied()
                    .find(|&n| n > f && self.clause(n))
                    .unwrap_or(end)
            })
            .unwrap_or(end);
        let mut local = Relations::new();
        let mut reliable = true;
        let mut derived = BTreeSet::new();
        if let Some(from) = from {
            let mut n = from + 1;
            let mut expect = true;
            while n < from_end {
                if self.is(n, "JOIN") || self.token(n, Token::Comma) {
                    expect = true;
                    n += 1;
                    continue;
                }
                if !expect {
                    n = if self.token(n, Token::LParen) {
                        self.close(n, from_end) + 1
                    } else {
                        n + 1
                    };
                    continue;
                }
                let (base, mut columns, mut next) = if self.token(n, Token::LParen) {
                    let close = self.close(n, from_end);
                    derived.insert(n);
                    (
                        None,
                        self.query(n + 1, close, &ctes, &Bindings::default(), depth + 1),
                        close + 1,
                    )
                } else if let Some(name) = self.name(n) {
                    let columns = ctes.get(&name).or_else(|| self.catalog.get(&name));
                    if columns.is_none() {
                        reliable = false;
                    }
                    (Some(name), columns.cloned().unwrap_or_default(), n + 1)
                } else {
                    reliable = false;
                    break;
                };
                if self.token(next, Token::Period) || self.token(next, Token::LParen) {
                    // Qualified catalog paths and table functions are not catalog relations.
                    reliable = false;
                }
                let explicit_alias = self.is(next, "AS");
                if explicit_alias {
                    next += 1;
                }
                let alias = if next < from_end && (explicit_alias || self.alias(next)) {
                    let alias = self.name(next);
                    next += usize::from(alias.is_some());
                    alias
                } else {
                    None
                };
                if alias.is_some() && self.token(next, Token::LParen) {
                    let close = self.close(next, from_end);
                    columns = (next + 1..close).filter_map(|n| self.name(n)).collect();
                    next = close + 1;
                }
                if let Some(name) = alias.or(base) {
                    local.insert(name, columns);
                } else {
                    reliable = false;
                }
                n = next;
                expect = false;
            }
            if expect {
                reliable = false;
            }
        }
        let mut sources = outer.qualified.clone();
        sources.extend(local.clone());
        let local_columns: BTreeSet<_> = local.values().flatten().collect();
        let mut unqualified: Relations = outer
            .unqualified
            .iter()
            .filter(|(alias, _)| !local.contains_key(*alias))
            .map(|(alias, cols)| {
                (
                    alias.clone(),
                    cols.iter()
                        .filter(|c| !local_columns.contains(c))
                        .cloned()
                        .collect(),
                )
            })
            .collect();
        unqualified.extend(local.clone());
        let bindings = Bindings {
            qualified: sources,
            unqualified,
        };
        // Inspect expression subqueries after all local bindings are available.
        let mut n = select + 1;
        while n < end {
            if self.token(n, Token::LParen) {
                let close = self.close(n, end);
                if !derived.contains(&n) && (self.is(n + 1, "SELECT") || self.is(n + 1, "WITH")) {
                    self.query(n + 1, close, &ctes, &bindings, depth + 1);
                    n = close + 1;
                    continue;
                }
                if derived.contains(&n) {
                    n = close + 1;
                    continue;
                }
            }
            n += 1;
        }
        let projection_end = from.unwrap_or_else(|| {
            top.iter()
                .copied()
                .find(|&n| n > select && self.clause(n))
                .unwrap_or(end)
        });
        let output = self.projection(select + 1, projection_end, &local);
        self.scopes.push(Scope {
            start: self.tokens[start].start,
            end: self.tokens.get(end).map_or(usize::MAX, |t| t.start),
            from_start: from.map_or(usize::MAX, |f| self.tokens[f].end),
            from_end: self.tokens.get(from_end).map_or(usize::MAX, |t| t.start),
            sources: bindings.qualified,
            unqualified: bindings.unqualified,
            ctes,
            reliable,
        });
        output
    }

    fn clause(&self, i: usize) -> bool {
        [
            "WHERE",
            "GROUP",
            "ORDER",
            "HAVING",
            "LIMIT",
            "OFFSET",
            "QUALIFY",
            "WINDOW",
            "UNION",
            "INTERSECT",
            "EXCEPT",
        ]
        .iter()
        .any(|s| self.is(i, s))
            || self.token(i, Token::SemiColon)
    }

    fn alias(&self, i: usize) -> bool {
        matches!(&self.tokens[i].token, Token::Word(w) if w.quote_style.is_some() || !RESERVED_FOR_TABLE_ALIAS.contains(&w.keyword) && !RESERVED_FOR_IDENTIFIER.contains(&w.keyword))
    }

    fn projection(&self, start: usize, end: usize, sources: &Relations) -> Vec<String> {
        let mut result = vec![];
        let mut begin = start + usize::from(self.is(start, "DISTINCT") || self.is(start, "ALL"));
        let mut boundaries: Vec<_> = self
            .top(begin, end)
            .into_iter()
            .filter(|&n| self.token(n, Token::Comma))
            .collect();
        boundaries.push(end);
        for stop in boundaries {
            let text = self.tokens[begin.min(stop)..stop]
                .iter()
                .map(|t| t.token.to_string())
                .collect::<Vec<_>>()
                .join(" ");
            if let Some(item) = projection_item(&text) {
                match item {
                    SelectItem::ExprWithAlias { alias, .. } => result.push(ident_name(&alias)),
                    SelectItem::UnnamedExpr(Expr::Identifier(ident)) => {
                        let col = ident_name(&ident);
                        if sources.values().any(|cols| cols.contains(&col)) {
                            result.push(col);
                        }
                    }
                    SelectItem::UnnamedExpr(Expr::CompoundIdentifier(names))
                        if names.len() == 2 =>
                    {
                        let col = ident_name(&names[1]);
                        if sources
                            .get(&ident_name(&names[0]))
                            .is_some_and(|cols| cols.contains(&col))
                        {
                            result.push(col);
                        }
                    }
                    SelectItem::Wildcard(options) if options.to_string().is_empty() => {
                        result.extend(sources.values().flatten().cloned())
                    }
                    SelectItem::QualifiedWildcard(
                        SelectItemQualifiedWildcardKind::ObjectName(name),
                        options,
                    ) if name.0.len() == 1 && options.to_string().is_empty() => {
                        if let Some(cols) = name.0[0]
                            .as_ident()
                            .and_then(|ident| sources.get(&ident_name(ident)))
                        {
                            result.extend(cols.clone());
                        }
                    }
                    _ => {} // An expression without a declared name is not a column suggestion.
                }
            }
            begin = stop + 1;
        }
        result
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn catalog() -> Catalog {
        Catalog {
            relations: BTreeMap::from([
                (
                    "wells".into(),
                    vec![
                        "id".into(),
                        "depth".into(),
                        "café".into(),
                        "Display Name".into(),
                    ],
                ),
                ("basins".into(), vec!["id".into(), "name".into()]),
            ]),
        }
    }

    fn suggestions(input: &str) -> Vec<String> {
        let pos = input.find('|').unwrap();
        let sql = input.replacen('|', "", 1);
        catalog().complete(&sql, pos).1
    }

    #[test]
    fn aliases_full_buffer_and_joins() {
        assert_eq!(suggestions("SELECT w.de| FROM wells AS w"), ["depth"]);
        assert_eq!(
            suggestions("SELECT b.| FROM wells w JOIN basins b ON w.id=b.id"),
            ["id", "name"]
        );
        let items = suggestions("SELECT i| FROM wells w JOIN basins b ON w.id=b.id");
        assert!(items.contains(&"w.id".into()) && items.contains(&"b.id".into()));
        assert!(!items.contains(&"id".into()));
        assert_eq!(suggestions("SELECT * FROM we|"), ["wells"]);
    }

    #[test]
    fn ctes_and_derived_columns() {
        assert_eq!(
            suggestions(
                "WITH x AS (SELECT id, depth AS d, count(*) AS total FROM wells) SELECT x.| FROM x"
            ),
            ["id", "d", "total"]
        );
        assert_eq!(
            suggestions("WITH x(a,b) AS (SELECT id, depth FROM wells) SELECT x.| FROM x"),
            ["a", "b"]
        );
        assert_eq!(
            suggestions(
                "WITH x AS (SELECT * FROM wells), y AS (SELECT x.id FROM x) SELECT y.| FROM y"
            ),
            ["id"]
        );
        assert_eq!(
            suggestions("SELECT q.| FROM (SELECT w.id, w.depth d, 1+2 FROM wells w) AS q"),
            ["id", "d"]
        );
    }

    #[test]
    fn nesting_shadowing_and_correlation() {
        assert_eq!(
            suggestions("SELECT * FROM wells w WHERE EXISTS (SELECT w.| FROM basins w)"),
            ["id", "name"]
        );
        assert_eq!(
            suggestions("SELECT * FROM wells w WHERE EXISTS (SELECT w.de| FROM basins b)"),
            ["depth"]
        );
        assert_eq!(
            suggestions("SELECT q.| FROM wells w WHERE EXISTS (SELECT * FROM basins q)"),
            Vec::<String>::new()
        );
        assert_eq!(
            suggestions("SELECT * FROM wells w UNION SELECT w.| FROM basins w"),
            ["id", "name"]
        );
    }

    #[test]
    fn quoted_unicode_incomplete_and_commands() {
        assert_eq!(
            suggestions("SELECT \"W\".\"Display| Name\" FROM wells \"W\""),
            ["\"Display Name\""]
        );
        assert_eq!(suggestions("SELECT w.ca| FROM wells w"), ["café"]);
        assert_eq!(
            suggestions("SELECT w.| FROM wells w WHERE ("),
            ["id", "depth", "café", "\"Display Name\""]
        );
        assert!(suggestions("SELECT 'hello |'").is_empty());
        assert!(suggestions("SELECT 1 -- w.|").is_empty());
        assert!(suggestions("SELECT 'unfinished |").is_empty());
        assert!(suggestions(".ask show we|").is_empty());
        assert_eq!(suggestions(".schema we|"), ["wells"]);
        assert_eq!(suggestions(".view v=SELECT w.de| FROM wells w"), ["depth"]);
        assert!(!suggestions("SELECT de| FROM missing").contains(&"depth".into()));
    }

    #[test]
    fn projection_names_and_lexical_scope_are_conservative() {
        assert_eq!(
            suggestions(
                "WITH x AS (SELECT id + depth, id AND depth, lower('x') label, depth * 2 AS doubled FROM wells) SELECT x.| FROM x"
            ),
            ["label", "doubled"]
        );
        assert_eq!(
            suggestions("SELECT * FROM wells w WHERE EXISTS (SELECT i| FROM basins b)"),
            ["IN", "INNER JOIN", "IS", "id"]
        );
        assert_eq!(
            suggestions(
                "WITH x AS (SELECT id FROM wells) SELECT * FROM x WHERE EXISTS (WITH x AS (SELECT name FROM basins) SELECT x.| FROM x)"
            ),
            ["name"]
        );
        assert_eq!(
            suggestions("SELECT w.\"Display| FROM wells w"),
            ["\"Display Name\""]
        );
        assert_eq!(
            suggestions("SELECT * FROM wells W WHERE w.DE| > 0"),
            ["depth"]
        );
        assert_eq!(quote("null"), "\"null\"");
        assert_eq!(quote("select"), "\"select\"");
        assert_eq!(quote("id"), "id");
    }

    #[test]
    fn replacement_boundaries_and_arbitrary_incomplete_input() {
        let sql = "SELECT w.depth FROM wells w";
        assert_eq!(replacement_end(sql, 9, 11), 14);
        let sql = "SELECT w.\"Display Name\" FROM wells w";
        assert_eq!(replacement_end(sql, 9, 13), 23);
        assert_eq!(replacement_end(".ask-views hello", 0, 4), 10);
        // Every character boundary is a valid editing position, even before or
        // inside incomplete CTEs, comments, and quoted names.
        for sql in [
            " WITH x(a) AS (SELECT id FROM wells) SELECT x.a FROM x",
            "SELECT 'hi' /* comment */",
            "SELECT ( SELECT w.id FROM wells w",
            ".view v=SELECT w.café FROM wells w",
            "  .schema wells",
            "SELECT \"unfinished",
        ] {
            for pos in sql.char_indices().map(|(i, _)| i).chain([sql.len()]) {
                let (start, _) = catalog().complete(sql, pos);
                assert!(
                    start <= pos && sql.is_char_boundary(start),
                    "{sql} at {pos}"
                );
            }
        }
    }
}
