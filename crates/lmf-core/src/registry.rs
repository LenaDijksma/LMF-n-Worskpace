//! The built-in extension registry: the few extensions this implementation understands.
//!
//! Everything else is an *unknown extension* and is preserved verbatim. Phase 1 deliberately
//! ships a tiny set of unambiguous extensions (callout, details, tabs).

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum AttrType {
    Str,
    Bool,
    Int,
    /// Integer or float.
    Number,
}

#[derive(Debug)]
pub struct AttrSpec {
    pub name: &'static str,
    pub ty: AttrType,
    pub required: bool,
    /// If set, string values must be one of these.
    pub allowed: Option<&'static [&'static str]>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum BodyRule {
    Optional,
    Required,
}

#[derive(Debug)]
pub struct ExtensionSpec {
    pub namespace: &'static str,
    pub name: &'static str,
    /// Extension payload version understood by this implementation.
    pub version: u32,
    pub summary: &'static str,
    pub attrs: &'static [AttrSpec],
    pub body: BodyRule,
    /// Qualified name (`ns:name`) the directive must sit directly inside.
    pub required_parent: Option<&'static str>,
    /// If set, the only directives allowed directly inside (qualified names).
    pub allowed_children: Option<&'static [&'static str]>,
    /// Pairs of attributes that must not be used together.
    pub exclusive: &'static [(&'static str, &'static str)],
}

/// Namespace reserved for extensions defined by the LMF specification itself.
pub const RESERVED_NAMESPACE: &str = "lmf";

pub const CALLOUT_KINDS: &[&str] = &[
    "note", "tip", "info", "success", "warning", "danger", "question", "example",
];

pub static EXTENSIONS: &[ExtensionSpec] = &[
    ExtensionSpec {
        namespace: "lmf",
        name: "callout",
        version: 1,
        summary: "Highlighted note, tip or warning box",
        attrs: &[
            AttrSpec {
                name: "kind",
                ty: AttrType::Str,
                required: false,
                allowed: Some(CALLOUT_KINDS),
            },
            AttrSpec {
                name: "title",
                ty: AttrType::Str,
                required: false,
                allowed: None,
            },
        ],
        body: BodyRule::Required,
        required_parent: None,
        allowed_children: None,
        exclusive: &[],
    },
    ExtensionSpec {
        namespace: "lmf",
        name: "details",
        version: 1,
        summary: "Collapsible section",
        attrs: &[
            AttrSpec {
                name: "title",
                ty: AttrType::Str,
                required: false,
                allowed: None,
            },
            AttrSpec {
                name: "open",
                ty: AttrType::Bool,
                required: false,
                allowed: None,
            },
        ],
        body: BodyRule::Required,
        required_parent: None,
        allowed_children: None,
        exclusive: &[],
    },
    ExtensionSpec {
        namespace: "lmf",
        name: "tabs",
        version: 1,
        summary: "Tab group; contains lmf:tab blocks",
        attrs: &[],
        body: BodyRule::Required,
        required_parent: None,
        allowed_children: Some(&["lmf:tab"]),
        exclusive: &[],
    },
    ExtensionSpec {
        namespace: "lmf",
        name: "tab",
        version: 1,
        summary: "One tab inside lmf:tabs",
        attrs: &[AttrSpec {
            name: "title",
            ty: AttrType::Str,
            required: true,
            allowed: None,
        }],
        body: BodyRule::Required,
        required_parent: Some("lmf:tabs"),
        allowed_children: None,
        exclusive: &[],
    },
    ExtensionSpec {
        namespace: "lmf",
        name: "include",
        version: 1,
        summary: "Include a section or block of another document; the body is the fallback text",
        attrs: &[
            AttrSpec {
                name: "document",
                ty: AttrType::Str,
                required: true,
                allowed: None,
            },
            AttrSpec {
                name: "section",
                ty: AttrType::Str,
                required: false,
                allowed: None,
            },
            AttrSpec {
                name: "block",
                ty: AttrType::Str,
                required: false,
                allowed: None,
            },
        ],
        body: BodyRule::Optional,
        required_parent: None,
        allowed_children: None,
        exclusive: &[("section", "block")],
    },
    ExtensionSpec {
        namespace: "lmf",
        name: "math",
        version: 1,
        summary: "Display math source (LaTeX); rendering is optional",
        attrs: &[AttrSpec {
            name: "format",
            ty: AttrType::Str,
            required: false,
            allowed: Some(&["latex"]),
        }],
        body: BodyRule::Required,
        required_parent: None,
        allowed_children: None,
        exclusive: &[],
    },
    ExtensionSpec {
        namespace: "lmf",
        name: "diagram",
        version: 1,
        summary: "Diagram source (never executed); rendering is optional",
        attrs: &[AttrSpec {
            name: "type",
            ty: AttrType::Str,
            required: true,
            allowed: Some(&["mermaid"]),
        }],
        body: BodyRule::Required,
        required_parent: None,
        allowed_children: None,
        exclusive: &[],
    },
];

pub fn lookup(namespace: &str, name: &str) -> Option<&'static ExtensionSpec> {
    EXTENSIONS
        .iter()
        .find(|e| e.namespace == namespace && e.name == name)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn lookup_works() {
        assert!(lookup("lmf", "callout").is_some());
        assert!(lookup("lmf", "nope").is_none());
        assert!(lookup("vendor", "callout").is_none());
    }
}
