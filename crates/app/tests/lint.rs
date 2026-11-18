//! Bare numbers in UI code. Clippy has no lint for them, and a number scaling a theme token
//! (`theme.label_width() * 0.75`) gets past the sizes banned in `clippy.toml`.
//!
//! A test binary of its own costs little here: it uses neither gpui nor Ely, so it links neither.

use std::{
    fs,
    path::{Path, PathBuf},
};

use proc_macro2::{TokenStream, TokenTree};
use syn::{
    Lit, LitFloat, LitInt,
    visit::{self, Visit},
};

/// Sorted, so a failure lists its hits in the same order on every OS.
fn sources(dir: &Path) -> Vec<PathBuf> {
    let mut files = Vec::new();
    let mut entries: Vec<PathBuf> = fs::read_dir(dir)
        .unwrap_or_else(|error| panic!("{} is unreadable: {error}", dir.display()))
        .map(|entry| entry.expect("a directory entry").path())
        .collect();
    entries.sort();
    for path in entries {
        if path.is_dir() {
            files.extend(sources(&path));
        } else if path.extension().is_some_and(|ext| ext == "rs") {
            files.push(path);
        }
    }
    files
}

/// Zero and one need no name, and a `const` or `static` item is where a number gets its name.
struct Numbers {
    file: String,
    hits: Vec<String>,
}

impl Numbers {
    fn hit(&mut self, span: proc_macro2::Span, text: String) {
        self.hits
            .push(format!("{}:{}: {text}", self.file, span.start().line));
    }

    fn int(&mut self, lit: &LitInt) {
        if !matches!(lit.base10_parse::<u128>(), Ok(0 | 1)) {
            self.hit(lit.span(), lit.to_string());
        }
    }

    fn float(&mut self, lit: &LitFloat) {
        if !matches!(lit.base10_parse::<f64>(), Ok(value) if value == 0.0 || value == 1.0) {
            self.hit(lit.span(), lit.to_string());
        }
    }

    /// Macro arguments are tokens, not syntax. A literal right after `.` is a tuple index.
    fn tokens(&mut self, tokens: TokenStream) {
        let mut after_dot = false;
        for token in tokens {
            match &token {
                TokenTree::Group(group) => self.tokens(group.stream()),
                TokenTree::Literal(literal) if !after_dot => match Lit::new(literal.clone()) {
                    Lit::Int(lit) => self.int(&lit),
                    Lit::Float(lit) => self.float(&lit),
                    _ => {}
                },
                _ => {}
            }
            after_dot = matches!(&token, TokenTree::Punct(punct) if punct.as_char() == '.');
        }
    }
}

impl<'ast> Visit<'ast> for Numbers {
    fn visit_item_const(&mut self, _: &'ast syn::ItemConst) {}

    fn visit_item_static(&mut self, _: &'ast syn::ItemStatic) {}

    fn visit_impl_item_const(&mut self, _: &'ast syn::ImplItemConst) {}

    fn visit_trait_item_const(&mut self, _: &'ast syn::TraitItemConst) {}

    fn visit_lit_int(&mut self, lit: &'ast LitInt) {
        self.int(lit);
    }

    fn visit_lit_float(&mut self, lit: &'ast LitFloat) {
        self.float(lit);
    }

    fn visit_macro(&mut self, mac: &'ast syn::Macro) {
        self.tokens(mac.tokens.clone());
        visit::visit_macro(self, mac);
    }
}

#[test]
fn ui_code_names_its_numbers() {
    let root = Path::new(env!("CARGO_MANIFEST_DIR"));
    let mut hits = Vec::new();
    for path in sources(&root.join("src/ui")) {
        let source = fs::read_to_string(&path)
            .unwrap_or_else(|error| panic!("{} is unreadable: {error}", path.display()));
        let ast = syn::parse_file(&source)
            .unwrap_or_else(|error| panic!("{} does not parse: {error}", path.display()));
        let file = path
            .strip_prefix(root)
            .expect("under the crate")
            .components()
            .map(|part| part.as_os_str().to_string_lossy())
            .collect::<Vec<_>>()
            .join("/");
        let mut numbers = Numbers {
            file,
            hits: Vec::new(),
        };
        numbers.visit_file(&ast);
        hits.extend(numbers.hits);
    }
    assert!(
        hits.is_empty(),
        "UI code uses bare numbers. Name each one as a `const` that says what it means, or take \
         it from the theme (`cx.theme()`); a size the theme has no token for belongs in \
         `ui::metrics`.\n{}",
        hits.join("\n")
    );
}
