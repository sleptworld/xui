//! The host widgets' tag names: the one list of names the macros keep. The
//! set of host widgets is closed, and knowing them by name is what lets a
//! component's view be compiled to a static template: which nodes are hosts,
//! and which are components, is decided before any type is known. The
//! generated code asserts that a tag treated as a host is one.
use syn::Path;

/// What a host tag's body can be.
#[derive(Clone, Copy, PartialEq, Eq)]
pub(crate) enum HostKind {
    /// Takes children.
    Parent,
    /// Takes no body.
    Leaf,
    /// `<text>{content}</text>`: its body is its content.
    Text,
}

/// The host widgets, by tag. The set is closed, so the macro can tell a host
/// from a component by name; the generated code asserts each one
/// (`xui_core::template::host`).
pub(crate) fn host_kind(tag: &Path) -> Option<HostKind> {
    // Qualified application components may share a built-in tag's name.
    // Only canonical runtime paths are eligible for host-template lowering.
    if tag.segments.len() > 1 {
        let names: Vec<_> = tag.segments.iter().map(|s| s.ident.to_string()).collect();
        let runtime = matches!(names[0].as_str(), "xui" | "xui_core");
        if !runtime || !(names.len() == 2 || names.get(1).is_some_and(|s| s == "prelude")) {
            return None;
        }
    }
    let segment = tag.segments.last()?;
    if !segment.arguments.is_none() {
        return None;
    }
    Some(match segment.ident.to_string().as_str() {
        "container" | "row" | "column" | "center" | "grid" | "z_stack" | "image" => {
            HostKind::Parent
        }
        "text_input" | "canvas" | "icon" => HostKind::Leaf,
        "text" => HostKind::Text,
        _ => return None,
    })
}
