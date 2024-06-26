use std::cell::Cell;

use crate::ast::{AstNode, SourceFile};
use crate::lexer::{GleamLexer, LexToken};
use crate::token_set::TokenSet;
use crate::SyntaxKind::{self, *};
use crate::{Error, ErrorKind, SyntaxNode};
use rowan::{GreenNode, GreenNodeBuilder, TextRange, TextSize};

const STMT_RECOVERY: TokenSet = TokenSet::new(&[
    T!["fn"],
    T!["type"],
    T!["import"],
    T!["const"],
    T!["pub"],
    T!["if"],
]);
const STMT_EXPR_RECOVERY: TokenSet =
    TokenSet::new(&[T!["let"], T!["use"], T!["}"], T!["{"]]).union(STMT_RECOVERY);

const PARAM_LIST_RECOVERY: TokenSet = TokenSet::new(&[T!["->"], T!["{"]]).union(STMT_RECOVERY);
const GENERIC_PARAM_LIST_RECOVERY: TokenSet =
    TokenSet::new(&[T!["{"], T!["="]]).union(STMT_RECOVERY);
const IMPORT_RECOVERY: TokenSet = TokenSet::new(&[T!["as"]]).union(STMT_RECOVERY);
const PATTERN_RECOVERY: TokenSet =
    TokenSet::new(&[T!["->"], T!["="], T!["}"], T!["{"]]).union(STMT_RECOVERY);

const PATTERN_FIRST: TokenSet = TokenSet::new(&[
    IDENT,
    U_IDENT,
    DISCARD_IDENT,
    INTEGER,
    FLOAT,
    STRING,
    T!["<<"],
    T!["["],
    T!["#"],
    T!["-"],
    T![".."],
]);
const TYPE_FIRST: TokenSet = TokenSet::new(&[T!["fn"], T!["#"], IDENT, U_IDENT, DISCARD_IDENT]);
const EXPR_FIRST: TokenSet = TokenSet::new(&[
    IDENT,
    U_IDENT,
    DISCARD_IDENT,
    T!["-"],
    T!["!"],
    T!["panic"],
    T!["todo"],
    INTEGER,
    FLOAT,
    STRING,
    T!["#"],
    T!["<<"],
    T!["["],
    T!["{"],
    T!["case"],
    T!["fn"],
    T![".."],
]);

#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct Parse {
    green: GreenNode,
    errors: Vec<Error>,
}

impl Parse {
    pub fn green_node(&self) -> GreenNode {
        self.green.clone()
    }

    pub fn root(&self) -> SourceFile {
        SourceFile::cast(self.syntax_node()).unwrap()
    }

    pub fn syntax_node(&self) -> SyntaxNode {
        SyntaxNode::new_root(self.green.clone())
    }

    pub fn errors(&self) -> &[Error] {
        &self.errors
    }
}

pub fn parse_module(src: &str) -> Parse {
    assert!(src.len() < u32::MAX as usize);
    let tokens_raw: Vec<_> = GleamLexer::new(src).collect();
    let tokens = tokens_raw
        .clone()
        .into_iter()
        .filter(|&t| !t.kind.is_trivia())
        .collect();
    let mut p = Parser {
        tokens,
        tokens_raw,
        errors: Vec::new(),
        src,
        pos: 0,
        fuel: Cell::new(1024),
        events: Vec::new(),
    };
    module(&mut p);
    p.build_tree()
}

#[derive(Debug)]
enum Event {
    Open { kind: SyntaxKind },
    Close,
    Advance,
}

struct MarkOpened {
    index: usize,
}

struct MarkClosed {
    index: usize,
}

struct Parser<'i> {
    tokens: Vec<LexToken<'i>>,
    tokens_raw: Vec<LexToken<'i>>,
    pos: usize,
    src: &'i str,
    fuel: Cell<u32>,
    errors: Vec<Error>,
    events: Vec<Event>,
}

// This is very hackish to intersperce whitespace, but it's nice to not have to think about whitespace in the parser
// refactor next time this needs to be changed..
impl<'i> Parser<'i> {
    fn build_tree(self) -> Parse {
        let mut builder = GreenNodeBuilder::default();
        let tokens = self.tokens_raw;
        let mut events = self.events;
        let len = tokens.len();

        events.pop();

        let mut pos = 0;

        macro_rules! n_tokens {
            ($ident: ident) => {
                (pos..len)
                    .take_while(|&it| tokens.get(it).unwrap().kind.$ident())
                    .count()
            };
        }

        let eat_token = |n, builder: &mut GreenNodeBuilder, pos: &mut usize| {
            for _ in 0..n {
                let LexToken { kind, range, .. } = tokens.get(*pos).unwrap();
                builder.token((*kind).into(), &self.src[*range]);
                *pos += 1;
            }
        };

        for event in events {
            match event {
                Event::Open { kind } => match kind {
                    SOURCE_FILE => {
                        builder.start_node(kind.into());

                        let n_ws = n_tokens!(is_module_doc);
                        eat_token(n_ws, &mut builder, &mut pos);
                    }
                    FUNCTION | MODULE_CONSTANT | ADT | VARIANT => {
                        let n_ws = n_tokens!(is_whitespace);
                        eat_token(n_ws, &mut builder, &mut pos);

                        builder.start_node(kind.into());

                        let n_trivias = n_tokens!(is_stmt_doc);
                        eat_token(n_trivias, &mut builder, &mut pos);
                    }
                    _ => {
                        let n_trivias = n_tokens!(is_trivia);
                        eat_token(n_trivias, &mut builder, &mut pos);

                        builder.start_node(kind.into());
                    }
                },
                Event::Close => {
                    builder.finish_node();
                }
                Event::Advance => {
                    let n_trivias = n_tokens!(is_trivia);
                    eat_token(n_trivias + 1, &mut builder, &mut pos);
                }
            }
        }
        let n_trivias = n_tokens!(is_trivia);
        eat_token(n_trivias, &mut builder, &mut pos);
        builder.finish_node();

        Parse {
            green: builder.finish(),
            errors: self.errors,
        }
    }

