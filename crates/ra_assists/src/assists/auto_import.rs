use hir::{db::HirDatabase, AsName};
use ra_syntax::{
    ast::{self, AstNode},
    SmolStr, SyntaxElement,
    SyntaxKind::{NAME_REF, USE_ITEM},
    SyntaxNode,
};

use crate::{
    assist_ctx::{ActionBuilder, Assist, AssistCtx},
    auto_import_text_edit, AssistId, ImportsLocator,
};

// Assist: auto_import
//
// If the name is unresolved, provides all possible imports for it.
//
// ```
// fn main() {
//     let map = HashMap<|>::new();
// }
// ```
// ->
// ```
// use std::collections::HashMap;
//
// fn main() {
//     let map = HashMap<|>::new();
// }
// ```
pub(crate) fn auto_import<F: ImportsLocator>(
    ctx: AssistCtx<impl HirDatabase>,
    imports_locator: &mut F,
) -> Option<Assist> {
    let path: ast::Path = ctx.find_node_at_offset()?;
    let module = path.syntax().ancestors().find_map(ast::Module::cast);
    let position = match module.and_then(|it| it.item_list()) {
        Some(item_list) => item_list.syntax().clone(),
        None => {
            let current_file = path.syntax().ancestors().find_map(ast::SourceFile::cast)?;
            current_file.syntax().clone()
        }
    };
    let source_analyzer = ctx.source_analyzer(&position, None);
    let module_with_name_to_import = source_analyzer.module()?;
    let path_to_import = ctx.covering_element().ancestors().find_map(ast::Path::cast)?;
    if source_analyzer.resolve_path(ctx.db, &path_to_import).is_some() {
        return None;
    }

    let name_to_import = &find_applicable_name_ref(ctx.covering_element())?.as_name();
    let proposed_imports = imports_locator
        .find_imports(&name_to_import.to_string())
        .into_iter()
        .filter_map(|module_def| module_with_name_to_import.find_use_path(ctx.db, module_def))
        .filter(|use_path| !use_path.segments.is_empty())
        .take(20)
        .collect::<std::collections::HashSet<_>>();
    if proposed_imports.is_empty() {
        return None;
    }

    ctx.add_assist_group(AssistId("auto_import"), "auto import", || {
        proposed_imports
            .into_iter()
            .map(|import| import_to_action(import.to_string(), &position, &path_to_import))
            .collect()
    })
}

fn find_applicable_name_ref(element: SyntaxElement) -> Option<ast::NameRef> {
    if element.ancestors().find(|ancestor| ancestor.kind() == USE_ITEM).is_some() {
        None
    } else if element.kind() == NAME_REF {
        Some(element.as_node().cloned().and_then(ast::NameRef::cast)?)
    } else {
        let parent = element.parent()?;
        if parent.kind() == NAME_REF {
            Some(ast::NameRef::cast(parent)?)
        } else {
            None
        }
    }
}

fn import_to_action(import: String, position: &SyntaxNode, path: &ast::Path) -> ActionBuilder {
    let mut action_builder = ActionBuilder::default();
    action_builder.label(format!("Import `{}`", &import));
    auto_import_text_edit(
        position,
        &path.syntax().clone(),
        &[SmolStr::new(import)],
        action_builder.text_edit_builder(),
    );
    action_builder
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::helpers::{
        check_assist_with_imports_locator, check_assist_with_imports_locator_not_applicable,
    };
    use hir::Name;

    #[derive(Clone)]
    struct TestImportsLocator<'a> {
        import_path: &'a [Name],
    }

    impl<'a> TestImportsLocator<'a> {
        fn new(import_path: &'a [Name]) -> Self {
            TestImportsLocator { import_path }
        }
    }

    impl<'a> ImportsLocator for TestImportsLocator<'a> {
        fn find_imports(
            &mut self,
            _: hir::InFile<&ast::NameRef>,
            _: hir::Module,
        ) -> Option<Vec<hir::ModPath>> {
            if self.import_path.is_empty() {
                None
            } else {
                Some(vec![hir::ModPath {
                    kind: hir::PathKind::Plain,
                    segments: self.import_path.to_owned(),
                }])
            }
        }
    }

    #[test]
    fn applicable_when_found_an_import() {
        let import_path = &[hir::name::known::std, hir::name::known::ops, hir::name::known::Debug];
        let mut imports_locator = TestImportsLocator::new(import_path);
        check_assist_with_imports_locator(
            auto_import,
            &mut imports_locator,
            "
            fn main() {
            }

            Debug<|>",
            &format!(
                "
            use {};

            fn main() {{
            }}

            Debug<|>",
                import_path
                    .into_iter()
                    .map(|name| name.to_string())
                    .collect::<Vec<String>>()
                    .join("::")
            ),
        );
    }

    #[test]
    fn not_applicable_when_no_imports_found() {
        let mut imports_locator = TestImportsLocator::new(&[]);
        check_assist_with_imports_locator_not_applicable(
            auto_import,
            &mut imports_locator,
            "
            fn main() {
            }

            Debug<|>",
        );
    }

    #[test]
    fn not_applicable_in_import_statements() {
        let import_path = &[hir::name::known::std, hir::name::known::ops, hir::name::known::Debug];
        let mut imports_locator = TestImportsLocator::new(import_path);
        check_assist_with_imports_locator_not_applicable(
            auto_import,
            &mut imports_locator,
            "use Debug<|>;",
        );
    }
}
