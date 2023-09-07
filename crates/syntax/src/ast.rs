use crate::SyntaxKind::{self, *};
use crate::{GleamLanguage, SyntaxNode, SyntaxToken};
use rowan::ast::support::{child, children};
use rowan::NodeOrToken;
use smol_str::SmolStr;

pub use rowan::ast::{AstChildren, AstNode};

#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum BinaryOpKind {
    IntAdd,
    IntSub,
    IntMul,
    IntDiv,
    IntMod,
    IntGT,
    IntLT,
    IntGTE,
    IntLTE,
    FloatAdd,
    FloatSub,
    FloatMul,
    FloatDiv,
    FloatGT,
    FloatLT,
    FloatGTE,
    FloatLTE,
    Eq,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum UnaryOpKind {
    Not,
    Negate,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum LiteralKind {
    Int,
    Float,
    String,
    Bool,
}

trait NodeWrapper {
    const KIND: SyntaxKind;
}

macro_rules! enums {
    ($($name:ident { $($variant:ident$(<$gen:ident>)?,)* },)*) => {
        $(
        #[derive(Clone, Debug, PartialEq, Eq, Hash)]
        pub enum $name {
            $($variant($variant$(<$gen>)*),)*
        }

        impl AstNode for $name {
            type Language = GleamLanguage;

            fn can_cast(kind: SyntaxKind) -> bool
                where Self: Sized
            {
                matches!(kind, $(<$variant as NodeWrapper>::KIND)|*)
            }

            fn cast(node: SyntaxNode) -> Option<Self>
            where
                Self: Sized
            {
                match node.kind() {
                    $(<$variant as NodeWrapper>::KIND => Some(Self::$variant$(::<$gen>)*($variant(node))),)*
                    _ => None,
                }
            }

            fn syntax(&self) -> &SyntaxNode {
                match self {
                    $(Self::$variant(e) => &e.0,)*
                }
            }
        }
        )*
    };
}

macro_rules! asts {
    (
        $(
            $kind:ident = $name:ident $([$trait:tt])?
            { $($impl:tt)* },
        )*
    ) => {
        $(
        #[derive(Clone, Debug, PartialEq, Eq, Hash)]
        pub struct $name (SyntaxNode);

        impl $name {
            ast_impl!($($impl)*);
        }

        $(impl $trait for $name {})*

        impl NodeWrapper for $name{
            const KIND: SyntaxKind = SyntaxKind::$kind;
        }

        impl AstNode for $name {
            type Language = GleamLanguage;

            fn can_cast(kind: SyntaxKind) -> bool
                where Self: Sized
            {
                kind == SyntaxKind::$kind
            }

            fn cast(node: SyntaxNode) -> Option<Self>
            where
                Self: Sized
            {
                if node.kind() == SyntaxKind::$kind {
                    Some(Self(node))
                } else {
                    None
                }
            }

            fn syntax(&self) -> &SyntaxNode {
                &self.0
            }
        }
        )*
    };
}

macro_rules! ast_impl {
    () => {};
    ($field:ident: $ast:ident, $($tt:tt)*) => {
        pub fn $field(&self) -> Option<$ast> { child(&self.0) }
        ast_impl!($($tt)*);
    };
    ($field:ident[$k:tt]: $ast:ident, $($tt:tt)*) => {
        pub fn $field(&self) -> Option<$ast> { children(&self.0).nth($k) }
        ast_impl!($($tt)*);
    };
    ($field:ident: [$ast:ident], $($tt:tt)*) => {
        pub fn $field(&self) -> AstChildren<$ast> { children(&self.0) }
        ast_impl!($($tt)*);
    };
    ($field:ident: T![$tok:tt], $($tt:tt)*) => {
        pub fn $field(&self) -> Option<SyntaxToken> {
            token(&self.0, T![$tok])
        }
        ast_impl!($($tt)*);
    };
    ($field:ident[$k:tt]: T![$tok:tt], $($tt:tt)*) => {
        pub fn $field(&self) -> Option<SyntaxToken> {
            self.0
                .children_with_tokens()
                .filter_map(|it| it.into_token())
                .filter(|it| it.kind() == T![$tok])
                .nth($k)
        }
        ast_impl!($($tt)*);
    };
    ($($item:item)*) => {
        $($item)*
    };
}

enums! {
    ModuleStatement {
        ModuleConstant,
        Import,
        Function,
        Adt,
    },
    ConstantExpr {
        Literal,
        ConstantTuple,
        ConstantList,
    },
    StatementExpr {
        StmtLet,
        StmtExpr,
        StmtUse,
    },
    Expr {
        Case,
        BitString,
        Literal,
        Block,
        Variable,
        Lambda,
        BinaryOp,
        Hole,
        Pipe,
        UnaryOp,
        List,
        ExprCall,
        VariantConstructor,
        FieldAccessExpr,
        TupleIndex,
        ExprSpread,
    },
    TypeExpr {
        FnType,
        TupleType,
        TypeNameRef,
        TypeApplication,
    },
    Pattern {
        PatternVariable,
        VariantRef,
        PatternTuple,
        Literal,
        PatternList,
        Hole,
        PatternSpread,
    },
    TypeNameOrName {
        Name,
        TypeName,
    },
}

impl Variable {
    pub fn text(&self) -> Option<SmolStr> {
        self.name().and_then(|n| n.text())
    }
}

impl From<FieldAccessExpr> for Expr {
    fn from(field: FieldAccessExpr) -> Self {
        Expr::FieldAccessExpr(field)
    }
}

impl From<Variable> for Expr {
    fn from(name: Variable) -> Self {
        Expr::Variable(name)
    }
}

impl TypeNameOrName {
    pub fn token(&self) -> Option<SyntaxToken> {
        match self {
            TypeNameOrName::Name(name) => name.token(),
            TypeNameOrName::TypeName(type_name) => type_name.token(),
        }
    }

    pub fn text(&self) -> Option<SmolStr> {
        self.token().map(|t| t.text().into())
    }
}

impl FieldAccessExpr {
    pub fn for_label_name_ref(label: &NameRef) -> Option<FieldAccessExpr> {
        let syn = label.syntax();
        let candidate = syn.parent().and_then(FieldAccessExpr::cast)?;
        if candidate.label().as_ref() == Some(&label) {
            Some(candidate)
        } else {
            None
        }
    }
}

asts! {
    BLOCK = Block {
        expressions: [StatementExpr],
    },
    BIT_STRING = BitString {

    },
    BINARY_OP = BinaryOp {
        lhs: Expr,
        rhs[1]: Expr,

        pub fn op_details(&self) -> Option<(SyntaxToken, BinaryOpKind)> {
            self.syntax().children_with_tokens().find_map(|n| {
                let tok = n.into_token()?;
                let op = match tok.kind() {
                    T!["+"] => BinaryOpKind::IntAdd,
                    T!["-"] => BinaryOpKind::IntSub,
                    T!["*"] => BinaryOpKind::IntMul,
                    T!["/"] => BinaryOpKind::IntDiv,
                    T!["%"] => BinaryOpKind::IntMod,
                    T![">"] => BinaryOpKind::IntGT,
                    T!["<"] => BinaryOpKind::IntLT,
                    T![">="] => BinaryOpKind::IntGTE,
                    T!["<="] => BinaryOpKind::IntLTE,

                    T!["+."] => BinaryOpKind::FloatAdd,
                    T!["-."] => BinaryOpKind::FloatSub,
                    T!["*."] => BinaryOpKind::FloatMul,
                    T!["/."] => BinaryOpKind::FloatDiv,
                    T![">."] => BinaryOpKind::FloatGT,
                    T!["<."] => BinaryOpKind::FloatLT,
                    T![">=."] => BinaryOpKind::FloatGTE,
                    T!["<=."] => BinaryOpKind::FloatGTE,
                    T!["=="] => BinaryOpKind::Eq,
                    _ => return None,
                };
                Some((tok, op))
            })
        }
        pub fn op_token(&self) -> Option<SyntaxToken> {
            self.op_details().map(|t| t.0)
        }
        pub fn op_kind(&self) -> Option<BinaryOpKind> {
            self.op_details().map(|t| t.1)
        }
    },
    PIPE = Pipe {
        lhs: Expr,
        rhs[1]: Expr,
    },
    CONSTANT_LIST = ConstantList {
        elements: [ConstantExpr],
    },
    LITERAL = Literal {
        pub fn token(&self) -> Option<SyntaxToken> {
            self.0.children_with_tokens().find_map(NodeOrToken::into_token)
        }

        pub fn kind(&self) -> Option<LiteralKind> {
            Some(match self.token()?.kind() {
                INTEGER => LiteralKind::Int,
                FLOAT => LiteralKind::Float,
                STRING => LiteralKind::String,
                T!["False"] | T!["True"] => LiteralKind::Bool,
                _ => return None,
            })
        }
    },
    ADT = Adt {
        name: TypeName,
        constructors: [Variant],
    },
    CUSTOM_TYPE_ALIAS = CustomTypeAlias {
        name: TypeName,
        constructors: [Variant],

        pub fn is_public(&self) -> bool {
            self.syntax().children_with_tokens().find(|it| it.kind() == T!["pub"]).is_some()
        }

        pub fn is_opaque(&self) -> bool {
            self.0.children_with_tokens().find(|t| t.kind() == T!["opaque"]).is_some()
        }
    },
    VARIANT = Variant {
        name: Name,
        field_list: ConstructorFieldList,
    },
    VARIANT_CONSTRUCTOR = VariantConstructor {
        name: NameRef,
    },
    CONSTRUCTOR_FIELD_LIST = ConstructorFieldList {
        fields: [ConstructorField],
    },
    CONSTRUCTOR_FIELD = ConstructorField {
        label: Label,
        type_: TypeExpr,
    },
    LAMBDA = Lambda {
        param_list: ParamList,
        return_type: TypeExpr,
        body: Block,
    },
    FUNCTION = Function {
        name: Name,
        param_list: ParamList,
        return_type: TypeExpr,
        body: Block,
    },
    EXPR_CALL = ExprCall {
        func: Expr,
        arguments: ArgList,
    },
    ARG_LIST = ArgList {
        args: [Arg],
    },
    ARG = Arg {
        label: Label,
        value: Expr,
    },
    FIELD_ACCESS = FieldAccessExpr {
        base: Expr,
        label: NameRef,
    },
    TUPLE_INDEX = TupleIndex {
        index: Literal,
        base: Expr,
    },
    IMPORT = Import {
        module_path: ModulePath,
        as_name: Name,
        unqualified: [UnqualifiedImport],
    },
    MODULE_PATH = ModulePath {
        path: [Path],
    },
    VARIABLE = Variable {
        name: NameRef,
    },
    EXPR_SPREAD = ExprSpread {
        expr: Expr,
    },
    SOURCE_FILE = SourceFile {
        statements: [ModuleStatement],
    },
    MODULE_NAME = ModuleName {
        pub fn token(&self) -> SyntaxToken {
            self.0.children_with_tokens().find_map(NodeOrToken::into_token).unwrap()
        }

        pub fn text(&self) -> SmolStr {
            self.token().text().into()
        }
    },
    // Change to body with expression to be able to reuse parser / collecting logic and validate constant during lowering
    MODULE_CONSTANT = ModuleConstant {
        name: Name,
        value: ConstantExpr,
        annotation: TypeExpr,
        pub fn is_public(&self) -> bool {
            self.syntax().children_with_tokens().find(|it| it.kind() == T!["pub"]).is_some()
        }
    },
    NAME = Name {
        pub fn token(&self) -> Option<SyntaxToken> {
            self.0.children_with_tokens().find_map(NodeOrToken::into_token)
        }

        pub fn text(&self) -> Option<SmolStr> {
            self.token().map(|t| t.text().into())
        }
    },
    TYPE_NAME = TypeName {
        pub fn token(&self) -> Option<SyntaxToken> {
            self.0.children_with_tokens().find_map(NodeOrToken::into_token)
        }

        pub fn text(&self) -> Option<SmolStr>{
            self.token().map(|t| t.text().into())
        }
    },
    LABEL = Label {
        pub fn token(&self) -> Option<SyntaxToken> {
            self.0.children_with_tokens().find_map(NodeOrToken::into_token)
        }

        pub fn text(&self) -> Option<SmolStr>{
            self.token().map(|t| t.text().into())
        }
    },
    TARGET = Target {
        pub fn token(&self) -> Option<SyntaxToken> {
            self.0.children_with_tokens().find_map(NodeOrToken::into_token)
        }
    },
    PATH = Path {
        pub fn token(&self) -> Option<SyntaxToken> {
            self.0.children_with_tokens().find_map(NodeOrToken::into_token)
        }
    },
    UNQUALIFIED_IMPORT = UnqualifiedImport {
      name: TypeNameOrName,
      as_name[1]: TypeNameOrName,
    },
    PARAM = Param {
        pattern: AsPattern, // this is a pattern to make name resolution easier
        label: Label,
        ty: TypeExpr,
    },
    PARAM_LIST = ParamList {
        params: [Param],
    },
    UNARY_OP = UnaryOp {
        arg: Expr,

        pub fn op_details(&self) -> Option<(SyntaxToken, UnaryOpKind)> {
            self.syntax().children_with_tokens().find_map(|n| {
                let tok = n.into_token()?;
                let kind = match tok.kind() {
                    T!["!"] => UnaryOpKind::Not,
                    T!["-"] => UnaryOpKind::Negate,
                    _ => return None,
                };
                Some((tok, kind))
            })
        }
        pub fn op_token(&self) -> Option<SyntaxToken> {
            self.op_details().map(|t| t.0)
        }
        pub fn op_kind(&self) -> Option<UnaryOpKind> {
            self.op_details().map(|t| t.1)
        }
    },
    HOLE = Hole {
        pub fn token(&self) -> Option<SyntaxToken> {
            self.0.children_with_tokens().find_map(NodeOrToken::into_token)
        }
    },
    LIST = List {
        elements: [Expr],
    },
    STMT_EXPR = StmtExpr {
        expr: Expr,
    },
    STMT_LET = StmtLet {
        pattern: AsPattern,
        annotation: TypeExpr,
        body: Expr,
    },
    STMT_USE = StmtUse {
        assignments: [UseAssignment],
        expr: Expr,
    },
    USE_ASSIGNMENT = UseAssignment {
        pattern: AsPattern,
        annotation: TypeExpr,
    },
    CONSTANT_TUPLE = ConstantTuple {
        elements: [ConstantExpr],
    },
    TYPE_NAME_REF = TypeNameRef {
        module: ModuleName,
        constructor_name: TypeName,
    },
    TYPE_APPLICATION = TypeApplication {
        type_constructor: TypeNameRef,
        arg_list: TypeArgList,
    },
    TYPE_ARG_LIST = TypeArgList {
        args: [TypeArg],
    },
    TYPE_ARG = TypeArg {
        arg: TypeExpr,
    },
    TUPLE_TYPE = TupleType{
      field_types: [TypeExpr],
    },
    FN_TYPE = FnType {
        param_list: ParamTypeList,
        return_: TypeExpr,
    },
    PARAM_TYPE_LIST = ParamTypeList {
        params: [TypeExpr],
    },
    NAME_REF = NameRef {
        pub fn token(&self) -> Option<SyntaxToken> {
            self.0.children_with_tokens().find_map(NodeOrToken::into_token)
        }

        pub fn text(&self) -> Option<SmolStr>{
            self.token().map(|t| t.text().into())
        }
    },
    CASE = Case {
        subjects: [Expr],
        clauses: [Clause],
    },
    CLAUSE = Clause {
        patterns: [AlternativePattern],
        body: Expr,
    },
    AS_PATTERN = AsPattern {
        pattern: Pattern,
        as_name: Pattern,
    },
    ALTERNATIVE_PATTERN = AlternativePattern {
        patterns: [AsPattern],
    },
    PATTERN_VARIABLE = PatternVariable {
        pub fn token(&self) -> Option<SyntaxToken> {
            self.0.children_with_tokens().find_map(NodeOrToken::into_token)
        }

        pub fn text(&self) -> Option<SmolStr> {
            self.token().map(|t| t.text().into())
        }
    },
    VARIANT_REF = VariantRef {
        module: ModuleName,
        variant: NameRef,
        field_list: VariantRefFieldList,
    },
    VARIANT_REF_FIELD_LIST = VariantRefFieldList {
        fields: [VariantRefField],
    },
    VARIANT_REF_FIELD = VariantRefField {
        field: AsPattern,
    },
    PATTERN_TUPLE = PatternTuple {
        field_patterns: [AsPattern],
    },
    PATTERN_SPREAD = PatternSpread {
       name: Name, 
    },
    PATTERN_LIST = PatternList {
        elements: [AsPattern],
    },
    PATTERN_GUARD = PatternGuard {
        expr: Expr,
    },
}

impl Name {
    pub fn missing() -> SmolStr {
        "[missing]".into()
    }
}

#[cfg(test)]
mod tests {

    use super::*;
    use crate::tests::parse;

    trait HasSyntaxNode {
        fn has_syntax_node(&self) -> &SyntaxNode;
    }

    trait AstTest {
        fn should_eq(&self, expect: &str);
    }

    impl AstTest for SyntaxNode {
        #[track_caller]
        fn should_eq(&self, expect: &str) {
            assert_eq!(self.to_string().trim(), expect);
        }
    }

    impl AstTest for SyntaxToken {
        #[track_caller]
        fn should_eq(&self, expect: &str) {
            assert_eq!(self.to_string(), expect);
        }
    }

    #[test]
    fn apply() {
        let e = parse::<PatternTuple>("fn main() { let #(1,2) = #(1,2) }");
        println!("{:?}", e.field_patterns().next().unwrap().syntax());
        // println!("{:?}", e.statements().next().unwrap().syntax());
    }

    #[test]
    fn assert() {
        let e = crate::parse_module(
            "fn a() { 
                        let Dog(a) = Dog(1) 
                    }",
        );
        for error in e.errors() {
            println!("{}", error);
        }
        println!("{:#?}", e.syntax_node())
    }

    #[test]
    fn pub_const() {
        let e = parse::<ModuleConstant>("pub const a = \"123\"");
        e.name().unwrap().syntax().should_eq("a");
        assert!(e.is_public());
    }

    #[test]
    fn const_tuple() {
        let e = parse::<ConstantTuple>("const a = #(#(2,3),2)");
        let mut iter = e.elements();
        iter.next().unwrap().syntax().should_eq("#(2,3)");
        iter.next().unwrap().syntax().should_eq("2");
        assert!(iter.next().is_none())
    }

    #[test]
    fn module() {
        let e =
            parse::<SourceFile>("@target(erlang)\nconst a = 1 const b = 2 @target(javascript) const c = 3");
        let mut iter = e.statements();
        iter.next()
            .unwrap()
            .syntax()
            .should_eq("@target(erlang)\nconst a = 1");
        assert!(iter.next().is_some());
        assert!(iter.next().is_some());
        assert!(iter.next().is_none());
    }

    #[test]
    fn fn_type_ann() {
        let e = parse::<FnType>("const a: fn(Int, String) -> Cat = 1");
        e.return_().unwrap().syntax().should_eq("Cat");
        let mut iter = e.param_list().unwrap().params();
        iter.next().unwrap().syntax().should_eq("Int");
        iter.next().unwrap().syntax().should_eq("String");
    }
    
    #[test]
    fn pattern_spread() {
        let e = parse::<Pattern>("fn spread() { case [] { [..name] -> name } }");
        e.syntax().should_eq("[..name]");
        match e {
            Pattern::PatternList(list) => {
                match list.elements().next().unwrap().pattern().unwrap() {
                    Pattern::PatternSpread(spread) => spread.name().unwrap().syntax().should_eq("name"),
                    _ => unreachable!()
                }
                
            },
            _ => unreachable!()
        }
    }

    #[test]
    fn type_variant() {
        let e = parse::<Adt>(
            "type Wobbles {
            Alot(name: Int, Int)
            Of(String)
        }",
        );
        e.name().unwrap().syntax().should_eq("Wobbles");
        let mut iter = e.constructors();
        let variant = iter.next().unwrap();
        variant.name().unwrap().syntax().should_eq("Alot");
        let mut fields = variant.field_list().unwrap().fields();
        let first_field = fields.next().unwrap();
        let _ = fields.next().is_some();
        first_field.label().unwrap().syntax().should_eq("name");
        first_field.type_().unwrap().syntax().should_eq("Int");
        let _ = fields.next().is_some();
        let _ = fields.next().is_none();
    }

    #[test]
    fn type_variant_generic() {
        let e = parse::<Adt>(
            "type Wobbles(a,b) {
            Alot(name: Int, Int)
            Of(String)
        }",
        );
        e.name().unwrap().syntax().should_eq("Wobbles");
    }

    #[test]
    fn opaque_type() {
        let e = parse::<CustomTypeAlias>("pub type Bla = Bla");
        e.name().unwrap().syntax().should_eq("Bla");
    }

    #[test]
    fn tuple_type_ann() {
        let e = parse::<TupleType>("const a: #(Int, String) = 1");
        let mut iter = e.field_types();
        iter.next().unwrap().syntax().should_eq("Int");
        iter.next().unwrap().syntax().should_eq("String");
    }

    #[test]
    fn constructor_module_type() {
        let e = parse::<ModuleConstant>("const a: gleam.Int = 1");
        e.annotation().unwrap().syntax().should_eq("gleam.Int")
    }

    #[test]
    fn module_constructor_type() {
        let e = parse::<TypeNameRef>("const a : gleam.Int = 1");
        e.constructor_name().unwrap().syntax().should_eq("Int");
        e.module().unwrap().syntax().should_eq("gleam");
    }

    #[test]
    fn import_module() {
        let e = parse::<Import>("import aa/a");
        let module_path = e.module_path();
        let mut iter = module_path.unwrap().path();
        iter.next().unwrap().syntax().should_eq("aa");
        iter.next().unwrap().syntax().should_eq("a");
        assert!(iter.next().is_none());
    }

    #[test]
    fn import_unqualified() {
        let e = parse::<Import>("import aa/a.{m as a, M as A}");
        let mut iter = e.unqualified();
        let fst = iter.next().unwrap();
        let snd = iter.next().unwrap();

        fst.as_name().unwrap().syntax().should_eq("a");
        fst.name().unwrap().syntax().should_eq("m");
        snd.as_name().unwrap().syntax().should_eq("A");
        snd.name().unwrap().syntax().should_eq("M");
        assert!(iter.next().is_none());
    }

    #[test]
    fn import_qualified_as() {
        let e = parse::<Import>("import aa/a.{m as a, M as A} as e");

        let str = e
            .module_path()
            .unwrap()
            .path()
            .filter_map(|t| Some(format!("{}", t.token()?.text())))
            .collect::<Vec<_>>()
            .join("/");
        assert_eq!(str, "aa/a");
        e.as_name().unwrap().syntax().should_eq("e");
    }

    #[test]
    fn function_parameters() {
        let e = parse::<Function>("fn main(a b: Int) -> fn(Int) -> Int {}");
        e.name().unwrap().syntax().should_eq("main");
        e.return_type()
            .unwrap()
            .syntax()
            .should_eq("fn(Int) -> Int");
        let mut params = e.param_list().unwrap().params();
        let fst = params.next().unwrap();
        fst.label().unwrap().syntax().should_eq("a");
        fst.pattern().unwrap().syntax().should_eq("b");
        fst.ty().unwrap().syntax().should_eq("Int");
        assert!(params.next().is_none())
    }

    #[test]
    fn name() {
        let e = parse::<Param>("fn bla(a b: Int) {}");
        e.pattern().unwrap().syntax().should_eq("b");
    }

    #[test]
    fn type_name() {
        let e = parse::<Variant>(
            "type Wobble(a) {
            Wobble1(a)
        }",
        );
        e.name().unwrap().syntax().should_eq("Wobble1");
    }

    #[test]
    fn constructor_field() {
        let e = parse::<ConstructorField>(
            "type Wobble(a) {
            Wobble1(a: int.Wobbles)
        }",
        );
        e.label().unwrap().syntax().should_eq("a");
        e.type_().unwrap().syntax().should_eq("int.Wobbles")
    }

    #[test]
    fn block() {
        let e = parse::<Block>("fn b() { 1 }");
        let mut exprs = e.expressions();
        exprs.next().unwrap().syntax().should_eq("1")
    }

    #[test]
    fn binary_op() {
        let e = parse::<BinaryOp>("fn b() { 1 * 2 + 3 }");
        e.lhs().unwrap().syntax().should_eq("1 * 2");
        e.rhs().unwrap().syntax().should_eq("3");

        let e2 = parse::<BinaryOp>("fn b() { 1 + 2 * 3 }");
        e2.lhs().unwrap().syntax().should_eq("1");
        e2.rhs().unwrap().syntax().should_eq("2 * 3")
    }

    #[test]
    fn type_application() {
        let e = parse::<Variant>("type Bla { Bla2(#(Int)) }");
        e.field_list()
            .iter()
            .next()
            .unwrap()
            .syntax()
            .should_eq("#(Int)");
    }

    #[test]
    fn unary_op() {
        let e = parse::<UnaryOp>("fn a() { -1 }");
        assert_eq!(e.op_kind(), Some(UnaryOpKind::Negate));
        e.op_token().unwrap().should_eq("-");
        e.arg().unwrap().syntax().should_eq("1");
    }

    #[test]
    fn let_expr() {
        let e = parse::<StmtLet>("fn a() { let name:Int = 1}");
        e.annotation().unwrap().syntax().should_eq("Int");
        e.body().unwrap().syntax().should_eq("1")
    }

    #[test]
    fn call_expr() {
        let e = parse::<StmtExpr>("fn main() { abc(name: 1, 2) }");
        match e.expr().unwrap() {
            Expr::ExprCall(expr) => {
                expr.syntax().should_eq("abc(name: 1, 2)");
                expr.func().unwrap().syntax().should_eq("abc");
                let mut args = expr.arguments().unwrap().args();
                let first = args.next().unwrap();
                first.label().unwrap().syntax().should_eq("name");
                first.value().unwrap().syntax().should_eq("1");
                args.next().unwrap().syntax().should_eq("2");
            }
            _ => panic!(),
        }
    }

    #[test]
    fn literal() {
        let e = parse::<Literal>("fn a() { 1 }");
        assert_eq!(e.kind(), Some(LiteralKind::Int));
    }

    #[test]
    fn case_expr() {
        let e = parse::<Case>("fn a() { case wobble, 1 + 7 { Cat, Dog -> 1 }}");
        let mut subs = e.subjects().into_iter();
        subs.next().unwrap().syntax().should_eq("wobble");
        subs.next().unwrap().syntax().should_eq("1 + 7");
        assert!(subs.next().is_none());
        let mut clauses = e.clauses().into_iter();
        clauses.next().unwrap().syntax().should_eq("Cat, Dog -> 1")
    }

    #[test]
    fn clause() {
        let c = parse::<Clause>(
            "fn a() { 
                    case wobble, 1 + 7 
                    { 
                        Bird | Snake, a -> 2
                        Cat, Dog -> 1 
                    }}",
        );
        c.syntax().should_eq("Bird | Snake, a -> 2");
        c.patterns()
            .next()
            .unwrap()
            .syntax()
            .should_eq("Bird | Snake");
        c.body().unwrap().syntax().should_eq("2");
        let mut pats = c.patterns().into_iter();
        pats.next().unwrap().syntax().should_eq("Bird | Snake");
        pats.next().unwrap().syntax().should_eq("a");
    }

    #[test]
    fn alt_pattern() {
        let a = parse::<AlternativePattern>(
            "fn a() { 
                    case wobble
                    { 
                        Bird | Snake -> 2
                    }}",
        );
        a.syntax().should_eq("Bird | Snake");
    }

    #[test]
    fn bit_string() {
        let _b = parse::<BitString>("fn a() { <<a:size(0)>> <<a:8, rest:bit_string>> }");
    }

    #[test]
    fn pattern() {
        let p = parse::<AlternativePattern>(
            "fn a() { 
                    case wobble, 1 + 7 
                    { 
                        int.Bla(Some(a)), 1 -> 2
                    }}",
        );
        p.syntax().should_eq("int.Bla(Some(a))");
        let pattern = VariantRef::cast(p.patterns().next().unwrap().pattern().unwrap().syntax().clone()).unwrap();
        pattern
            .field_list()
            .unwrap()
            .fields()
            .into_iter()
            .next()
            .unwrap()
            .syntax()
            .should_eq("Some(a)");
        pattern.module().unwrap().syntax().should_eq("int");
    }

    #[test]
    fn use_() {
        let p = parse::<StmtUse>(
            "fn a() { 
                use manager: Int, a <- result.try(
                    start_manager()
                )
            }",
        );
        p.assignments()
            .into_iter()
            .next()
            .unwrap()
            .syntax()
            .should_eq("manager: Int");
    }

    #[test]
    fn field_access() {
        let f = parse::<FieldAccessExpr>("fn wops() { Mogie(name: 1).name}");
        f.label().unwrap().syntax().should_eq("name");
        f.base().unwrap().syntax().should_eq("Mogie(name: 1)");

        let f = parse::<FieldAccessExpr>("fn wops() { base.label}");
        f.label().unwrap().syntax().should_eq("label");
        f.base().unwrap().syntax().should_eq("base")
    }

    #[test]
    fn variant_constructor() {
        let f = parse::<VariantConstructor>("fn fields() { Muddle(name: 5) }");
        // f.args().unwrap().syntax().should_eq("(name: 5)");
        f.name().unwrap().syntax().should_eq("Muddle");
    }

    #[test]
    fn todo_test() {
        let _p = parse::<Block>(
            "pub fn todoo() -> Nil {
            todo
        }",
        );
    }
}
