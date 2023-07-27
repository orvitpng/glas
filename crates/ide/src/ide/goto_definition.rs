use super::NavigationTarget;
use crate::def::hir_def::{ModuleDefId, VariantLoc};
use crate::def::resolver::resolver_for_toplevel;
use crate::def::resolver_for_expr;
use crate::def::source_analyzer::find_def;
use crate::{DefDatabase, FilePos, InFile, VfsPath};
use smol_str::SmolStr;
use syntax::ast::{self, AstNode};
use syntax::{best_token_at_offset, TextRange, TextSize};

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum GotoDefinitionResult {
    Path(VfsPath),
    Targets(Vec<NavigationTarget>),
}

pub(crate) fn goto_definition(
    db: &dyn DefDatabase,
    FilePos { file_id, pos }: FilePos,
) -> Option<GotoDefinitionResult> {
    let parse = db.parse(file_id).syntax_node();
    let tok = best_token_at_offset(&parse, pos)?;
    // let module_data = db.module_items(file_id);
    // let source_map = db.source_map(file_id);

    tracing::info!(
        "Module name: {:?}",
        db.module_map().module_name_for_file(file_id)
    );
    //If tok.parent is field access or tuple access, it will be necessary to infer type first
    if matches!(
        tok.parent()?.kind(),
        syntax::SyntaxKind::FIELD_ACCESS | syntax::SyntaxKind::TUPLE_INDEX
    ) {
        return None;
    }

    // Resolver is not enough for goto definition, some expressions have to be inferred aswell eg. field access
    if ast::NameRef::can_cast(tok.parent()?.kind()) {
        let expr = ast::Expr::cast(tok.parent()?)?;

        let expr_ptr = InFile {
            file_id: file_id,
            value: &tok.parent()?,
        };

        // dangerous find_map because iterating hashmap has not always same order!
        // ToDo: Make diagnostic when multiple values declared
        // Find resolver based on where cursor is! not depending on luck!
        let resolver = match find_def(db, expr_ptr) {
            Some(ModuleDefId::FunctionId(id)) => {
                let source_map = db.body_source_map(id);
                let expr_ptr = expr_ptr.with_value(&expr);
                resolver_for_expr(db, id, source_map.expr_for_node(expr_ptr)?)
            }
            _ => resolver_for_toplevel(db, file_id),
        };

        tracing::info!("Name_res: {:#?}", resolver);
        // let ResolveResult((name, file_id)) = name_res.get(expr_id)?;

        // let source_map = db.source_map(*file_id);
        let deps = db.dependency_order(file_id);
        tracing::info!("WERE DOING SOMETHING {:?}", deps);
        let name = SmolStr::from(tok.text());

        let targets = resolver.resolve_name(&name).map(|ptr| {
            let (full_range, focus_range, file_id) = match ptr {
                crate::def::resolver::ResolveResult::LocalBinding(pattern) => {
                    let focus_node = db.body_source_map(resolver.body_owner()?.clone())
                        .node_for_pattern(pattern)?
                        .value
                        .syntax_node_ptr();
                    let root = db.parse(file_id).syntax_node();
                    let full_range = focus_node.to_node(&root).parent().unwrap().text_range();
                (
                    full_range,
                    focus_node.text_range(),
                    file_id,
                )}
                ,
                crate::def::resolver::ResolveResult::FunctionId(func_id) => {
                    let func = db.lookup_intern_function(func_id.clone());
                    let full_node = db.module_items(func.file_id)[func.value]
                            .ast_ptr
                            .syntax_node_ptr();
                    let root = db.parse(file_id).syntax_node();
                    let name = ast::Function::cast(full_node.to_node(&root)).unwrap().name().unwrap();
                    (
                        full_node.text_range(),
                        name.token().unwrap().text_range(),
                        func.file_id,
                    )
                }
                crate::def::resolver::ResolveResult::VariantId(variant_id) => {
                    let VariantLoc { value, .. } = db.lookup_intern_variant(variant_id.clone());
                    let full_range = db.module_items(value.file_id)[value.value]
                            .ast_ptr
                            .syntax_node_ptr().text_range();
                    (
                        full_range,
                        full_range,
                        value.file_id,
                    )
                }
            };

            // let full_node = name_node.ancestors().find(|n| {
            //     matches!(
            //         n.kind(),
            //         SyntaxKind::LAMBDA | SyntaxKind::ATTR_PATH_VALUE | SyntaxKind::INHERIT
            //     )
            // })?;
            Some(NavigationTarget {
                file_id,
                focus_range,
                full_range,
            })
        })?;

        return Some(GotoDefinitionResult::Targets(vec![targets?]));
    }
    // let ptr: AstPtr<ast::Literal> = tok.parent_ancestors().find_map(|node| {
    //     match_ast! {
    //         match node {
    //             ast::Variable(n) => Some(AstPtr::new(&n.into())),
    //             ast::Name(n) => Some(AstPtr::new(&n.into())),
    //             ast::Literal(n) => Some(AstPtr::new(&n.into())),
    //             _ => None,
    //         }
    //     }
    // })?;
    Some(GotoDefinitionResult::Targets(vec![NavigationTarget {
        file_id,
        focus_range: TextRange::new(TextSize::from(0), TextSize::from(5)),
        full_range: TextRange::new(TextSize::from(0), TextSize::from(5)),
    }]))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::base::SourceDatabase;
    use crate::tests::TestDB;
    use expect_test::{expect, Expect};
    use tracing_test::traced_test;

    #[track_caller]
    fn check_no(fixture: &str) {
        let (db, f) = TestDB::from_fixture(fixture).unwrap();
        assert_eq!(f.markers().len(), 1, "Missing markers");
        assert_eq!(goto_definition(&db, f[0]), None);
    }

    #[track_caller]
    fn check(fixture: &str, expect: Expect) {
        let (db, f) = TestDB::from_fixture(fixture).unwrap();
        tracing::info!("Fixture {:?}", db.module_map());
        assert_eq!(f.markers().len(), 1, "Missing markers");
        let mut got = match goto_definition(&db, f[0]).expect("No definition") {
            GotoDefinitionResult::Path(path) => format!("file://{}", path.display()),
            GotoDefinitionResult::Targets(targets) => {
                assert!(!targets.is_empty());
                targets
                    .into_iter()
                    .map(|target| {
                        tracing::info!("{:?}", target);
                        assert!(target.full_range.contains_range(target.focus_range));
                        let src = db.file_content(target.file_id);
                        let mut full = src[target.full_range].to_owned();
                        let relative_focus = target.focus_range - target.full_range.start();
                        full.insert(relative_focus.end().into(), '>');
                        full.insert(relative_focus.start().into(), '<');
                        full
                    })
                    .collect::<Vec<_>>()
                    .join("\n")
            }
        };
        // Prettify.
        if got.contains('\n') {
            got += "\n";
        }
        expect.assert_eq(&got);
    }

    #[traced_test]
    #[test]
    fn let_expr() {
        check("fn main(a) { let c = 123 $0c }", expect!["let <c> = 123"]);
        check(r#"
#-test.gleam
fn main() {
    1
}


#-test2.gleam
import test

fn bla() {
    $0main()
}
"#, expect![r#"
fn <main>() {
    1
}
"#]);
    }
}