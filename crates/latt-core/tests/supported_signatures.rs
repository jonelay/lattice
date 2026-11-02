//! No supported signature names an unstable item or an unlisted dependency's type.
//!
//! `docs/embedding.md` makes the rule mechanical: an item the rustdoc shows is
//! supported, a `#[doc(hidden)]` one is not. A hidden item stays callable, so an
//! import list cannot see a supported function that returns one, or a public field
//! whose type comes from a crate the API never promised. This walks the rustdoc JSON
//! of each supported surface and fails naming the root and the item it leaks.

use std::collections::{BTreeSet, HashSet};
use std::path::{Path, PathBuf};
use std::process::Command;

use rustdoc_types::{
    Attribute, Crate, FORMAT_VERSION, FunctionSignature, GenericArg, GenericArgs, GenericBound,
    GenericParamDef, GenericParamDefKind, Generics, Id, Item, ItemEnum, StructKind, Term, Type,
    VariantKind, Visibility, WherePredicate,
};

/// The crates a supported signature may name besides the crate's own roots.
const ALLOWED_CRATES: &[&str] = &[
    "std",
    "core",
    "alloc",
    "serde",
    // serde's traits are defined in serde_core and re-exported by serde.
    "serde_core",
    "serde_json",
    "serde_norway",
    "latt_adapter",
];

fn workspace() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("../..")
}

/// Produce the rustdoc JSON of `package` with `extra` cargo arguments and decode it.
///
/// A target directory of its own, because a cargo run inside `cargo test` otherwise
/// waits on the build directory's lock.
fn rustdoc_json(package: &str, extra: &[&str]) -> Crate {
    let target = workspace().join("target/rustdoc-json");
    let status = Command::new(std::env::var("CARGO").unwrap_or_else(|_| "cargo".into()))
        .current_dir(workspace())
        .args(["rustdoc", "--locked", "--lib", "-p", package])
        .args(extra)
        .args([
            "--",
            "-Z",
            "unstable-options",
            "--output-format",
            "json",
            "--document-hidden-items",
        ])
        .env("CARGO_TARGET_DIR", &target)
        .env("RUSTC_BOOTSTRAP", "1")
        .env_remove("RUSTFLAGS")
        .env_remove("RUSTDOCFLAGS")
        .env_remove("CARGO_ENCODED_RUSTFLAGS")
        .env_remove("CARGO_ENCODED_RUSTDOCFLAGS")
        .status()
        .expect("cargo rustdoc runs");
    assert!(
        status.success(),
        "cargo rustdoc -p {package} {extra:?} failed"
    );
    let path = target.join(format!("doc/{}.json", package.replace('-', "_")));
    let text = std::fs::read_to_string(&path).expect("rustdoc wrote its JSON");
    let format: serde_json::Value = serde_json::from_str(&text).expect("rustdoc JSON");
    assert_eq!(
        format["format_version"], FORMAT_VERSION,
        "rustdoc JSON format differs from rustdoc-types; move the pin with the toolchain"
    );
    serde_json::from_str(&text).expect("rustdoc JSON decodes")
}

/// True when the item carries `#[doc(hidden)]`.
fn hidden(item: &Item) -> bool {
    item.attrs
        .iter()
        .any(|attr| matches!(attr, Attribute::Other(text) if text == "#[doc(hidden)]"))
}

/// The items reachable from the crate root through modules and `pub use` that are
/// not hidden: the supported surface.
fn roots(krate: &Crate) -> HashSet<Id> {
    let mut roots = HashSet::new();
    let mut modules = HashSet::new();
    visit_module(krate, krate.root, &mut roots, &mut modules);
    roots
}

fn visit_module(krate: &Crate, module: Id, roots: &mut HashSet<Id>, seen: &mut HashSet<Id>) {
    if !seen.insert(module) {
        return;
    }
    let Some(ItemEnum::Module(m)) = krate.index.get(&module).map(|item| &item.inner) else {
        return;
    };
    for id in &m.items {
        let Some(item) = krate.index.get(id) else {
            continue;
        };
        if hidden(item) || item.visibility != Visibility::Public {
            continue;
        }
        match &item.inner {
            ItemEnum::Module(_) => visit_module(krate, *id, roots, seen),
            ItemEnum::Use(use_) => {
                let Some(target) = use_.id else { continue };
                // Re-exporting an item hidden where it is defined does not show it.
                if krate.index.get(&target).is_some_and(hidden) {
                    continue;
                }
                match krate.index.get(&target).map(|t| &t.inner) {
                    Some(ItemEnum::Module(_)) if use_.is_glob => {
                        visit_module(krate, target, roots, seen);
                    }
                    Some(ItemEnum::Module(_)) => visit_module(krate, target, roots, seen),
                    Some(_) => {
                        roots.insert(target);
                    }
                    // A re-exported crate, or an item of another crate: not ours to walk.
                    None => {}
                }
            }
            ItemEnum::Impl(_) => {}
            _ => {
                roots.insert(*id);
            }
        }
    }
}

