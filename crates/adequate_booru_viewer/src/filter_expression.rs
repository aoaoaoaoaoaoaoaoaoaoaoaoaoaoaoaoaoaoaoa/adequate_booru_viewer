use std::fmt::{Display, Formatter};

use crate::model::{BoolGroup, BoolOp, Query, QueryAtom, QueryExpr, TagPattern};

const OR_PRECEDENCE: u8 = 1;
const XOR_PRECEDENCE: u8 = 2;
const AND_PRECEDENCE: u8 = 3;
const NOT_PRECEDENCE: u8 = 4;
const ATOM_PRECEDENCE: u8 = 5;

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct Fault {
    column: usize,
    message: String,
}

impl Fault {
    fn at(source: &str, byte: usize, message: impl Into<String>) -> Self {
        Self {
            column: source[..byte].chars().count() + 1,
            message: message.into(),
        }
    }

    fn structural(message: impl Into<String>) -> Self {
        Self {
            column: 1,
            message: message.into(),
        }
    }
}

impl Display for Fault {
    fn fmt(&self, formatter: &mut Formatter<'_>) -> std::fmt::Result {
        write!(formatter, "column {}: {}", self.column, self.message)
    }
}

impl std::error::Error for Fault {}

pub fn parse(source: &str) -> Result<Query, Fault> {
    if source.trim().is_empty() {
        return Ok(Query::default());
    }
    let mut parser = Parser::new(source)?;
    let root = parser.expression()?;
    if !matches!(parser.peek().kind, TokenKind::End) {
        return Err(parser.fault("expected `AND`, `XOR`, `OR`, or end of expression"));
    }
    Ok(Query::from_root(root))
}

pub fn render(query: &Query) -> Result<String, Fault> {
    if query.is_empty() {
        return Ok(String::new());
    }
    render_expr(query.root()).map(|rendered| rendered.text)
}

#[derive(Clone, Debug)]
struct Rendered {
    text: String,
    precedence: u8,
    group_op: Option<BoolOp>,
}

fn render_expr(expr: &QueryExpr) -> Result<Rendered, Fault> {
    let (negated, core) = expr.denote();
    let mut rendered = match core {
        QueryExpr::Atom { atom } => Rendered {
            text: render_atom(atom),
            precedence: ATOM_PRECEDENCE,
            group_op: None,
        },
        QueryExpr::Group { group } => render_group(group)?,
        QueryExpr::Not { .. } => unreachable!("denote removes every leading NOT"),
    };
    if negated {
        if rendered.precedence < NOT_PRECEDENCE {
            rendered.text = format!("~({})", rendered.text);
        } else {
            rendered.text.insert(0, '~');
        }
        rendered.precedence = NOT_PRECEDENCE;
        rendered.group_op = None;
    }
    Ok(rendered)
}

fn render_group(group: &BoolGroup) -> Result<Rendered, Fault> {
    let precedence = precedence(group.op);
    let mut children = group.children.iter();
    let Some(first) = children.next() else {
        return Err(Fault::structural(
            "an empty Boolean group cannot be represented in a saved filter",
        ));
    };
    if group.children.len() == 1 {
        return render_expr(first);
    }
    let mut rendered = Vec::with_capacity(group.children.len());
    rendered.push(render_child(first, group.op)?);
    for child in children {
        rendered.push(render_child(child, group.op)?);
    }
    Ok(Rendered {
        text: rendered.join(&format!(" {} ", operator(group.op))),
        precedence,
        group_op: Some(group.op),
    })
}

fn render_child(child: &QueryExpr, parent: BoolOp) -> Result<String, Fault> {
    let rendered = render_expr(child)?;
    let parentheses = rendered.precedence < precedence(parent)
        || parent == BoolOp::Xor && rendered.group_op == Some(BoolOp::Xor);
    Ok(if parentheses {
        format!("({})", rendered.text)
    } else {
        rendered.text
    })
}

