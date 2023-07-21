


use crate::def::{ModuleItemData, InternDatabase};
use crate::tests::TestDB;
use crate::{
    DefDatabase,
};
use expect_test::{expect, Expect};
use tracing_test::traced_test;

use super::{Ty, InferenceResult, TyDatabase};

// #[track_caller]
// fn check(src: &str, expect: Expect) {
//     let (db, file) = TestDB::single_file(src).unwrap();
//     let module = db.module(file);
//     let infer = db.infer(file);
//     let ty = infer.ty_for_expr(module.entry_expr());
//     let got = ty.debug().to_string();
//     expect.assert_eq(&got);
// }

// #[track_caller]
// fn check_name(name: &str, src: &str, expect: Expect) {
//     let (db, file) = TestDB::single_file(src).unwrap();
//     let module = db.module(file);
//     let name = module
//         .names()
//         .find(|(_, n)| n.text == name)
//         .expect("Name not found")
//         .0;
//     let infer = db.infer(file);
//     let ty = infer.ty_for_name(name);
//     let got = ty.debug().to_string();
//     expect.assert_eq(&got);
// }

// fn all_types(module: &ModuleItemData, infer: &InferenceResult) -> String {
//     module
//         .functions()
//         .map(|(i, func)| format!("{}: {:?}\n", func, infer.ty_for_name(i)))
//         .collect()
// }

#[track_caller]
fn check_all(src: &str, expect: Expect) {
    let (db, file) = TestDB::single_file(src).unwrap();
    let scope= db.module_scope(file);
    for fun in scope.declarations() {
        match fun.0 {
            crate::def::hir_def::ModuleDefId::FunctionId(fn_id) => {
                let infer = db.infer_function(fn_id);
                
                let got = format!("{:?}", infer);
                expect.assert_eq(&got);
            },
        }
    }
}

// #[track_caller]
// fn check_all_expect(src: &str, _expect_ty: Ty, expect: Expect) {
//     let (db, file) = TestDB::single_file(src).unwrap();
//     let module = db.module(file);
//     let name = module.functions().nth(0).unwrap().1.name;
//     let infer = super::infer::infer(&db, name, file);
//     let got = all_types(&module, &infer.1);
//     expect.assert_eq(&got);
// }

#[traced_test]
#[test] 
fn let_in() {
    check_all("fn bla(a, b) { a }", expect![[r#"
        main: Function { params: [Int, Unknown], return_: Int }
        a: Int
        b: Unknown
    "#]])
}

#[traced_test]
#[test] 
fn use_() {
    check_all("fn bla(a, b, c, d) { a + 1 } fn main(a) { main2(bla) } fn main2(b) { b(1.1) }", expect![[r#"
        main: Function { params: [Unknown], return_: Int }
        a: Unknown
        bla: Function { params: [], return_: Float }
    "#]])
}