    fn error(&mut self, kind: ErrorKind) {
        let range = self
            .tokens
            .get(self.pos)
            .map(|&LexToken { range, .. }| range)
            .unwrap_or_else(|| TextRange::empty(TextSize::from(self.src.len() as u32)));
        self.errors.push(Error { range, kind });
    }

    fn start_node(&mut self) -> MarkOpened {
        let mark = MarkOpened {
            index: self.events.len(),
        };
        self.events.push(Event::Open {
            kind: SyntaxKind::ERROR,
        });
        mark
    }

    fn start_node_before(&mut self, m: MarkClosed) -> MarkOpened {
        let mark = MarkOpened { index: m.index };
        self.events.insert(
            m.index,
            Event::Open {
                kind: SyntaxKind::ERROR,
            },
        );
        mark
    }

    fn finish_node(&mut self, m: MarkOpened, kind: SyntaxKind) -> MarkClosed {
        self.events[m.index] = Event::Open { kind };
        self.events.push(Event::Close);
        MarkClosed { index: m.index }
    }

    fn bump(&mut self) {
        assert!(!self.eof());
        self.fuel.set(1024);
        self.events.push(Event::Advance);
        self.pos += 1;
    }

    fn bump_with_error(&mut self, kind: ErrorKind) {
        let m = self.start_node();
        self.error(kind);
        self.bump();
        self.finish_node(m, ERROR);
    }

    fn eof(&self) -> bool {
        self.pos == self.tokens.len()
    }

    /// Ignores whitespace
    fn nth(&self, lookahead: usize) -> SyntaxKind {
        if self.fuel.get() == 0 {
            panic!("parser is stuck")
        }
        self.fuel.set(self.fuel.get() - 1);
        self.tokens
            .get(self.pos + lookahead)
            .map_or(SyntaxKind::EOF, |it| it.kind)
    }

    fn at(&self, kind: SyntaxKind) -> bool {
        self.nth(0) == kind
    }

    fn at_any(&self, kinds: TokenSet) -> bool {
        kinds.contains(self.nth(0))
    }

    fn eat(&mut self, kind: SyntaxKind) -> bool {
        if self.at(kind) {
            self.bump();
            true
        } else {
            false
        }
    }

    fn expect(&mut self, kind: SyntaxKind) {
        if self.eat(kind) {
            return;
        }
        self.error(ErrorKind::ExpectToken(kind));
    }
}

fn module(p: &mut Parser) {
    let m = p.start_node();
    while !p.eof() {
        statement(p)
    }
    p.finish_node(m, SOURCE_FILE);
}

fn statement(p: &mut Parser) {
    let m = p.start_node();
    //parse attribute
    attributes(p);

    let is_pub = p.eat(T!["pub"]);

    match p.nth(0) {
        T!["const"] => module_const(p, m),
        T!["fn"] => {
            function(p, m, false);
        }
        T!["import"] => {
            if is_pub {
                p.error(ErrorKind::UnexpectedImport);
            }
            import(p, m);
        }
        T!["type"] | T!["opaque"] => custom_type(p, m),
        _ => {
            p.error(ErrorKind::ExpectedStatement);
            if !p.eof() {
                p.bump()
            };
            p.finish_node(m, ERROR);
        }
    }
}

fn attributes(p: &mut Parser<'_>) {
    while p.at(T!["@"]) && !p.eof() {
        attribute(p);
    }
}

fn attribute(p: &mut Parser) {
    assert!(p.at(T!["@"]));
    let attr = p.start_node();
    p.expect(T!["@"]);
    match p.nth(0) {
        T!["external"] => external(p, attr),
        IDENT => {
            target(p, attr);
        }
        _ => {
            p.error(ErrorKind::ExpectedAttribute);
            p.finish_node(attr, ERROR);
        }
    }
}

fn external(p: &mut Parser, attr: MarkOpened) {
    assert!(p.at(T!["external"]));
    p.expect(T!["external"]);
    p.expect(T!["("]);
    p.expect(IDENT);
    p.expect(T![","]);
    p.expect(STRING);
    p.expect(T![","]);
    p.expect(STRING);
    p.expect(T![")"]);
    p.finish_node(attr, EXTERNAL_ATTR);
}

fn target(p: &mut Parser, attr: MarkOpened) {
    assert!(p.at(IDENT));
    p.expect(IDENT);
    p.expect(T!["("]);
    p.expect(IDENT);
    p.expect(T![")"]);
    p.finish_node(attr, TARGET_ATTR);
}

