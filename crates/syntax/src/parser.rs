use std::cell::Cell;

use crate::ast::{AstNode, SourceFile};
use crate::lexer::{GleamLexer, LexToken};
use crate::token_set::TokenSet;
use crate::SyntaxKind::{self, *};
use crate::{Error, ErrorKind, SyntaxNode};
use rowan::{GreenNode, GreenNodeBuilder, TextRange, TextSize};

const IDENTIFIER: TokenSet = TokenSet::new(&[IDENT, U_IDENT]);

const STMT_RECOVERY: TokenSet =
    TokenSet::new(&[T!["fn"], T!["type"], T!["import"], T!["const"], T!["pub"]]);

const PARAM_LIST_RECOVERY: TokenSet = TokenSet::new(&[T!["->"], T!["("]]).union(STMT_RECOVERY);
const IMPORT_RECOVERY: TokenSet = TokenSet::new(&[T!["as"]]).union(STMT_RECOVERY);
const CONST_RECOVERY: TokenSet = TokenSet::new(&[T!["="]]).union(STMT_RECOVERY);

const TYPE_FIRST: TokenSet = TokenSet::new(&[T!["fn"], T!["#"], IDENT, U_IDENT]);

const CONST_FIRST: TokenSet = TokenSet::new(&[IDENT, T!["#"], T!["["], INTEGER, FLOAT, STRING]);

