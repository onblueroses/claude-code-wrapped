use std::collections::BTreeSet;
use std::env;
use std::fs;
use std::path::Path;

use syn::punctuated::Punctuated;
use syn::visit::{self, Visit};
use syn::{
    Attribute, Expr, Fields, Item, ItemEnum, ItemImpl, ItemMod, ItemStruct, ItemUnion, ItemUse,
    Lit, Meta, Token, UseTree, Visibility,
};

fn main() {
    let source_path = env::args_os()
        .nth(1)
        .unwrap_or_else(|| fail("usage: report-source-audit <report-source.rs>"));
    let source = fs::read_to_string(&source_path).unwrap_or_else(|error| {
        fail(&format!(
            "read {}: {error}",
            Path::new(&source_path).display()
        ))
    });
    let file = syn::parse_file(&source).unwrap_or_else(|error| {
        fail(&format!(
            "parse {}: {error}",
            Path::new(&source_path).display()
        ))
    });

    EscapeHatchAudit.visit_file(&file);
    let mut captured_types = BTreeSet::new();
    audit_items(&file.items, "", true, &mut captured_types);
    for name in captured_types {
        println!("{name}");
    }
}

struct EscapeHatchAudit;

impl<'ast> Visit<'ast> for EscapeHatchAudit {
    fn visit_item_impl(&mut self, item: &'ast ItemImpl) {
        audit_impl(item, "syntax tree");
        visit::visit_item_impl(self, item);
    }

    fn visit_item_macro(&mut self, item: &'ast syn::ItemMacro) {
        let name = item
            .mac
            .path
            .segments
            .last()
            .map(|segment| segment.ident.to_string())
            .unwrap_or_else(|| "<unknown>".to_owned());
        fail(&format!(
            "item macro `{name}` can generate unaudited report types or Serialize implementations"
        ));
    }
}

fn fail(message: &str) -> ! {
    eprintln!("report source audit: {message}");
    std::process::exit(1);
}

fn audit_items(
    items: &[Item],
    module_prefix: &str,
    publicly_reachable: bool,
    captured_types: &mut BTreeSet<String>,
) {
    for item in items {
        match item {
            Item::Struct(item) => {
                audit_struct(item, module_prefix, publicly_reachable, captured_types)
            }
            Item::Enum(item) => audit_enum(item, module_prefix),
            Item::Union(item) => audit_union(item, module_prefix),
            Item::Impl(item) => audit_impl(item, module_prefix),
            Item::Use(item) => audit_use(item, module_prefix),
            Item::Type(item) if is_public(&item.vis) => fail(&format!(
                "public type alias `{}` can expose an unaudited serializable report type",
                qualified_name(module_prefix, &item.ident.to_string())
            )),
            Item::ExternCrate(_) => fail(&format!(
                "extern-crate imports in module `{}` can alias derive macros and are not allowed",
                display_module(module_prefix)
            )),
            Item::Macro(item) => {
                let name = item
                    .mac
                    .path
                    .segments
                    .last()
                    .map(|segment| segment.ident.to_string())
                    .unwrap_or_else(|| "<unknown>".to_owned());
                fail(&format!(
                    "top-level macro `{name}` can generate unaudited report types or Serialize implementations"
                ));
            }
            Item::Mod(item) => {
                audit_module(item, module_prefix, publicly_reachable, captured_types)
            }
            _ => audit_attributes(item_attrs(item), "top-level item"),
        }
    }
}

fn item_attrs(item: &Item) -> &[Attribute] {
    match item {
        Item::Const(item) => &item.attrs,
        Item::ExternCrate(item) => &item.attrs,
        Item::Fn(item) => &item.attrs,
        Item::ForeignMod(item) => &item.attrs,
        Item::Static(item) => &item.attrs,
        Item::Trait(item) => &item.attrs,
        Item::TraitAlias(item) => &item.attrs,
        Item::Type(item) => &item.attrs,
        Item::Use(item) => &item.attrs,
        _ => &[],
    }
}

fn audit_module(
    item: &ItemMod,
    module_prefix: &str,
    publicly_reachable: bool,
    captured_types: &mut BTreeSet<String>,
) {
    let name = qualified_name(module_prefix, &item.ident.to_string());
    audit_attributes(&item.attrs, &format!("module `{name}`"));
    let Some((_, items)) = &item.content else {
        fail(&format!(
            "external module `{name}` escapes the report source syntax audit"
        ));
    };
    audit_items(
        items,
        &name,
        publicly_reachable && is_public(&item.vis),
        captured_types,
    );
}