fn custom_type(p: &mut Parser, m: MarkOpened) {
    let opaque = p.at(T!["opaque"]);
    if opaque {
        p.expect(T!["opaque"]);
    }
    p.expect(T!["type"]);
    if p.at(U_IDENT) {
        type_name(p);
    } else {
        p.error(ErrorKind::ExpectedIdentifier);
    }
    if p.at(T!["("]) {
        // parse generic args
        let pl = p.start_node();
        p.expect(T!["("]);
        while !p.at(T![")"]) && !p.eof() {
            if p.at(IDENT) {
                let ty = p.start_node();
                let g = p.start_node();
                p.expect(IDENT);
                p.finish_node(g, TYPE_NAME);
                p.finish_node(ty, TYPE_NAME_REF);
                if !p.at(T![")"]) {
                    p.expect(T![","]);
                }
            } else {
                if p.at_any(GENERIC_PARAM_LIST_RECOVERY) {
                    break;
                }
                p.bump_with_error(ErrorKind::ExpectedParameter)
            }
        }
        p.expect(T![")"]);
        p.finish_node(pl, GENERIC_PARAM_LIST);
    }

    match p.nth(0) {
        T!["{"] => {
            p.expect(T!["{"]);
            while !p.at(T!["}"]) && !p.eof() {
                if p.at(U_IDENT) {
                    variant(p);
                } else {
                    if p.at_any(STMT_RECOVERY) {
                        break;
                    }
                    p.bump_with_error(ErrorKind::ExpectedType);
                }
            }
            p.expect(T!["}"]);
            p.finish_node(m, ADT);
        }
        T!["="] => {
            if opaque {
                p.error(ErrorKind::OpaqueAlias)
            }
            p.expect(T!["="]);
            if p.at_any(TYPE_FIRST) && p.nth(1) != IDENT {
                type_expr(p);
            }
            p.finish_node(m, TYPE_ALIAS);
        }
        _ => {
            p.finish_node(m, ADT);
        }
    }
}

fn variant(p: &mut Parser) {
    assert!(p.at(U_IDENT));
    let m = p.start_node();
    let n = p.start_node();
    p.expect(U_IDENT);
    p.finish_node(n, NAME);
    if p.at(T!["("]) {
        let f = p.start_node();
        p.expect(T!["("]);
        while !p.at(T![")"]) && !p.eof() {
            if p.at_any(TYPE_FIRST) {
                variant_field(p);
                if !p.at(T![")"]) {
                    p.expect(T![","]);
                }
            } else {
                if p.at_any(STMT_RECOVERY) {
                    break;
                }
                p.bump_with_error(ErrorKind::ExpectedType);
            }
        }
        p.expect(T![")"]);
        p.finish_node(f, VARIANT_FIELD_LIST);
    }

    p.finish_node(m, VARIANT);
}

fn variant_field(p: &mut Parser) {
    let m = p.start_node();
    if p.nth(1) == T![":"] {
        let n = p.start_node();
        p.expect(IDENT);
        p.finish_node(n, NAME);
        p.expect(T![":"]);
    }
    type_expr(p);
    p.finish_node(m, VARIANT_FIELD);
}

fn function(p: &mut Parser, m: MarkOpened, is_anon: bool) -> MarkClosed {
    assert!(p.at(T!["fn"]));
    p.expect(T!["fn"]);
    if !is_anon {
        if p.at(IDENT) {
            name(p)
        } else {
            p.error(ErrorKind::ExpectedIdentifier)
        }
    }
    if p.at(T!["("]) {
        param_list(p, is_anon);
    } else {
        p.error(ErrorKind::ExpectToken(T!["("]));
    }

    // UX: when user is typing '-' error could be nicer
    if p.eat(T!["->"]) {
        type_expr(p)
    }

    if p.at(T!["{"]) {
        block(p);
    }

    if is_anon {
        return p.finish_node(m, LAMBDA);
    }
    p.finish_node(m, FUNCTION)
}

fn param_list(p: &mut Parser, is_anon: bool) {
    assert!(p.at(T!["("]));
    let m = p.start_node();
    p.expect(T!["("]);

    while !p.at(T![")"]) && !p.eof() {
        if p.at_any(TokenSet::new(&[IDENT, DISCARD_IDENT])) {
            param(p, is_anon);
        } else {
            if p.at_any(PARAM_LIST_RECOVERY) {
                break;
            }
            p.bump_with_error(ErrorKind::ExpectedParameter)
        }
    }
    p.expect(T![")"]);
    p.finish_node(m, PARAM_LIST);
}