struct Walk<'a> {
    krate: &'a Crate,
    roots: &'a HashSet<Id>,
    root: String,
    leaks: BTreeSet<String>,
}

impl Walk<'_> {
    fn name(&self, id: Id) -> String {
        self.krate.paths.get(&id).map_or_else(
            || {
                self.krate
                    .index
                    .get(&id)
                    .and_then(|item| item.name.clone())
                    .unwrap_or_else(|| format!("{id:?}"))
            },
            |summary| summary.path.join("::"),
        )
    }

    fn path(&mut self, path: &rustdoc_types::Path) {
        if !self.roots.contains(&path.id) {
            let allowed = self.krate.paths.get(&path.id).is_some_and(|summary| {
                summary.crate_id != 0
                    && self
                        .krate
                        .external_crates
                        .get(&summary.crate_id)
                        .is_some_and(|c| ALLOWED_CRATES.contains(&c.name.as_str()))
            });
            if !allowed {
                let leak = format!("{} names {}", self.root, self.name(path.id));
                self.leaks.insert(leak);
            }
        }
        if let Some(args) = &path.args {
            self.args(args);
        }
    }

    fn args(&mut self, args: &GenericArgs) {
        match args {
            GenericArgs::AngleBracketed { args, constraints } => {
                for arg in args {
                    match arg {
                        GenericArg::Type(ty) => self.ty(ty),
                        GenericArg::Lifetime(_) | GenericArg::Const(_) | GenericArg::Infer => {}
                    }
                }
                for constraint in constraints {
                    if let Some(args) = &constraint.args {
                        self.args(args);
                    }
                    match &constraint.binding {
                        rustdoc_types::AssocItemConstraintKind::Equality(term) => self.term(term),
                        rustdoc_types::AssocItemConstraintKind::Constraint(bounds) => {
                            self.bounds(bounds);
                        }
                    }
                }
            }
            GenericArgs::Parenthesized { inputs, output } => {
                for ty in inputs {
                    self.ty(ty);
                }
                if let Some(ty) = output {
                    self.ty(ty);
                }
            }
            GenericArgs::ReturnTypeNotation => {}
        }
    }

    fn term(&mut self, term: &Term) {
        if let Term::Type(ty) = term {
            self.ty(ty);
        }
    }

    fn ty(&mut self, ty: &Type) {
        match ty {
            Type::ResolvedPath(path) => self.path(path),
            Type::DynTrait(dyn_trait) => {
                for poly in &dyn_trait.traits {
                    self.params(&poly.generic_params);
                    self.path(&poly.trait_);
                }
            }
            Type::FunctionPointer(pointer) => {
                self.params(&pointer.generic_params);
                self.sig(&pointer.sig);
            }
            Type::Tuple(types) => {
                for ty in types {
                    self.ty(ty);
                }
            }
            Type::Slice(ty)
            | Type::Array { type_: ty, .. }
            | Type::Pat { type_: ty, .. }
            | Type::RawPointer { type_: ty, .. }
            | Type::BorrowedRef { type_: ty, .. } => self.ty(ty),
            Type::ImplTrait(bounds) => self.bounds(bounds),
            Type::QualifiedPath {
                args,
                self_type,
                trait_,
                ..
            } => {
                if let Some(args) = args {
                    self.args(args);
                }
                self.ty(self_type);
                if let Some(trait_) = trait_ {
                    self.path(trait_);
                }
            }
            Type::Generic(_) | Type::Primitive(_) | Type::Infer => {}
        }
    }

    fn bounds(&mut self, bounds: &[GenericBound]) {
        for bound in bounds {
            if let GenericBound::TraitBound {
                trait_,
                generic_params,
                ..
            } = bound
            {
                self.params(generic_params);
                self.path(trait_);
            }
        }
    }

    fn params(&mut self, params: &[GenericParamDef]) {
        for param in params {
            match &param.kind {
                GenericParamDefKind::Type {
                    bounds, default, ..
                } => {
                    self.bounds(bounds);
                    if let Some(ty) = default {
                        self.ty(ty);
                    }
                }
                GenericParamDefKind::Const { type_, .. } => self.ty(type_),
                GenericParamDefKind::Lifetime { .. } => {}
            }
        }
    }

    fn generics(&mut self, generics: &Generics) {
        self.params(&generics.params);
        for predicate in &generics.where_predicates {
            match predicate {
                WherePredicate::BoundPredicate {
                    type_,
                    bounds,
                    generic_params,
                } => {
                    self.ty(type_);
                    self.bounds(bounds);
                    self.params(generic_params);
                }
                WherePredicate::EqPredicate { lhs, rhs } => {
                    self.ty(lhs);
                    self.term(rhs);
                }
                WherePredicate::LifetimePredicate { .. } => {}
            }
        }
    }

    fn sig(&mut self, sig: &FunctionSignature) {
        for (_, ty) in &sig.inputs {
            self.ty(ty);
        }
        if let Some(ty) = &sig.output {
            self.ty(ty);
        }
    }

    /// Walk a field, variant field or associated item unless it is hidden or private.
    fn member(&mut self, id: Id, require_public: bool) {
        let Some(item) = self.krate.index.get(&id) else {
            return;
        };
        if hidden(item) || (require_public && item.visibility != Visibility::Public) {
            return;
        }
        match &item.inner {
            ItemEnum::StructField(ty) => self.ty(ty),
            ItemEnum::Function(function) => {
                self.generics(&function.generics);
                self.sig(&function.sig);
            }
            ItemEnum::AssocType {
                generics,
                bounds,
                type_,
                ..
            } => {
                self.generics(generics);
                self.bounds(bounds);
                if let Some(ty) = type_ {
                    self.ty(ty);
                }
            }
            ItemEnum::AssocConst { type_, .. } => self.ty(type_),
            ItemEnum::Variant(variant) => match &variant.kind {
                VariantKind::Plain => {}
                VariantKind::Tuple(fields) => {
                    for field in fields.iter().flatten() {
                        self.member(*field, false);
                    }
                }
                VariantKind::Struct { fields, .. } => {
                    for field in fields {
                        self.member(*field, false);
                    }
                }
            },
            _ => {}
        }
    }

    /// Walk the impls whose self type is `root`; an impl listed on it for another
    /// type (`From<Root> for Other`) is the other type's API.
    fn impls(&mut self, root: Id, impls: &[Id]) {
        for id in impls {
            let Some(item) = self.krate.index.get(id) else {
                continue;
            };
            let ItemEnum::Impl(imp) = &item.inner else {
                continue;
            };
            if imp.blanket_impl.is_some() || imp.is_synthetic || hidden(item) {
                continue;
            }
            if !matches!(&imp.for_, Type::ResolvedPath(path) if path.id == root) {
                continue;
            }
            self.generics(&imp.generics);
            if let Some(trait_) = &imp.trait_ {
                self.path(trait_);
            }
            self.ty(&imp.for_);
            // An inherent impl's private items are not API; a trait impl's are the trait's.
            let require_public = imp.trait_.is_none();
            for member in &imp.items {
                self.member(*member, require_public);
            }
        }
    }

    fn item(&mut self, id: Id) {
        let Some(item) = self.krate.index.get(&id) else {
            return;
        };
        match &item.inner {
            ItemEnum::Struct(s) => {
                self.generics(&s.generics);
                match &s.kind {
                    StructKind::Unit => {}
                    StructKind::Tuple(fields) => {
                        for field in fields.iter().flatten() {
                            self.member(*field, true);
                        }
                    }
                    StructKind::Plain { fields, .. } => {
                        for field in fields {
                            self.member(*field, true);
                        }
                    }
                }
                self.impls(id, &s.impls);
            }
            ItemEnum::Enum(e) => {
                self.generics(&e.generics);
                for variant in &e.variants {
                    self.member(*variant, false);
                }
                self.impls(id, &e.impls);
            }
            ItemEnum::Union(u) => {
                self.generics(&u.generics);
                for field in &u.fields {
                    self.member(*field, true);
                }
                self.impls(id, &u.impls);
            }
            ItemEnum::Function(function) => {
                self.generics(&function.generics);
                self.sig(&function.sig);
            }
            ItemEnum::Trait(t) => {
                self.generics(&t.generics);
                self.bounds(&t.bounds);
                for member in &t.items {
                    self.member(*member, false);
                }
            }
            ItemEnum::TypeAlias(alias) => {
                self.generics(&alias.generics);
                self.ty(&alias.type_);
            }
            ItemEnum::Constant { type_, .. } => self.ty(type_),
            ItemEnum::Static(s) => self.ty(&s.type_),
            _ => {}
        }
    }
}

/// Every leak of one surface, as `root names item`.
fn leaks(krate: &Crate) -> BTreeSet<String> {
    let roots = roots(krate);
    let mut all = BTreeSet::new();
    for root in &roots {
        let mut walk = Walk {
            krate,
            roots: &roots,
            root: String::new(),
            leaks: BTreeSet::new(),
        };
        walk.root = walk.name(*root);
        walk.item(*root);
        all.append(&mut walk.leaks);
    }
    all
}

// Requirement: graph-service/The supported embedding surface is service-first
#[test]
fn no_supported_signature_names_an_unstable_item() {
    let mut failures = Vec::new();
    for (package, extra, surface) in [
        ("latt-core", &[][..], "latt-core default features"),
        (
            "latt-core",
            &["--no-default-features"][..],
            "latt-core no default features",
        ),
        ("latt-adapter", &[][..], "latt-adapter"),
    ] {
        for leak in leaks(&rustdoc_json(package, extra)) {
            failures.push(format!("{surface}: {leak}"));
        }
    }
    assert!(
        failures.is_empty(),
        "{} supported signature(s) name an unstable item:\n{}",
        failures.len(),
        failures.join("\n")
    );
}
