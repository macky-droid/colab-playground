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

    /// Macro arguments are tokens, not syntax. A literal after a lone `.` is a tuple index;
    /// after `..` it ends a range.
    fn tokens(&mut self, tokens: TokenStream) {
        let mut dots = 0;
        for token in tokens {
            let tuple_index = dots == 1;
            match &token {
                TokenTree::Group(group) => self.tokens(group.stream()),
                TokenTree::Literal(literal) if !tuple_index => match Lit::new(literal.clone()) {
                    Lit::Int(lit) => self.int(&lit),
                    Lit::Float(lit) => self.float(&lit),
                    _ => {}
                },
                _ => {}
            }
            dots = match &token {
                TokenTree::Punct(punct) if punct.as_char() == '.' => dots + 1,
                _ => 0,
            };
        }
    }
}

/// The bare numbers in `source`, each as `file:line: number`.
fn bare_numbers(file: &str, source: &str) -> Vec<String> {
    let ast =
        syn::parse_file(source).unwrap_or_else(|error| panic!("{file} does not parse: {error}"));
    let mut numbers = Numbers {
        file: file.to_owned(),
        hits: Vec::new(),
    };
    numbers.visit_file(&ast);
    numbers.hits
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
        let file = path
            .strip_prefix(root)
            .expect("under the crate")
            .components()
            .map(|part| part.as_os_str().to_string_lossy())
            .collect::<Vec<_>>()
            .join("/");
        hits.extend(bare_numbers(&file, &source));
    }
    assert!(
        hits.is_empty(),
        "UI code uses bare numbers. Name each one as a `const` that says what it means, or take \
         it from the theme (`cx.theme()`); a size the theme has no token for belongs in \
         `ui::metrics`.\n{}",
        hits.join("\n")
    );
}

#[test]
fn in_a_macro_the_ends_of_a_range_are_numbers_and_a_tuple_index_is_not() {
    let found = |body: &str| bare_numbers("probe.rs", &format!("fn probe() {{ {body} }}"));
    assert_eq!(found("vec![0..24];"), ["probe.rs:1: 24"]);
    assert_eq!(found("vec![0..=24];"), ["probe.rs:1: 24"]);
    assert_eq!(found("vec![tuple.2];"), [""; 0]);
    assert_eq!(found("vec![nested.2.3];"), [""; 0]);
}