fn param(p: &mut Parser, is_anon: bool) {
    assert!(p.at_any(TokenSet::new(&[IDENT, DISCARD_IDENT])));
    let m = p.start_node();
    match p.nth(0) {
        DISCARD_IDENT => {
            let h = p.start_node();
            p.bump();
            p.finish_node(h, HOLE);
        }
        IDENT => {
            if p.nth(1) == IDENT || p.nth(1) == DISCARD_IDENT {
                if is_anon {
                    p.error(ErrorKind::UnexpectedLabel);
                } else {
                    let l = p.start_node();
                    p.bump();
                    p.finish_node(l, LABEL);
                }
            };

            match p.nth(0) {
                IDENT => {
                    let pat = p.start_node();
                    name(p);
                    p.finish_node(pat, PATTERN_VARIABLE);
                }
                DISCARD_IDENT => {
                    let pat = p.start_node();
                    p.bump();
                    p.finish_node(pat, HOLE);
                }
                _ => p.error(ErrorKind::ExpectedIdentifier),
            }
        }
        _ => p.error(ErrorKind::ExpectedIdentifier),
    }
    if p.eat(T![":"]) {
        type_expr(p);
    }
    if !p.at(T![")"]) {
        p.expect(T![","]);
    }

    p.finish_node(m, PARAM);
}

fn block(p: &mut Parser) -> MarkClosed {
    assert!(p.at(T!["{"]));
    let m = p.start_node();
    p.expect(T!["{"]);
    while !p.at(T!["}"]) && !p.eof() {
        match p.nth(0) {
            T!["let"] => stmt_let(p),
            T!["use"] => stmt_use(p),
            _ => {
                if p.at_any(EXPR_FIRST) {
                    stmt_expr(p)
                } else {
                    if p.at_any(STMT_EXPR_RECOVERY) {
                        break;
                    }
                    p.bump_with_error(ErrorKind::ExpectedStatement);
                }
                // p.bump_with_error(ErrorKind::ExpectedStatement)
            }
        }
    }
    p.expect(T!["}"]);
    p.finish_node(m, BLOCK)
}

fn stmt_use(p: &mut Parser) {
    assert!(p.at(T!["use"]));
    let m = p.start_node();
    p.expect(T!["use"]);
    while !p.at(T!["<-"]) && !p.eof() {
        if p.at_any(PATTERN_FIRST) {
            let pat_m = p.start_node();
            pattern(p);
            if p.eat(T![":"]) {
                type_expr(p);
            }
            p.finish_node(pat_m, USE_ASSIGNMENT);
            if !p.at(T!["<-"]) {
                p.expect(T![","]);
            }
        } else {
            if p.at_any(STMT_EXPR_RECOVERY) {
                break;
            }
            p.bump_with_error(ErrorKind::ExpectedIdentifier);
        }
    }
    p.expect(T!["<-"]);
    if p.at_any(EXPR_FIRST) {
        expr(p);
    } else {
        p.error(ErrorKind::ExpectedExpression);
    }
    p.finish_node(m, STMT_USE);
}

fn stmt_expr(p: &mut Parser) {
    let m = p.start_node();
    expr(p);
    p.finish_node(m, STMT_EXPR);
}

fn stmt_let(p: &mut Parser) {
    assert!(p.at(T!["let"]));
    let m = p.start_node();
    p.expect(T!["let"]);
    if p.at(T!["assert"]) {
        p.expect(T!["assert"]);
    }
    pattern(p);
    if p.eat(T![":"]) {
        type_expr(p);
    }

    p.expect(T!["="]);
    if p.at_any(EXPR_FIRST) {
        expr(p);
    } else {
        p.error(ErrorKind::ExpectedExpression);
    }
    p.finish_node(m, STMT_LET);
}

fn expr(p: &mut Parser) {
    expr_bp(p, 0)
}

fn expr_bp(p: &mut Parser, min_bp: u8) {
    // let Some(mut lhs) = expr_unit(p) else {
    //     return;
    // };

    let Some(mut lhs) = (match p.nth(0).prefix_bp() {
        Some(rbp) => {
            let m = p.start_node();
            p.bump(); // Prefix op.
            expr_bp(p, rbp);
            Some(p.finish_node(m, UNARY_OP))
        }
        _ => expr_unit(p),
    }) else {
        return;
    };

    loop {
        match p.nth(0) {
            T!["("] => {
                let m = p.start_node_before(lhs);
                arg_list(p);
                lhs = p.finish_node(m, EXPR_CALL);
            }
            T!["."] => {
                p.expect(T!["."]);
                match p.nth(0) {
                    IDENT | U_IDENT => {
                        let m = p.start_node_before(lhs);
                        name_ref(p);
                        lhs = p.finish_node(m, FIELD_ACCESS);
                    }
                    INTEGER => {
                        let m = p.start_node_before(lhs);
                        let lit = p.start_node();
                        p.bump();
                        p.finish_node(lit, LITERAL);
                        lhs = p.finish_node(m, TUPLE_INDEX);
                    }
                    _ => {
                        let m = p.start_node_before(lhs);
                        p.error(ErrorKind::ExpectedIdentifier);
                        lhs = p.finish_node(m, FIELD_ACCESS);
                        break;
                    }
                }
            }
            _ => break,
        }
    }

    loop {
        let right = p.nth(0);

        let (lbp, rbp) = match right.infix_bp() {
            None => break,
            Some(bps) => bps,
        };
        if lbp == min_bp {
            p.error(ErrorKind::MultipleNoAssoc);
            break;
        }
        if lbp < min_bp {
            break;
        }

        p.bump(); // Infix op.
        if !p.at_any(EXPR_FIRST) {
            break;
        }

        let m = p.start_node_before(lhs);
        expr_bp(p, rbp);

        if right == T!["|>"] {
            lhs = p.finish_node(m, PIPE);
        } else {
            lhs = p.finish_node(m, BINARY_OP);
        }
    }
}