fn audit_struct(
    item: &ItemStruct,
    module_prefix: &str,
    publicly_reachable: bool,
    captured_types: &mut BTreeSet<String>,
) {
    let name = qualified_name(module_prefix, &item.ident.to_string());
    let derives = derive_names(&item.attrs, &format!("struct `{name}`"));
    let serializable = derives.contains("Serialize");
    let deserializable = derives.contains("Deserialize");
    let defaultable = derives.contains("Default");
    let serde_count = audit_container_attributes(&item.attrs, &format!("struct `{name}`"));

    if deserializable && !serializable {
        fail(&format!(
            "deserializable struct `{name}` is outside the bounded serializable report domain"
        ));
    }
    if serializable && !defaultable {
        fail(&format!(
            "serializable struct `{name}` is not Default; add an explicit bounded fixture strategy"
        ));
    }
    if serializable && serde_count != 1 {
        fail(&format!(
            "serializable struct `{name}` must have exactly one #[serde(rename_all = \"camelCase\")] attribute"
        ));
    }

    for (index, field) in fields(&item.fields).enumerate() {
        let field_name = field
            .ident
            .as_ref()
            .map(ToString::to_string)
            .unwrap_or_else(|| index.to_string());
        audit_field_attributes(&field.attrs, &format!("field `{name}.{field_name}`"));
    }

    if serializable && publicly_reachable && is_public(&item.vis) {
        captured_types.insert(name);
    }
}

fn audit_enum(item: &ItemEnum, module_prefix: &str) {
    let name = qualified_name(module_prefix, &item.ident.to_string());
    let derives = derive_names(&item.attrs, &format!("enum `{name}`"));
    audit_attributes(&item.attrs, &format!("enum `{name}`"));
    if derives.contains("Serialize") || derives.contains("Deserialize") {
        fail(&format!(
            "Serde enum `{name}` requires an exhaustive variant fixture matrix"
        ));
    }
}

fn audit_union(item: &ItemUnion, module_prefix: &str) {
    let name = qualified_name(module_prefix, &item.ident.to_string());
    let derives = derive_names(&item.attrs, &format!("union `{name}`"));
    audit_attributes(&item.attrs, &format!("union `{name}`"));
    if derives.contains("Serialize") || derives.contains("Deserialize") {
        fail(&format!(
            "Serde union `{name}` is outside the bounded report fixture domain"
        ));
    }
}

fn audit_impl(item: &ItemImpl, module_prefix: &str) {
    audit_attributes(&item.attrs, "implementation");
    if let Some((_, trait_path, _)) = &item.trait_ {
        let trait_name = trait_path
            .segments
            .iter()
            .map(|segment| segment.ident.to_string())
            .collect::<Vec<_>>()
            .join("::");
        if trait_path
            .segments
            .last()
            .is_some_and(|segment| segment.ident == "Serialize" || segment.ident == "Deserialize")
        {
            fail(&format!(
                "manual Serde implementation in module `{}` escapes the derived-struct fixture audit",
                if module_prefix.is_empty() { "report" } else { module_prefix }
            ));
        }
        if trait_name != "std::ops::AddAssign" {
            fail(&format!(
                "manual trait implementation `{trait_name}` in module `{}` is not allowlisted and can hide serialization behavior",
                if module_prefix.is_empty() { "report" } else { module_prefix }
            ));
        }
    }
}

fn audit_use(item: &ItemUse, module_prefix: &str) {
    audit_attributes(&item.attrs, "use declaration");
    if is_public(&item.vis) {
        fail(&format!(
            "public re-export in module `{}` can expose an unaudited serializable report type",
            display_module(module_prefix)
        ));
    }
    let mut paths = Vec::new();
    collect_use_paths(&item.tree, &mut Vec::new(), &mut paths, module_prefix);
    for path in paths {
        let Some(imported_name) = path.last() else {
            continue;
        };
        if matches!(
            imported_name.as_str(),
            "Debug" | "Clone" | "Default" | "Deserialize" | "Serialize"
        ) && path != ["serde", "Deserialize"]
            && path != ["serde", "Serialize"]
        {
            fail(&format!(
                "import `{}` in module `{}` can shadow an allowlisted derive macro",
                path.join("::"),
                display_module(module_prefix)
            ));
        }
    }
}