fn render_atom(atom: &QueryAtom) -> String {
    match atom {
        QueryAtom::Pattern(pattern) => format!("/{}/", pattern.source().replace('/', "\\/")),
        QueryAtom::Tag(_) | QueryAtom::Rating(_) => atom.term(),
    }
}

const fn precedence(op: BoolOp) -> u8 {
    match op {
        BoolOp::Or => OR_PRECEDENCE,
        BoolOp::Xor => XOR_PRECEDENCE,
        BoolOp::And => AND_PRECEDENCE,
    }
}

const fn operator(op: BoolOp) -> &'static str {
    match op {
        BoolOp::And => "AND",
        BoolOp::Or => "OR",
        BoolOp::Xor => "XOR",
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
struct Token {
    kind: TokenKind,
    byte: usize,
}

#[derive(Clone, Debug, Eq, PartialEq)]
enum TokenKind {
    Atom(QueryAtom),
    And,
    Or,
    Xor,
    Not,
    Left,
    Right,
    End,
}

struct Parser<'a> {
    source: &'a str,
    tokens: Vec<Token>,
    cursor: usize,
}

impl<'a> Parser<'a> {
    fn new(source: &'a str) -> Result<Self, Fault> {
        Ok(Self {
            source,
            tokens: lex(source)?,
            cursor: 0,
        })
    }

    fn expression(&mut self) -> Result<QueryExpr, Fault> {
        self.or()
    }

    fn or(&mut self) -> Result<QueryExpr, Fault> {
        let first = self.xor()?;
        self.chain(first, TokenKind::Or, BoolOp::Or, Self::xor)
    }

    fn xor(&mut self) -> Result<QueryExpr, Fault> {
        let first = self.and()?;
        self.chain(first, TokenKind::Xor, BoolOp::Xor, Self::and)
    }

    fn and(&mut self) -> Result<QueryExpr, Fault> {
        let first = self.unary()?;
        self.chain(first, TokenKind::And, BoolOp::And, Self::unary)
    }

    fn chain(
        &mut self,
        first: QueryExpr,
        token: TokenKind,
        op: BoolOp,
        operand: fn(&mut Self) -> Result<QueryExpr, Fault>,
    ) -> Result<QueryExpr, Fault> {
        if self.peek().kind != token {
            return Ok(first);
        }
        let mut children = vec![first];
        while self.peek().kind == token {
            self.advance();
            children.push(operand(self)?);
        }
        Ok(QueryExpr::Group {
            group: BoolGroup { op, children },
        })
    }

    fn unary(&mut self) -> Result<QueryExpr, Fault> {
        if matches!(self.peek().kind, TokenKind::Not) {
            self.advance();
            return Ok(QueryExpr::Not {
                child: Box::new(self.unary()?),
            });
        }
        self.primary()
    }

    fn primary(&mut self) -> Result<QueryExpr, Fault> {
        match self.peek().kind.clone() {
            TokenKind::Atom(atom) => {
                self.advance();
                Ok(QueryExpr::Atom { atom })
            }
            TokenKind::Left => {
                self.advance();
                let expression = self.expression()?;
                if !matches!(self.peek().kind, TokenKind::Right) {
                    return Err(self.fault("expected `)`"));
                }
                self.advance();
                Ok(expression)
            }
            TokenKind::And | TokenKind::Or | TokenKind::Xor => {
                Err(self.fault("expected a tag, regexp, rating, `~`, or `(`"))
            }
            TokenKind::Not | TokenKind::Right | TokenKind::End => {
                Err(self.fault("expected a tag, regexp, rating, `~`, or `(`"))
            }
        }
    }

    fn peek(&self) -> &Token {
        &self.tokens[self.cursor]
    }

    fn advance(&mut self) {
        self.cursor += 1;
    }

    fn fault(&self, message: impl Into<String>) -> Fault {
        Fault::at(self.source, self.peek().byte, message)
    }
}