// The cases that match EXPR_FIRST have to consume a token, otherwise the parser might get stuck
fn expr_unit(p: &mut Parser) -> Option<MarkClosed> {
    let res = match p.nth(0) {
        INTEGER | FLOAT | STRING => {
            let m = p.start_node();
            p.bump();
            p.finish_node(m, LITERAL)
        }
        IDENT => {
            let v = p.start_node();
            name_ref(p);
            p.finish_node(v, VARIABLE)
        }
        U_IDENT => {
            let b = p.start_node();
            name_ref(p);
            p.finish_node(b, VARIANT_CONSTRUCTOR)
        }
        DISCARD_IDENT => {
            let m = p.start_node();
            p.bump();
            p.finish_node(m, HOLE)
        }
        T!["<<"] => bit_array(p),
        T!["{"] => block(p),
        T!["#"] => tuple(p),
        T!["["] => list(p),
        T!["case"] => case(p),
        T!["panic"] | T!["todo"] => {
            let m = p.start_node();
            p.bump();
            if p.eat(T!["as"]) {
                expr(p);
            }
            p.finish_node(m, MISSING)
        }
        T!["fn"] => {
            let m = p.start_node();
            function(p, m, true)
        }
        T![".."] => {
            let m = p.start_node();
            p.expect(T![".."]);
            if p.at_any(EXPR_FIRST) {
                expr(p);
            }
            p.finish_node(m, EXPR_SPREAD)
        }
        _ => return None,
    };
    Some(res)
}

// ToDo: Parse bit string correctly
fn bit_array(p: &mut Parser<'_>) -> MarkClosed {
    assert!(p.at(T!["<<"]));
    p.expect(T!["<<"]);
    let m = p.start_node();
    while !p.at(T![">>"]) && !p.eof() {
        p.bump();
        if p.at_any(STMT_RECOVERY) {
            break;
        }
    }
    p.expect(T![">>"]);
    p.finish_node(m, BIT_ARRAY)
}

fn case(p: &mut Parser) -> MarkClosed {
    assert!(p.at(T!["case"]));
    let m = p.start_node();
    p.expect(T!["case"]);
    while !p.eof() {
        if p.at_any(EXPR_FIRST) {
            expr(p);
            if !p.eat(T![","]) {
                break;
            }
        } else {
            if p.at_any(STMT_EXPR_RECOVERY) {
                break;
            }
            p.bump_with_error(ErrorKind::ExpectedExpression);
        }
    }

    if p.eat(T!["{"]) {
        while !p.at(T!["}"]) && !p.eof() {
            if p.at_any(PATTERN_FIRST) {
                clause(p);
            } else {
                if p.at_any(EXPR_FIRST.union(STMT_EXPR_RECOVERY)) {
                    break;
                }
                p.bump_with_error(ErrorKind::ExpectedExpression)
            }
        }
        p.expect(T!["}"]);
    } else {
        p.error(ErrorKind::ExpectToken(T!["{"]));
    }

    p.finish_node(m, CASE)
}

fn clause(p: &mut Parser) {
    let m = p.start_node();
    while !p.eof() {
        if p.at_any(PATTERN_FIRST) {
            alternative_pattern(p);
            if !p.eat(T![","]) {
                break;
            }
        } else {
            if p.at_any(PATTERN_RECOVERY) {
                break;
            }
            p.bump_with_error(ErrorKind::ExpectedExpression)
        }
    }
    if p.at(T!["if"]) {
        // parse guards
        let guard = p.start_node();
        p.expect(T!["if"]);
        expr(p);
        p.finish_node(guard, PATTERN_GUARD);
    }

    p.expect(T!["->"]);
    if p.at_any(EXPR_FIRST) {
        expr(p);
    } else {
        p.error(ErrorKind::ExpectedExpression)
    }

    p.finish_node(m, CLAUSE);
}

fn alternative_pattern(p: &mut Parser) {
    let m = p.start_node();
    while !p.at(T![","]) && !p.eof() {
        if p.at_any(PATTERN_FIRST) {
            pattern(p);

            if !p.eat(T!["|"]) {
                break;
            }
        } else {
            if p.at_any(EXPR_FIRST) {
                break;
            }
            p.bump_with_error(ErrorKind::ExpectedExpression)
        }
    }
    p.finish_node(m, ALTERNATIVE_PATTERN);
}

