use proc_macro2::{Span,
                  TokenStream,
                  TokenTree};
use std::path::{Path,
                PathBuf};
use syn::{Attribute,
          Item,
          Meta,
          Token,
          UseTree,
          punctuated::Punctuated,
          spanned::Spanned,
          visit::{self,
                  Visit}};

const HOST_MACROS: &[&str] = &[
    "include",
    "include_str",
    "include_bytes",
    "env",
    "option_env",
    "print",
    "println",
    "eprint",
    "eprintln",
    "dbg",
    "asm",
    "global_asm",
    "macro_rules",
];
const FILE_METHODS: &[&str] = &[
    "absolute",
    "exists",
    "try_exists",
    "is_file",
    "is_dir",
    "is_symlink",
    "metadata",
    "symlink_metadata",
    "canonicalize",
    "read_dir",
    "read_link",
];

fn name(ident: &proc_macro2::Ident) -> String { ident.to_string().trim_start_matches("r#").to_owned() }

// Unknown platform/feature predicates remain enabled; only definitely test-only
// items are excluded, so this checks production code for every target.
fn without_tests(meta: &Meta) -> Option<bool> {
    match meta {
        | Meta::Path(path) if path.is_ident("test") => Some(false),
        | Meta::List(list) => {
            let args = list.parse_args_with(Punctuated::<Meta, Token![,]>::parse_terminated).ok()?;
            let values: Vec<_> = args.iter().map(without_tests).collect();
            if list.path.is_ident("not") && values.len() == 1 {
                values[0].map(|value| !value)
            } else if list.path.is_ident("all") {
                if values.contains(&Some(false)) {
                    Some(false)
                } else if values.iter().all(|value| *value == Some(true)) {
                    Some(true)
                } else {
                    None
                }
            } else if list.path.is_ident("any") {
                if values.contains(&Some(true)) {
                    Some(true)
                } else if values.iter().all(|value| *value == Some(false)) {
                    Some(false)
                } else {
                    None
                }
            } else {
                None
            }
        },
        | _ => None,
    }
}

fn test_only(attrs: &[Attribute]) -> bool {
    attrs.iter().any(|attr| {
        attr.path().is_ident("test") || (attr.path().is_ident("cfg") && attr.parse_args::<Meta>().ok().is_some_and(|meta| without_tests(&meta) == Some(false)))
    })
}

fn forbidden(path: &[String]) -> bool {
    if path.first().is_none_or(|root| root != "std" && root != "core") {
        return false;
    }
    if path.last().is_some_and(|name| HOST_MACROS.contains(&name.as_str())) {
        return true;
    }
    if matches!(path.get(1).map(String::as_str), Some("arch" | "intrinsics")) {
        return true;
    }
    if path[0] == "core" {
        return path.len() == 1 || matches!(path.get(1).map(String::as_str), Some("self" | "*"));
    }
    match path.get(1).map(String::as_str) {
        // Root/module imports could hide forbidden paths behind an alias or glob.
        | None | Some("self" | "*") => true,
        | Some("fs" | "net" | "process" | "env" | "io" | "os" | "thread") => true,
        | Some("time") => matches!(
            path.get(2).map(String::as_str),
            None | Some("self" | "*" | "SystemTime" | "Instant" | "UNIX_EPOCH")
        ),
        | _ => false,
    }
}

struct Guard<'a> {
    file: PathBuf,
    module_dir: PathBuf,
    errors: &'a mut Vec<String>,
}