fn lex(source: &str) -> Result<Vec<Token>, Fault> {
    let mut tokens = Vec::new();
    let mut cursor = 0;
    while cursor < source.len() {
        let Some(character) = source[cursor..].chars().next() else {
            break;
        };
        if character.is_whitespace() {
            cursor += character.len_utf8();
            continue;
        }
        let byte = cursor;
        let kind = match character {
            '~' => {
                cursor += 1;
                TokenKind::Not
            }
            '(' => {
                cursor += 1;
                TokenKind::Left
            }
            ')' => {
                cursor += 1;
                TokenKind::Right
            }
            '/' => {
                let (pattern, end) = lex_pattern(source, cursor)?;
                cursor = end;
                TokenKind::Atom(QueryAtom::Pattern(pattern))
            }
            _ => {
                let end = bare_end(source, cursor);
                let raw = &source[cursor..end];
                cursor = end;
                match raw {
                    "AND" => TokenKind::And,
                    "OR" => TokenKind::Or,
                    "XOR" => TokenKind::Xor,
                    _ if raw.chars().any(char::is_uppercase) => {
                        return Err(Fault::at(
                            source,
                            byte,
                            "tag and rating literals must be lower-case",
                        ));
                    }
                    _ => TokenKind::Atom(QueryAtom::parse(raw).ok_or_else(|| {
                        Fault::at(source, byte, format!("invalid query atom `{raw}`"))
                    })?),
                }
            }
        };
        tokens.push(Token { kind, byte });
    }
    tokens.push(Token {
        kind: TokenKind::End,
        byte: source.len(),
    });
    Ok(tokens)
}

fn bare_end(source: &str, start: usize) -> usize {
    let mut cursor = start;
    let mut parentheses = 0_u32;
    while cursor < source.len() {
        let Some(character) = source[cursor..].chars().next() else {
            break;
        };
        if character.is_whitespace() || character == '~' {
            break;
        }
        match character {
            '(' => parentheses += 1,
            ')' if parentheses == 0 => break,
            ')' => parentheses -= 1,
            _ => {}
        }
        cursor += character.len_utf8();
    }
    cursor
}

fn lex_pattern(source: &str, start: usize) -> Result<(TagPattern, usize), Fault> {
    let mut pattern = String::new();
    let mut cursor = start + 1;
    while cursor < source.len() {
        let Some(character) = source[cursor..].chars().next() else {
            break;
        };
        if character == '/' {
            let end = cursor + 1;
            return TagPattern::forge(&pattern)
                .map(|pattern| (pattern, end))
                .map_err(|error| Fault::at(source, start, error.to_string()));
        }
        if character == '\\' {
            let next = cursor + 1;
            if source[next..].starts_with('/') {
                pattern.push('/');
                cursor = next + 1;
                continue;
            }
        }
        pattern.push(character);
        cursor += character.len_utf8();
    }
    Err(Fault::at(
        source,
        start,
        "regexp literal is missing its closing `/`",
    ))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn boolean_language_round_trips_its_canonical_surface() -> Result<(), Fault> {
        let source = "solo AND ~(blue_eyes OR /(^|_)eyes$/) XOR pokemon_(creature)";
        let query = parse(source)?;
        assert_eq!(render(&query)?, source);
        assert_eq!(parse(&render(&query)?)?, query);
        Ok(())
    }

    #[test]
    fn operators_have_boolean_precedence_and_lowercase_names_remain_tags() -> Result<(), Fault> {
        let query = parse("and OR a AND b XOR c")?;
        assert_eq!(render(&query)?, "and OR a AND b XOR c");
        assert!(parse("a b").is_err());
        assert!(parse("A AND b").is_err());
        Ok(())
    }

    #[test]
    fn nested_exactly_one_groups_keep_their_boundary() -> Result<(), Fault> {
        let query = parse("(a XOR b) XOR c")?;
        assert_eq!(render(&query)?, "(a XOR b) XOR c");
        Ok(())
    }
}