fn pattern(p: &mut Parser) {
    let pat = match p.nth(0) {
        // variable definition or qualified constructor type
        IDENT => {
            let m = p.start_node();
            // let n = p.start_node();
            p.expect(IDENT);
            if !p.at(T!["."]) {
                let pat = p.finish_node(m, NAME);
                let pat = p.start_node_before(pat);
                p.finish_node(pat, PATTERN_VARIABLE);

                return;
            }

            let n_r = p.finish_node(m, NAME_REF);
            let m_r = p.start_node_before(n_r);
            let m_r = p.finish_node(m_r, MODULE_NAME_REF);
            let t_ref = p.start_node_before(m_r);
            p.expect(T!["."]);
            name_ref(p);
            if p.at(T!["("]) {
                pattern_constructor_arg_list(p);
            }

            p.finish_node(t_ref, VARIANT_REF)
        }
        // constructor
        U_IDENT => {
            let m = p.start_node();
            name_ref(p);
            if p.at(T!["("]) {
                pattern_constructor_arg_list(p);
            }

            p.finish_node(m, VARIANT_REF)
        }
        DISCARD_IDENT => {
            let m = p.start_node();
            p.bump();
            p.finish_node(m, HOLE)
        }
        s @ (INTEGER | FLOAT | STRING) => {
            let m = p.start_node();
            p.bump();
            let mut literal = p.finish_node(m, LITERAL);
            if s == STRING && p.at(T!["<>"]) {
                let concat = p.start_node_before(literal);
                p.expect(T!["<>"]);
                let var = p.start_node();
                pattern(p);
                p.finish_node(var, PATTERN_VARIABLE);
                literal = p.finish_node(concat, PATTERN_CONCAT);
            }
            literal
        }
        T!["<<"] => bit_array(p),
        T!["["] => pattern_list(p),
        T!["-"] | T!["!"] => {
            let u = p.start_node();
            p.bump();
            pattern(p);
            p.finish_node(u, UNARY_OP)
        }
        T!("#") => pattern_tuple(p),
        T![".."] => {
            let spread = p.start_node();
            p.expect(T![".."]);
            if p.at(IDENT) {
                name(p);
            }
            p.finish_node(spread, PATTERN_SPREAD)
        }
        _ => {
            p.error(ErrorKind::ExpectedPattern);
            return;
        }
    };
    if p.eat(T!["as"]) {
        let var = p.start_node();
        if p.at(IDENT) {
            name(p);
        } else {
            p.error(ErrorKind::ExpectedIdentifier);
        }
        p.finish_node(var, PATTERN_VARIABLE);
        let as_pat = p.start_node_before(pat);
        p.finish_node(as_pat, AS_PATTERN);
    }
}

fn pattern_list(p: &mut Parser<'_>) -> MarkClosed {
    assert!(p.at(T!["["]));
    let m = p.start_node();

    p.expect(T!["["]);
    while !p.at(T!["]"]) && !p.eof() {
        if p.at_any(PATTERN_FIRST) {
            pattern(p);
            if !p.at(T!["]"]) {
                p.expect(T![","]);
            }
        } else {
            if p.at_any(STMT_EXPR_RECOVERY) {
                break;
            }
            p.bump_with_error(ErrorKind::ExpectedExpression)
        }
    }
    p.expect(T!["]"]);

    p.finish_node(m, PATTERN_LIST)
}

fn pattern_constructor_arg_list(p: &mut Parser) {
    assert!(p.at(T!["("]));
    let m = p.start_node();

    p.expect(T!["("]);
    while !p.at(T![")"]) && !p.eof() {
        if p.at_any(PATTERN_FIRST) {
            pattern_constructor_arg(p);
        } else {
            if p.at_any(STMT_EXPR_RECOVERY) {
                break;
            }
            p.bump_with_error(ErrorKind::ExpectedExpression)
        }
    }
    p.expect(T![")"]);

    p.finish_node(m, VARIANT_REF_FIELD_LIST);
}

fn pattern_constructor_arg(p: &mut Parser) {
    let m = p.start_node();
    if !p.at_any(PATTERN_FIRST) {
        p.error(ErrorKind::ExpectedExpression);
    }
    if p.nth(1) == T![":"] && p.nth(2) != T![".."] {
        if p.at(IDENT) {
            let n = p.start_node();
            p.bump();
            p.finish_node(n, LABEL);
        } else {
            p.error(ErrorKind::ExpectedIdentifier)
        };
        p.expect(T![":"]);
    }
    pattern(p);
    if !p.at(T![")"]) {
        p.expect(T![","]);
    }
    p.finish_node(m, VARIANT_REF_FIELD);
}

fn pattern_tuple(p: &mut Parser) -> MarkClosed {
    assert!(p.at(T!["#"]));
    let m = p.start_node();
    p.expect(T!["#"]);
    p.expect(T!["("]);
    while !p.eof() && !p.at(T![")"]) {
        if p.at_any(PATTERN_FIRST) {
            pattern(p);
            if !p.at(T![")"]) {
                p.expect(T![","]);
            }
        } else {
            if p.at_any(PATTERN_FIRST) {
                break;
            }
            p.bump_with_error(ErrorKind::ExpectedExpression)
        }
    }
    p.expect(T![")"]);
    p.finish_node(m, PATTERN_TUPLE)
}

fn list(p: &mut Parser) -> MarkClosed {
    assert!(p.at(T!["["]));
    let m = p.start_node();

    p.expect(T!["["]);
    while !p.at(T!["]"]) && !p.eof() {
        if p.at_any(TokenSet::new(&[T![".."]]).union(EXPR_FIRST)) {
            expr(p);
            if !p.at(T!["]"]) {
                p.expect(T![","]);
            }
        } else {
            break;
        }
    }

    p.expect(T!["]"]);
    p.finish_node(m, LIST)
}

fn arg_list(p: &mut Parser) {
    assert!(p.at(T!["("]));
    let m = p.start_node();

    p.expect(T!["("]);
    while !p.at(T![")"]) && !p.eof() {
        if p.at_any(EXPR_FIRST) {
            arg(p);
        } else {
            break;
        }
    }

    p.expect(T![")"]);
    p.finish_node(m, ARG_LIST);
}