impl Guard<'_> {
    fn reject(&mut self, span: Span, reason: &str) {
        let at = span.start();
        self.errors.push(format!("{}:{}:{}: {reason}", self.file.display(), at.line, at.column + 1));
    }

    fn path(&mut self, path: &[String], span: Span) {
        if path.len() > 1 && path.last().is_some_and(|name| FILE_METHODS.contains(&name.as_str())) {
            self.reject(span, "filesystem query methods are forbidden in pure code");
        }
        if forbidden(path) {
            self.reject(span, &format!("pure code must not access {}", path.join("::")));
        }
    }

    fn imports(&mut self, tree: &UseTree, mut prefix: Vec<String>) {
        match tree {
            | UseTree::Path(path) => {
                prefix.push(name(&path.ident));
                self.imports(&path.tree, prefix);
            },
            | UseTree::Group(group) =>
                for item in &group.items {
                    self.imports(item, prefix.clone());
                },
            | UseTree::Name(item) => {
                prefix.push(name(&item.ident));
                self.path(&prefix, item.span());
            },
            | UseTree::Rename(rename) => {
                prefix.push(name(&rename.ident));
                self.path(&prefix, rename.span());
            },
            | UseTree::Glob(glob) => {
                prefix.push("*".into());
                self.path(&prefix, glob.span());
            },
        }
    }

    // Macro arguments are tokens, not visited expression ASTs. Literals are
    // ignored; nested groups and qualified paths still need checking.
    fn tokens(&mut self, tokens: TokenStream) {
        let tokens: Vec<_> = tokens.into_iter().collect();
        for (index, token) in tokens.iter().enumerate() {
            match token {
                | TokenTree::Group(group) => self.tokens(group.stream()),
                | TokenTree::Ident(ident) if matches!(name(ident).as_str(), "std" | "core") => {
                    let mut path = vec![name(ident)];
                    let mut rest = &tokens[index + 1 ..];
                    while let [TokenTree::Punct(a), TokenTree::Punct(b), TokenTree::Ident(next), tail @ ..] = rest {
                        if a.as_char() != ':' || b.as_char() != ':' {
                            break;
                        }
                        path.push(name(next));
                        rest = tail;
                    }
                    if path.len() > 1 || matches!(rest.first(), Some(TokenTree::Ident(next)) if next == "as") {
                        self.path(&path, ident.span());
                    }
                    // A grouped root import inside a macro could rename std.
                    if matches!(rest, [TokenTree::Punct(a), TokenTree::Punct(b), TokenTree::Group(_), ..] if a.as_char() == ':' && b.as_char() == ':') {
                        self.path(&path, ident.span());
                    }
                },
                | TokenTree::Ident(ident) if HOST_MACROS.contains(&name(ident).as_str()) => {
                    if matches!(tokens.get(index + 1), Some(TokenTree::Punct(p)) if p.as_char() == '!') {
                        self.reject(
                            ident.span(),
                            "host I/O, environment, assembly and local macro definitions are forbidden in pure code",
                        );
                    }
                },
                | TokenTree::Ident(ident) if FILE_METHODS.contains(&name(ident).as_str()) => {
                    let method = matches!(tokens.get(index.wrapping_sub(1)), Some(TokenTree::Punct(p)) if p.as_char() == '.')
                        && matches!(tokens.get(index + 1), Some(TokenTree::Group(g)) if g.delimiter() == proc_macro2::Delimiter::Parenthesis);
                    let associated = matches!(&tokens[..index], [.., TokenTree::Punct(a), TokenTree::Punct(b)] if a.as_char() == ':' && b.as_char() == ':');
                    if method || associated {
                        self.reject(ident.span(), "filesystem query methods are forbidden in pure code");
                    }
                },
                | TokenTree::Ident(ident) if ident == "extern" => {
                    self.reject(ident.span(), "foreign functions are forbidden in pure code");
                },
                | _ => {},
            }
        }
    }
}