fn collect_use_paths(
    tree: &UseTree,
    prefix: &mut Vec<String>,
    paths: &mut Vec<Vec<String>>,
    module_prefix: &str,
) {
    match tree {
        UseTree::Path(path) => {
            prefix.push(path.ident.to_string());
            collect_use_paths(&path.tree, prefix, paths, module_prefix);
            prefix.pop();
        }
        UseTree::Name(name) => {
            let mut path = prefix.clone();
            path.push(name.ident.to_string());
            paths.push(path);
        }
        UseTree::Group(group) => {
            for item in &group.items {
                collect_use_paths(item, prefix, paths, module_prefix);
            }
        }
        UseTree::Rename(_) | UseTree::Glob(_) => fail(&format!(
            "renamed or glob import in module `{}` can hide derive-macro identity",
            display_module(module_prefix)
        )),
    }
}

fn fields(fields: &Fields) -> impl Iterator<Item = &syn::Field> {
    fields.iter()
}

fn audit_container_attributes(attrs: &[Attribute], context: &str) -> usize {
    let mut direct_serde = 0;
    for attr in attrs {
        if attr.path().is_ident("serde") {
            direct_serde += 1;
            if !is_camel_case_container_rule(attr) {
                fail(&format!(
                    "{context} has value-dependent or unsupported Serde behavior; expand the fixture matrix"
                ));
            }
        } else if attr.path().is_ident("cfg_attr") {
            fail(&format!(
                "{context} uses cfg_attr and can conditionally change serialization behavior"
            ));
        } else if !is_inert_container_attribute(attr) {
            fail(&format!(
                "{context} has an unsupported attribute that can rewrite the bounded report item"
            ));
        }
    }
    direct_serde
}

fn audit_field_attributes(attrs: &[Attribute], context: &str) {
    for attr in attrs {
        if attr.path().is_ident("serde") || attr.path().is_ident("cfg_attr") {
            fail(&format!(
                "{context} has field-level Serde behavior; expand the fixture matrix"
            ));
        } else if !attr.path().is_ident("doc") {
            fail(&format!(
                "{context} has an unsupported attribute that can rewrite the bounded report field"
            ));
        }
    }
}

fn audit_attributes(attrs: &[Attribute], context: &str) {
    for attr in attrs {
        if attr.path().is_ident("serde") || attr.path().is_ident("cfg_attr") {
            fail(&format!(
                "{context} has Serde behavior outside an audited report struct"
            ));
        } else if !attr.path().is_ident("doc") && !attr.path().is_ident("allow") {
            fail(&format!(
                "{context} has an unsupported attribute that can rewrite report source"
            ));
        }
    }
}

fn derive_names(attrs: &[Attribute], context: &str) -> BTreeSet<String> {
    let mut names = BTreeSet::new();
    for attr in attrs.iter().filter(|attr| attr.path().is_ident("derive")) {
        let paths = attr
            .parse_args_with(Punctuated::<syn::Path, Token![,]>::parse_terminated)
            .unwrap_or_else(|error| fail(&format!("parse derive attribute on {context}: {error}")));
        for path in paths {
            let qualified = path
                .segments
                .iter()
                .map(|segment| segment.ident.to_string())
                .collect::<Vec<_>>();
            let allowed = matches!(
                qualified.as_slice(),
                [name] if matches!(
                    name.as_str(),
                    "Debug" | "Clone" | "Default" | "Deserialize" | "Serialize"
                )
            ) || qualified == ["serde", "Deserialize"]
                || qualified == ["serde", "Serialize"];
            if !allowed {
                fail(&format!(
                    "derive `{}` on {context} is not allowlisted for the bounded report domain",
                    qualified.join("::")
                ));
            }
            if let Some(segment) = path.segments.last() {
                names.insert(segment.ident.to_string());
            }
        }
    }
    names
}

fn is_camel_case_container_rule(attr: &Attribute) -> bool {
    let Ok(entries) = attr.parse_args_with(Punctuated::<Meta, Token![,]>::parse_terminated) else {
        return false;
    };
    if entries.len() != 1 {
        return false;
    }
    let Some(Meta::NameValue(rule)) = entries.first() else {
        return false;
    };
    if !rule.path.is_ident("rename_all") {
        return false;
    }
    matches!(
        &rule.value,
        Expr::Lit(value) if matches!(&value.lit, Lit::Str(value) if value.value() == "camelCase")
    )
}

fn is_inert_container_attribute(attr: &Attribute) -> bool {
    attr.path().is_ident("derive") || attr.path().is_ident("doc")
}

fn qualified_name(prefix: &str, name: &str) -> String {
    if prefix.is_empty() {
        name.to_owned()
    } else {
        format!("{prefix}::{name}")
    }
}

fn is_public(vis: &Visibility) -> bool {
    matches!(vis, Visibility::Public(_))
}

fn display_module(module_prefix: &str) -> &str {
    if module_prefix.is_empty() {
        "report"
    } else {
        module_prefix
    }
}