fn arg(p: &mut Parser) {
    let m = p.start_node();

    if p.nth(1) == T![":"] && p.nth(2) != T![".."] {
        if p.at(IDENT) {
            let n = p.start_node();
            p.bump();
            p.finish_node(n, LABEL);
        } else {
            p.error(ErrorKind::ExpectedIdentifier)
        }
        p.expect(T![":"]);
    }
    if !p.at_any(EXPR_FIRST) {
        p.error(ErrorKind::ExpectedExpression);
    }
    expr(p);

    p.finish_node(m, ARG);

    if !p.at(T![")"]) {
        p.expect(T![","]);
    }
}

fn import(p: &mut Parser, m: MarkOpened) {
    assert!(p.at(T!["import"]));
    p.expect(T!["import"]);
    let mut parsed_ident = false;

    let module_path = p.start_node();
    while !p.at_any(STMT_RECOVERY) && !p.at(T!["."]) && !p.eof() {
        parsed_ident = true;
        let n = p.start_node();
        if !p.eat(IDENT) {
            p.bump_with_error(ErrorKind::ExpectedIdentifier);
        }
        p.finish_node(n, PATH);
        if p.at(T!["/"]) {
            parsed_ident = false;
            p.bump();
        } else {
            break;
        }
    }
    p.finish_node(module_path, MODULE_PATH);

    if !parsed_ident {
        p.error(ErrorKind::ExpectedIdentifier);
    }

    if p.at(T!["."]) {
        unqualified_imports(p);
    }

    if p.at(T!["as"]) {
        p.bump();
        let n = p.start_node();
        p.expect(IDENT);
        p.finish_node(n, NAME);
    }

    p.finish_node(m, IMPORT);
}

fn unqualified_imports(p: &mut Parser) {
    assert!(p.at(T!["."]));
    p.eat(T!["."]);
    p.expect(T!["{"]);
    while !p.eof() && !p.at(T!["}"]) {
        match p.nth(0) {
            IDENT => as_name(p),
            // U_IDENT => type_name(p) ToDo!
            U_IDENT => as_name(p),
            T!["type"] => as_type_name(p),
            k if IMPORT_RECOVERY.contains(k) => break,
            _ => p.bump_with_error(ErrorKind::ExpectedParameter),
        }
    }
    p.expect(T!["}"]);
}

fn as_name(p: &mut Parser) {
    const AS_NAME_TOKENS: TokenSet = TokenSet::new(&[U_IDENT, IDENT]);
    assert!(p.at_any(TokenSet::new(&[U_IDENT, IDENT])));
    let m = p.start_node();
    let name = p.start_node();
    p.bump();
    p.finish_node(name, NAME);
    if p.at(T!["as"]) {
        p.expect(T!["as"]);
        let n = p.start_node();
        if p.at_any(AS_NAME_TOKENS) {
            p.bump();
        } else {
            p.error(ErrorKind::ExpectedIdentifier);
        }
        p.finish_node(n, NAME);
    }
    if !p.at(T!["}"]) {
        p.expect(T![","]);
    }
    p.finish_node(m, UNQUALIFIED_IMPORT);
}

fn as_type_name(p: &mut Parser) {
    assert!(p.at(T!["type"]));
    let m = p.start_node();
    p.eat(T!["type"]);
    if p.at(U_IDENT) {
        type_name(p);
    } else {
        p.error(ErrorKind::ExpectedType)
    }
    if p.at(T!["as"]) {
        p.expect(T!["as"]);
        if p.at(U_IDENT) {
            type_name(p);
        } else {
            p.error(ErrorKind::ExpectedType)
        }
    }
    if !p.at(T!["}"]) {
        p.expect(T![","]);
    }
    p.finish_node(m, UNQUALIFIED_IMPORT);
}

fn name(p: &mut Parser) {
    assert!(p.at(IDENT));
    let m = p.start_node();
    p.expect(IDENT);
    p.finish_node(m, NAME);
}

fn type_name(p: &mut Parser) -> MarkClosed {
    assert!(p.at(U_IDENT));
    let m = p.start_node();
    p.expect(U_IDENT);
    p.finish_node(m, TYPE_NAME)
}

fn type_name_ref(p: &mut Parser) -> MarkClosed {
    assert!(p.at(U_IDENT));
    let m = p.start_node();
    let t = p.start_node();
    p.expect(U_IDENT);
    p.finish_node(t, TYPE_NAME);
    p.finish_node(m, TYPE_NAME_REF)
}

fn name_ref(p: &mut Parser) {
    if p.at_any(TokenSet::new(&[IDENT, U_IDENT])) {
        let n = p.start_node();
        p.bump();
        p.finish_node(n, NAME_REF);
        return;
    }
    p.error(ErrorKind::ExpectedIdentifier);
}

fn module_const(p: &mut Parser, m: MarkOpened) {
    assert!(p.at(T!["const"]));
    p.bump();
    let n = p.start_node();
    p.expect(IDENT);
    p.finish_node(n, NAME);
    if p.eat(T![":"]) {
        type_expr(p);
    }
    p.expect(T!["="]);
    expr(p);
    p.finish_node(m, MODULE_CONSTANT);
}