const EXPR_FIRST: TokenSet = TokenSet::new(&[
    U_IDENT,
    T!["use"],
    T!["-"],
    T!["!"],
    T!["panic"],
    T!["todo"],
    IDENT,
    INTEGER,
    FLOAT,
    STRING,
    T!["#"],
    T!["<<"],
    T!["["],
    T!["{"],
    T!["case"],
    T!["fn"],
    T!["let"],
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

pub fn parse_file(src: &str) -> Parse {
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
        fuel: Cell::new(256),
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
                    FUNCTION | MODULE_CONSTANT => {
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
        self.fuel.set(256);
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
        stmnt_or_tg(p)
    }
    p.finish_node(m, SOURCE_FILE);
}

fn stmnt_or_tg(p: &mut Parser) {
    let m = p.start_node();
    match p.nth(0) {
        T!["if"] => target_group(p),
        _ => statement(p),
    }

    p.finish_node(m, TARGET_GROUP);
}

fn target_group(p: &mut Parser) {
    assert!(p.at(T!["if"]));
    p.expect(T!["if"]);
    let t = p.start_node();
    p.expect(IDENT);
    p.finish_node(t, TARGET);
    if p.at(T!["{"]) {
        statements_block(p);
    }
}

fn statements_block(p: &mut Parser) {
    assert!(p.at(T!["{"]));
    // let m = p.start_node();
    p.expect(T!["{"]);
    while !p.at(T!["}"]) && !p.eof() {
        statement(p);
    }
    p.expect(T!["}"]);
    // p.finish_node(m, STATEMENTS);
}

fn statement(p: &mut Parser) {
    let m = p.start_node();
    let is_pub = p.at(T!["pub"]);
    if is_pub {
        p.expect(T!["pub"]);
    }
    match p.nth(0) {
        T!["const"] => module_const(p, m),
        T!["fn"] => function(p, m),
        T!["import"] => {
            if is_pub {
                p.bump_with_error(ErrorKind::UnexpectedImport);
            } else {
                import(p, m);
            }
        }
        _ => {
            p.bump_with_error(ErrorKind::ExpectedStatement);
            p.finish_node(m, ERROR);
        }
    }
}

fn function(p: &mut Parser, m: MarkOpened) {
    assert!(p.at(T!["fn"]));
    p.expect(T!["fn"]);
    let n = p.start_node();
    p.expect(IDENT);
    p.finish_node(n, NAME);
    if p.at(T!["("]) {
        param_list(p);
    }

    // UX: when user is typing '-' error could be nicer
    if p.eat(T!["->"]) {
        if p.at_any(TYPE_FIRST) {
            type_expr(p);
        } else {
            p.error(ErrorKind::ExpectedType);
        }
    }

    if p.at(T!["{"]) {
        block(p);
    }

    p.finish_node(m, FUNCTION);
}

fn param_list(p: &mut Parser) {
    assert!(p.at(T!["("]));
    let m = p.start_node();
    p.expect(T!["("]);

    while !p.at(T![")"]) && !p.eof() {
        if p.at(IDENT) {
            param(p);
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

fn param(p: &mut Parser) {
    assert!(p.at(IDENT));
    let m = p.start_node();
    if p.nth(1) == IDENT {
        let n = p.start_node();
        p.expect(IDENT);
        p.finish_node(n, LABEL);
    }
    let o = p.start_node();
    p.expect(IDENT);
    p.finish_node(o, NAME);

    if p.at(T![":"]) {
        p.expect(T![":"]);
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
            //   T!["use"] => stmt_use(p)
            _ => {
                if p.at_any(EXPR_FIRST) {
                    stmt_expr(p)
                } else {
                    if p.at_any(STMT_RECOVERY) {
                        break;
                    }
                    p.bump_with_error(ErrorKind::ExpectedStatement);
                }
            }
        }
    }
    p.expect(T!["}"]);
    p.finish_node(m, BLOCK)
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
    let t = p.start_node();
    p.expect(IDENT); //parse pattern
    p.finish_node(t, NAME);
    if p.at(T![":"]) {
        p.expect(T![":"]);
        type_expr(p);
    }

    p.expect(T!["="]);
    expr(p);
    p.finish_node(m, STMT_LET);
}

fn expr(p: &mut Parser) {
    expr_bp(p, 0)
}

fn expr_bp(p: &mut Parser, min_bp: u8) {
    let Some(mut lhs) = expr_unit(p) else {
        return;
    };

    loop {
        match p.nth(0) {
            T!["("] => {
                let m = p.start_node_before(lhs);
                arg_list(p);
                lhs = p.finish_node(m, EXPR_CALL);
            }
            // T!["."] => {
            //     let m = p.start_node_before(lhs);
            //     p.expect(T!["."]);
            //     match p.nth(0) {
            //         U_IDENT | IDENT => {
            //             p.bump();
            //             p.finish_node(m, FIELD_ACCESS);
            //         },
            //         INTEGER => {
            //             p.bump();
            //             p.finish_node(m, TUPLE_INDEX);
            //         }
            //         _ => break
            //     }
            // }
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

        let m = p.start_node_before(lhs);
        p.bump(); // Infix op.
        expr_bp(p, rbp);
        lhs = p.finish_node(m, BINARY_OP);
    }
}

fn expr_unit(p: &mut Parser) -> Option<MarkClosed> {
    let res = match p.nth(0) {
        INTEGER | FLOAT | STRING => {
            let m = p.start_node();
            p.bump();
            p.finish_node(m, LITERAL)
        }
        T!["{"] => block(p),
        IDENT | U_IDENT => {
            let m = p.start_node();
            p.bump();
            p.finish_node(m, NAME_REF)
        }
        T!["#"] => tuple(p),
        _ => return None,
    };
    Some(res)
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
    if p.nth(1) == T![":"] {
        let n = p.start_node();
        p.expect(IDENT);
        p.finish_node(n, LABEL);
    }
    expr(p);
    if !p.at(T![")"]) {
        p.expect(T![","]);
    }
    p.finish_node(m, ARG);
}

fn import(p: &mut Parser, m: MarkOpened) {
    assert!(p.at(T!["import"]));
    p.expect(T!["import"]);

    while p.at_any(TokenSet::new(&[IDENT, U_IDENT])) && !p.eof() {
        let n = p.start_node();
        p.bump();
        p.finish_node(n, PATH);
        if p.at(T!["/"]) {
            p.bump();
        } else {
            break;
        }
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
    p.expect(T!["."]);
    p.expect(T!["{"]);
    while !p.eof() && !p.at(T!["}"]) {
        match p.nth(0) {
            IDENT => as_name(p),
            // U_IDENT => type_name(p) ToDo!
            U_IDENT => as_type_name(p),
            k if IMPORT_RECOVERY.contains(k) => break,
            _ => p.bump_with_error(ErrorKind::ExpectedParameter),
        }
    }
    p.expect(T!["}"]);
}

fn as_name(p: &mut Parser) {
    assert!(p.at(IDENT));
    let m = p.start_node();
    name(p);
    if p.at(T!["as"]) {
        p.expect(T!["as"]);
        let n = p.start_node();
        p.expect(IDENT);
        p.finish_node(n, NAME);
    }
    if !p.at(T!["}"]) {
        p.expect(T![","]);
    }
    p.finish_node(m, UNQUALIFIED_IMPORT);
}

fn as_type_name(p: &mut Parser) {
    assert!(p.at(U_IDENT));
    let m = p.start_node();
    type_name(p);
    if p.at(T!["as"]) {
        p.expect(T!["as"]);
        let n = p.start_node();
        p.expect(U_IDENT);
        p.finish_node(n, NAME);
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

fn type_name(p: &mut Parser) {
    assert!(p.at(U_IDENT));
    let m = p.start_node();
    p.expect(U_IDENT);
    p.finish_node(m, NAME);
}

fn module_const(p: &mut Parser, m: MarkOpened) {
    assert!(p.at(T!["const"]));
    p.bump();
    let n = p.start_node();
    p.expect(IDENT);
    p.finish_node(n, NAME);
    if p.at(T![":"]) {
        p.expect(T![":"]);
        type_expr(p);
    }
    p.expect(T!["="]);
    const_expr(p);
    p.finish_node(m, MODULE_CONSTANT);
}

fn const_expr(p: &mut Parser) {
    match p.nth(0) {
        INTEGER | FLOAT | STRING => {
            let n = p.start_node();
            p.bump();
            p.finish_node(n, LITERAL);
        }
        T!["{"] => {
            block(p);
        }
        IDENT | U_IDENT => {
            let n = p.start_node();
            p.bump();
            p.finish_node(n, NAME_REF);
        }
        T!["#"] => {
            const_tuple(p);
        }
        _ => (),
    };
}

fn const_tuple(p: &mut Parser) -> MarkClosed {
    assert!(p.at(T!["#"]));
    let m = p.start_node();
    p.expect(T!["#"]);
    p.expect(T!["("]);
    while !p.eof() && !p.at(T![")"]) {
        if p.at_any(CONST_FIRST) {
            const_expr(p);
            if !p.at(T![")"]) {
                p.expect(T![","]);
            }
        } else {
            break;
        }
    }
    p.expect(T![")"]);
    p.finish_node(m, CONSTANT_TUPLE)
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

fn type_expr(p: &mut Parser) {
    match p.nth(0) {
        // function
        T!["fn"] => fn_type(p),
        // type variable or constructor type
        IDENT => {
            let m = p.start_node();
            match p.nth(1) {
                T!["."] => {
                    let n = p.start_node();
                    p.expect(IDENT);
                    p.finish_node(n, MODULE_NAME);
                    p.expect(T!["."]);
                    if p.at(U_IDENT) {
                        type_name(p);
                    } else {
                        if !p.at_any(CONST_RECOVERY) {
                            p.bump_with_error(ErrorKind::ExpectedType);
                        } else {
                            p.error(ErrorKind::ExpectedType);
                        }
                    }
                    p.finish_node(m, CONSTRUCTOR_TYPE);
                }
                _ => {
                    p.expect(IDENT);
                    p.finish_node(m, VAR_TYPE);
                }
            }
        }
        // constructor
        U_IDENT => {
            let m = p.start_node();
            type_name(p);
            p.finish_node(m, CONSTRUCTOR_TYPE);
        }
        // tuple
        T!("#") => {
            tuple_type(p);
        }
        _ => {
            p.bump_with_error(ErrorKind::ExpectedType);
        }
    }
}

fn tuple_type(p: &mut Parser) {
    assert!(p.at(T!["#"]));
    let m = p.start_node();
    p.expect(T!["#"]);
    p.expect(T!["("]);
    while !p.eof() && !p.at(T![")"]) {
        if p.at_any(TYPE_FIRST) {
            type_expr(p);
            if !p.at(T![")"]) {
                p.expect(T![","]);
            }
        } else {
            break;
        }
    }
    p.expect(T![")"]);
    p.finish_node(m, TUPLE_TYPE);
}

fn fn_type(p: &mut Parser) {
    assert!(p.at(T!["fn"]));
    let m = p.start_node();
    p.expect(T!["fn"]);
    let n = p.start_node();
    p.expect(T!["("]);
    while !p.at(T![")"]) && !p.eof() {
        if p.at_any(TYPE_FIRST) {
            type_expr(p);
            if !p.at(T![")"]) {
                p.expect(T![","]);
            }
        } else {
            break;
        }
    }
    p.finish_node(n, PARAM_TYPE_LIST);

    p.expect(T![")"]);
    p.expect(T!["->"]);
    if p.at_any(TYPE_FIRST) {
        type_expr(p);
    } else {
        p.error(ErrorKind::ExpectedType);
    }
    p.finish_node(m, FN_TYPE);
}

impl SyntaxKind {
    fn prefix_bp(self) -> Option<u8> {
        Some(match self {
            T!["!"] => 9,
            T!["-"] => 11,
            _ => return None,
        })
    }

    fn infix_bp(self) -> Option<(u8, u8)> {
        Some(match self {
            T!["+"] | T!["-"] => (1, 2),
            T!["*"] | T!["/"] => (3, 4),
            _ => return None,
        })
    }
}