impl<'ast> Visit<'ast> for Guard<'_> {
    fn visit_item(&mut self, item: &'ast Item) {
        let attrs = match item {
            | Item::Const(i) => &i.attrs,
            | Item::Enum(i) => &i.attrs,
            | Item::ExternCrate(i) => &i.attrs,
            | Item::Fn(i) => &i.attrs,
            | Item::ForeignMod(i) => &i.attrs,
            | Item::Impl(i) => &i.attrs,
            | Item::Macro(i) => &i.attrs,
            | Item::Mod(i) => &i.attrs,
            | Item::Static(i) => &i.attrs,
            | Item::Struct(i) => &i.attrs,
            | Item::Trait(i) => &i.attrs,
            | Item::TraitAlias(i) => &i.attrs,
            | Item::Type(i) => &i.attrs,
            | Item::Union(i) => &i.attrs,
            | Item::Use(i) => &i.attrs,
            | _ => {
                self.reject(item.span(), "unsupported Rust item in purity check");
                return;
            },
        };
        if !test_only(attrs) {
            visit::visit_item(self, item);
        }
    }

    fn visit_path(&mut self, path: &'ast syn::Path) {
        self.path(&path.segments.iter().map(|segment| name(&segment.ident)).collect::<Vec<_>>(), path.span());
        visit::visit_path(self, path);
    }

    fn visit_impl_item_fn(&mut self, method: &'ast syn::ImplItemFn) {
        if !test_only(&method.attrs) {
            visit::visit_impl_item_fn(self, method);
        }
    }

    fn visit_item_use(&mut self, item: &'ast syn::ItemUse) { self.imports(&item.tree, vec![]); }

    fn visit_item_extern_crate(&mut self, item: &'ast syn::ItemExternCrate) {
        if matches!(name(&item.ident).as_str(), "std" | "core") {
            self.reject(item.span(), "standard library roots must not be hidden behind an extern crate alias");
        }
    }

    fn visit_item_foreign_mod(&mut self, item: &'ast syn::ItemForeignMod) { self.reject(item.span(), "foreign functions are forbidden in pure code"); }

    fn visit_macro(&mut self, item: &'ast syn::Macro) {
        self.path(
            &item.path.segments.iter().map(|segment| name(&segment.ident)).collect::<Vec<_>>(),
            item.path.span(),
        );
        if HOST_MACROS.contains(&name(&item.path.segments.last().unwrap().ident).as_str()) {
            self.reject(
                item.span(),
                "host I/O, environment, assembly and local macro definitions are forbidden in pure code",
            );
        }
        self.tokens(item.tokens.clone());
    }

    fn visit_expr_method_call(&mut self, call: &'ast syn::ExprMethodCall) {
        // shortcut: receiver types are unresolved; reserve I/O method names
        // until a compiler lint is needed to allow unrelated methods
        // with these names.
        if FILE_METHODS.contains(&name(&call.method).as_str()) {
            self.reject(call.method.span(), "filesystem query methods are forbidden in pure code");
        }
        visit::visit_expr_method_call(self, call);
    }

    fn visit_attribute(&mut self, attr: &'ast Attribute) {
        fn redirects(meta: &Meta) -> bool {
            meta.path().is_ident("path")
                || (meta.path().is_ident("cfg_attr")
                    && matches!(meta, Meta::List(list) if list.parse_args_with(Punctuated::<Meta, Token![,]>::parse_terminated)
                    .is_ok_and(|args| args.iter().any(redirects))))
        }
        if redirects(&attr.meta) {
            self.reject(attr.span(), "production modules must remain under the checked source tree");
        }
    }

    fn visit_item_mod(&mut self, item: &'ast syn::ItemMod) {
        let next = self.module_dir.join(item.ident.to_string());
        if item.content.is_some() {
            let previous = std::mem::replace(&mut self.module_dir, next);
            visit::visit_item_mod(self, item);
            self.module_dir = previous;
        } else {
            let previous_errors = self.errors.len();
            for attr in &item.attrs {
                self.visit_attribute(attr);
            }
            if self.errors.len() != previous_errors {
                return;
            }
            let flat = next.with_extension("rs");
            let file = if flat.is_file() { flat } else { next.join("mod.rs") };
            inspect(&file, &next, self.errors);
        }
    }
}

fn inspect(file: &Path, module_dir: &Path, errors: &mut Vec<String>) {
    let source = std::fs::read_to_string(file).unwrap_or_else(|error| panic!("Cannot inspect {}: {error}", file.display()));
    check(&source, file, module_dir, errors);
}

fn check(source: &str, file: &Path, module_dir: &Path, errors: &mut Vec<String>) {
    let syntax = syn::parse_file(source).unwrap_or_else(|error| panic!("Cannot parse {}: {error}", file.display()));
    if test_only(&syntax.attrs) {
        return;
    }
    Guard {
        file: file.into(),
        module_dir: module_dir.into(),
        errors,
    }
    .visit_file(&syntax);
}

#[test]
fn pure_production_sources_do_not_access_the_host() {
    let root = Path::new(env!("CARGO_MANIFEST_DIR")).parent().unwrap();
    let mut errors = vec![];
    for layer in ["domain", "contracts", "dci", "presentation", "ports"] {
        let directory = root.join(layer).join("src");
        inspect(&directory.join("lib.rs"), &directory, &mut errors);
    }
    let application = root.join("application/src");
    inspect(&application.join("job_events.rs"), &application, &mut errors);
    assert!(errors.is_empty(), "Pure-layer side effects:\n{}", errors.join("\n"));
}

#[test]
fn application_use_cases_route_clock_and_diagnostics_through_ports() {
    let root = Path::new(env!("CARGO_MANIFEST_DIR")).join("src");
    let mut local = vec![];
    inspect(&root.join("lib.rs"), &root, &mut local);
    let errors: Vec<_> = local
        .into_iter()
        .filter(|message| !message.contains("host_defaults.rs"))
        .filter(|message| {
            message.contains("SystemTime") || message.contains("UNIX_EPOCH") || message.contains("host I/O, environment, assembly and local macro definitions")
        })
        .collect();
    assert!(
        errors.is_empty(),
        "Application use cases must use Clock and DiagnosticSink ports:\n{}",
        errors.join("\n")
    );
}

#[test]
fn follows_external_and_inline_modules_but_skips_test_only_module_files() {
    let directory = Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/architecture/fixtures");
    let mut errors = vec![];
    inspect(&directory.join("lib.rs"), &directory, &mut errors);
    assert_eq!(errors.len(), 1, "{}", errors.join("\n"));
    assert!(errors[0].contains("clock.rs:2:"), "{}", errors[0]);
    assert!(errors[0].contains("std::time::SystemTime::now"), "{}", errors[0]);
}

#[test]
fn rejects_host_access_imports_aliases_function_values_and_macro_arguments() {
    for source in [
        "fn f() { let _ = std::fs::read(\"x\"); }",
        "struct State; impl State { fn run() { let _ = std::fs::read(\"x\"); } }",
        "use std::{fs as storage}; fn f() { let _ = storage::read(\"x\"); }",
        "use std::fs::{read as load}; fn f() { let _ = load(\"x\"); }",
        "pub use std::process::Command as Runner;",
        "use std as host;",
        "use core::include_str as embed; fn f() { let _ = embed!(\"secret\"); }",
        "use std::println as output; fn f() { output!(\"message\"); }",
        "extern crate std as host;",
        "use std::*;",
        "fn f() { let _ = std::net::TcpStream::connect(\"x:1\"); }",
        "fn f() { let _ = std::env::var(\"HOME\"); }",
        "use std::time::SystemTime as Clock; fn f() { let _ = Clock::now(); }",
        "fn f() { let _ = std::time::Instant::now; }",
        "fn f() { let _ = std::time::UNIX_EPOCH.elapsed(); }",
        "fn f() { let _ = r#std::r#fs::read(\"x\"); }",
        "fn f(p: &std::path::Path) { let _ = p.exists(); }",
        "fn f() { let _ = std::path::absolute(\"relative\"); }",
        "fn f() { let _ = unsafe { core::arch::x86_64::_rdtsc() }; }",
        "fn f(p: &std::path::Path) { let _ = std::path::Path::exists(p); }",
        "use std::path::Path as P; fn f(p: &P) { let _ = P::exists(p); }",
        "fn f() { let _ = format!(\"{:?}\", std::fs::read(\"x\")); }",
        "fn f(p: &std::path::Path) { let _ = format!(\"{}\", p.exists()); }",
        "use std::path::Path as P; fn f(p: &P) { let _ = format!(\"{}\", P::exists(p)); }",
        "fn f() { let _ = format!(\"{:?}\", r#std::fs::read(\"x\")); }",
        "fn f() { println!(\"output\"); }",
        "fn f() { let _ = include_str!(\"secret\"); }",
        "fn f() { let _ = env!(\"HOME\"); }",
        "macro_rules! hidden { () => { std::fs::read(\"x\") }; }",
        "#[cfg(any(test, windows))] fn f() { let _ = std::fs::read(\"x\"); }",
        "#[cfg(not(test))] fn f() { let _ = std::time::SystemTime::now(); }",
        "#[path = \"outside.rs\"] mod redirected {}",
        "#[cfg_attr(windows, path = \"outside.rs\")] mod redirected {}",
        "unsafe extern \"C\" { fn host_call(); }",
    ] {
        let mut errors = vec![];
        check(source, Path::new("fixture.rs"), Path::new("."), &mut errors);
        assert!(!errors.is_empty(), "Purity guard missed: {source}");
        assert!(errors[0].starts_with("fixture.rs:1:"), "Missing source location: {}", errors[0]);
    }
}

#[test]
fn allows_pure_values_literals_ports_and_test_only_host_access() {
    let mut errors = vec![];
    check(
        r#"
        use std::{collections::HashMap, path::Path, time::Duration};
        const EXAMPLE: &str = "std::fs::read std::time::SystemTime::now";
        // std::process::Command::new("ignored comment")
        fn pure(path: &Path) -> Duration { let _ = path.is_absolute(); Duration::from_secs(2) }
        fn text() -> String { format!("std::fs::read {}", 2) }
        fn qualified_macro() -> String { std::format!("literal") }
        fn local_name() -> String { let std = 1; format!("{}", std) }
        trait Port { fn read(&self, name: &str) -> Vec<u8>; }
        #[cfg(test)] mod tests { fn io() { let _ = std::env::current_exe(); } }
        #[cfg(all(test, windows))] mod external_tests;
        #[test] fn unit() { let _ = std::fs::read("fixture"); }
        struct State;
        impl State {
            #[cfg(test)] fn fixture() { let _ = std::env::current_exe(); }
        }
    "#,
        Path::new("fixture.rs"),
        Path::new("."),
        &mut errors,
    );
    assert!(errors.is_empty(), "False positives: {}", errors.join("\n"));
}