fn tuple(p: &mut Parser) -> MarkClosed {
    assert!(p.at(T!["#"]));
    let m = p.start_node();
    p.expect(T!["#"]);
    p.expect(T!["("]);
    while !p.eof() && !p.at(T![")"]) {
        if p.at_any(EXPR_FIRST) {
            expr(p);
            if !p.at(T![")"]) {
                p.expect(T![","]);
            }
        } else {
            break;
        }
    }
    p.expect(T![")"]);
    p.finish_node(m, TUPLE)
}

fn type_arg_list(p: &mut Parser) {
    assert!(p.at(T!["("]));
    let m = p.start_node();

    p.expect(T!["("]);
    while !p.at(T![")"]) && !p.eof() {
        // Lookahead to not continue parsing if a function definition follows.
        // e.g. type Wobble =
        //      fn main() {}
        if p.at_any(TYPE_FIRST) && p.nth(1) != IDENT {
            type_arg(p);
        } else {
            break;
        }
    }
    p.expect(T![")"]);

    p.finish_node(m, TYPE_ARG_LIST);
}

fn type_arg(p: &mut Parser) {
    let m = p.start_node();
    type_expr(p);
    if !p.at(T![")"]) {
        p.expect(T![","]);
    }
    p.finish_node(m, TYPE_ARG);
}

fn type_expr(p: &mut Parser) {
    let mut type_application = false;
    let res = match p.nth(0) {
        T!["fn"] => fn_type(p),
        // type variable or constructor type
        IDENT => {
            let m = p.start_node();
            p.expect(IDENT);
            if !p.at(T!["."]) {
                let ty_name = p.finish_node(m, TYPE_NAME);
                let type_name = p.start_node_before(ty_name);
                p.finish_node(type_name, TYPE_NAME_REF);
                return;
            }
            let m = p.finish_node(m, NAME);
            let n = p.start_node_before(m);
            type_application = true;
            p.expect(T!["."]);
            if p.at(U_IDENT) {
                type_name(p);
            } else {
                p.error(ErrorKind::ExpectedType)
            }
            p.finish_node(n, TYPE_NAME_REF)
        }
        // constructor
        U_IDENT => {
            type_application = true;
            type_name_ref(p)
        }
        DISCARD_IDENT => {
            let m = p.start_node();
            p.expect(DISCARD_IDENT);
            p.finish_node(m, HOLE)
        }
        // tuple
        T!("#") => tuple_type(p),
        _ => {
            p.error(ErrorKind::ExpectedType);
            return;
            // p.bump_with_error(ErrorKind::ExpectedType);
        }
    };
    if !type_application {
        return;
    }

    if p.nth(0) == T!["("] {
        let m = p.start_node_before(res);
        type_arg_list(p);
        p.finish_node(m, TYPE_APPLICATION);
    }
}

fn tuple_type(p: &mut Parser) -> MarkClosed {
    assert!(p.at(T!["#"]));
    let m = p.start_node();
    p.expect(T!["#"]);
    p.expect(T!["("]);
    while !p.eof() && !p.at(T![")"]) {
        if p.at_any(TYPE_FIRST) && p.nth(1) != IDENT {
            type_expr(p);
            if !p.at(T![")"]) {
                p.expect(T![","]);
            }
        } else {
            break;
        }
    }
    p.expect(T![")"]);
    p.finish_node(m, TUPLE_TYPE)
}

fn fn_type(p: &mut Parser) -> MarkClosed {
    assert!(p.at(T!["fn"]));
    let m = p.start_node();
    p.expect(T!["fn"]);
    if p.at(T!["("]) {
        let n = p.start_node();
        p.bump();
        while !p.at(T![")"]) && !p.eof() {
            if p.at_any(TYPE_FIRST) && p.nth(1) != IDENT {
                type_expr(p);
                if !p.at(T![")"]) {
                    p.expect(T![","]);
                }
            } else {
                break;
            }
        }
        p.finish_node(n, PARAM_TYPE_LIST);
    } else {
        p.error(ErrorKind::ExpectToken(T!["("]));
    };

    p.expect(T![")"]);
    p.expect(T!["->"]);
    type_expr(p);
    p.finish_node(m, FN_TYPE)
}

impl SyntaxKind {
    fn prefix_bp(self) -> Option<u8> {
        Some(match self {
            T!["!"] => 17,
            T!["-"] => 18,
            _ => return None,
        })
    }

    fn infix_bp(self) -> Option<(u8, u8)> {
        Some(match self {
            T!["||"] => (1, 2),
            T!["&&"] => (3, 4),
            T!["=="] | T!["!="] => (5, 6),
            T!["<"]
            | T!["<="]
            | T!["<."]
            | T!["<=."]
            | T![">"]
            | T![">="]
            | T![">."]
            | T![">=."] => (7, 8),
            T!["<>"] => (9, 10),
            T!["|>"] => (11, 12),
            T!["+"] | T!["-"] | T!["+."] | T!["-."] => (13, 14),
            T!["*"] | T!["/"] | T!["*."] | T!["/."] | T!["%"] => (15, 16),
            _ => return None,
        })
    }
}
