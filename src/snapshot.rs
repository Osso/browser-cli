use serde::Deserialize;

use crate::cdp::CdpConnection;

/// Options for snapshot command
#[derive(Clone)]
pub struct SnapshotOptions {
    pub interactive: bool,
    pub compact: bool,
    pub react: bool,
    pub max_depth: Option<usize>,
    pub filter: Option<String>,
}

/// A node in the accessibility or React fiber tree
#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct TreeNode {
    name: String,
    is_component: bool,
    #[serde(default)]
    props: serde_json::Map<String, serde_json::Value>,
    #[serde(rename = "ref")]
    ref_id: Option<String>,
    #[allow(dead_code)]
    role: Option<String>,
    aria_name: Option<String>,
    tag: Option<String>,
    #[serde(default)]
    html_attrs: Option<serde_json::Map<String, serde_json::Value>>,
    #[serde(default)]
    children: Vec<TreeNode>,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct FiberResult {
    found: bool,
    tree: Vec<TreeNode>,
    all_minified: bool,
}

/// CDP Accessibility tree node
#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct AXNode {
    #[allow(dead_code)]
    node_id: String,
    role: Option<AXValue>,
    name: Option<AXValue>,
    #[serde(default)]
    children: Option<Vec<AXNode>>,
    #[serde(default)]
    #[allow(dead_code)]
    child_ids: Vec<String>,
}

#[derive(Deserialize)]
struct AXValue {
    value: Option<serde_json::Value>,
}

const INTERACTIVE_ROLES: &[&str] = &[
    "button",
    "link",
    "textbox",
    "checkbox",
    "radio",
    "combobox",
    "listbox",
    "menuitem",
    "menuitemcheckbox",
    "menuitemradio",
    "option",
    "searchbox",
    "slider",
    "spinbutton",
    "switch",
    "tab",
    "treeitem",
];

const INTERACTIVE_TAGS: &[&str] = &[
    "a", "button", "input", "select", "textarea", "details", "summary",
];

pub async fn take_snapshot(
    cdp: &mut CdpConnection,
    opts: &SnapshotOptions,
) -> anyhow::Result<String> {
    if opts.react {
        take_react_snapshot(cdp, opts).await
    } else {
        take_aria_snapshot(cdp, opts).await
    }
}

async fn take_aria_snapshot(
    cdp: &mut CdpConnection,
    opts: &SnapshotOptions,
) -> anyhow::Result<String> {
    let result = cdp
        .send("Accessibility.getFullAXTree", serde_json::json!({}))
        .await?;

    let nodes_val = result
        .get("nodes")
        .ok_or_else(|| anyhow::anyhow!("No accessibility nodes returned"))?;

    let nodes: Vec<AXNode> = serde_json::from_value(nodes_val.clone())?;
    if nodes.is_empty() {
        return Ok("(empty page)".to_string());
    }

    let tree = build_ax_tree(&nodes);
    let mut lines = Vec::new();
    for node in &tree {
        format_ax_node(node, 0, opts, &mut lines);
    }

    if lines.is_empty() {
        Ok("(empty page)".to_string())
    } else {
        Ok(lines.join("\n"))
    }
}

fn build_ax_tree(nodes: &[AXNode]) -> Vec<&AXNode> {
    // The first node is typically the root. Build children from child_ids.
    // For simplicity, just return the root and let format_ax_node recurse via child_ids.
    if nodes.is_empty() {
        return vec![];
    }
    // Return top-level root
    vec![&nodes[0]]
}

fn ax_value_str(v: &Option<AXValue>) -> Option<String> {
    v.as_ref()
        .and_then(|av| av.value.as_ref())
        .and_then(|val| val.as_str().map(String::from))
}

fn format_ax_node(node: &AXNode, depth: usize, opts: &SnapshotOptions, lines: &mut Vec<String>) {
    if let Some(max) = opts.max_depth {
        if depth > max {
            return;
        }
    }

    let role = ax_value_str(&node.role).unwrap_or_default();
    let name = ax_value_str(&node.name).unwrap_or_default();

    // Skip ignored/none roles
    if role == "none" || role == "Ignored" || role == "generic" {
        if let Some(children) = &node.children {
            for child in children {
                format_ax_node(child, depth, opts, lines);
            }
        }
        return;
    }

    // Interactive filter
    if opts.interactive && !INTERACTIVE_ROLES.contains(&role.as_str()) {
        if let Some(children) = &node.children {
            for child in children {
                format_ax_node(child, depth, opts, lines);
            }
        }
        return;
    }

    // Compact: skip structural nodes with no text
    if opts.compact && name.is_empty() && !INTERACTIVE_ROLES.contains(&role.as_str()) {
        if let Some(children) = &node.children {
            for child in children {
                format_ax_node(child, depth, opts, lines);
            }
        }
        return;
    }

    let indent = "  ".repeat(depth);
    if name.is_empty() {
        lines.push(format!("{}- {}", indent, role));
    } else {
        lines.push(format!("{}- {} \"{}\"", indent, role, name));
    }

    if let Some(children) = &node.children {
        for child in children {
            format_ax_node(child, depth + 1, opts, lines);
        }
    }
}

async fn take_react_snapshot(
    cdp: &mut CdpConnection,
    opts: &SnapshotOptions,
) -> anyhow::Result<String> {
    let js_depth = opts.max_depth.unwrap_or(200);
    let script = build_fiber_walker_script(js_depth);
    let result = cdp.eval(&script).await?;

    let fiber: FiberResult = match serde_json::from_value(result.clone()) {
        Ok(f) => f,
        Err(_) => {
            // JS eval returned non-fiber result (React not found or error)
            return take_aria_snapshot(
                cdp,
                &SnapshotOptions {
                    react: false,
                    ..opts.clone()
                },
            )
            .await;
        }
    };

    if !fiber.found {
        return take_aria_snapshot(
            cdp,
            &SnapshotOptions {
                react: false,
                ..opts.clone()
            },
        )
        .await;
    }

    let mut lines = Vec::new();
    if fiber.all_minified {
        lines.push("# Warning: All component names are minified (production build)".to_string());
    }

    for node in &fiber.tree {
        if opts.filter.is_some() {
            collect_filtered_subtrees(node, opts, &mut lines);
        } else {
            format_fiber_node(node, 0, opts, &mut lines);
        }
    }

    if lines.is_empty() {
        Ok("(empty)".to_string())
    } else {
        Ok(lines.join("\n"))
    }
}

fn format_fiber_node(
    node: &TreeNode,
    depth: usize,
    opts: &SnapshotOptions,
    lines: &mut Vec<String>,
) {
    if let Some(max) = opts.max_depth {
        if depth > max {
            return;
        }
    }

    // Interactive filter for host elements
    if opts.interactive && !node.is_component {
        let tag = node.tag.as_deref().unwrap_or("");
        if !INTERACTIVE_TAGS.contains(&tag) {
            for child in &node.children {
                format_fiber_node(child, depth, opts, lines);
            }
            return;
        }
    }

    // Compact: skip components with no interactive descendants
    if opts.compact && node.is_component && !has_interactive_descendant(node) {
        return;
    }

    let indent = "  ".repeat(depth);
    let mut line = format!("{}- {}", indent, node.name);

    // Accessible name for host elements
    if !node.is_component {
        if let Some(ref name) = node.aria_name {
            line.push_str(&format!(" \"{}\"", name));
        }
    }

    // Ref
    if let Some(ref r) = node.ref_id {
        line.push_str(&format!(" [ref={}]", r));
    }

    // Props
    for (key, value) in &node.props {
        match value {
            serde_json::Value::String(s) => line.push_str(&format!(" {}=\"{}\"", key, s)),
            serde_json::Value::Number(n) => line.push_str(&format!(" {}={{{}}}", key, n)),
            serde_json::Value::Bool(b) => line.push_str(&format!(" {}={{{}}}", key, b)),
            serde_json::Value::Null => line.push_str(&format!(" {}={{null}}", key)),
            _ => line.push_str(&format!(" {}={{...}}", key)),
        }
    }

    // HTML attrs for host elements
    if let Some(ref attrs) = node.html_attrs {
        for (key, value) in attrs {
            if node.props.contains_key(key) {
                continue;
            }
            if let Some(s) = value.as_str() {
                line.push_str(&format!(" {}=\"{}\"", key, s));
            }
        }
    }

    lines.push(line);

    for child in &node.children {
        format_fiber_node(child, depth + 1, opts, lines);
    }
}

fn name_matches_filter(name: &str, filter: &str) -> bool {
    let name_lower = name.to_ascii_lowercase();
    let filter_lower = filter.to_ascii_lowercase();
    if filter.contains('*') {
        glob_match(&filter_lower, &name_lower)
    } else {
        name_lower.contains(&filter_lower)
    }
}

fn glob_match(pattern: &str, text: &str) -> bool {
    let parts: Vec<&str> = pattern.split('*').collect();
    if parts.len() == 1 {
        return text == pattern;
    }
    let mut pos = 0;
    for (i, part) in parts.iter().enumerate() {
        if part.is_empty() {
            continue;
        }
        if let Some(idx) = text[pos..].find(part) {
            if i == 0 && idx != 0 {
                return false; // pattern doesn't start with *, must match beginning
            }
            pos += idx + part.len();
        } else {
            return false;
        }
    }
    // If pattern doesn't end with *, text must end at pos
    !parts.last().is_some_and(|p| !p.is_empty()) || pos == text.len()
}

/// Walk tree looking for nodes matching filter, output each match as a root subtree
fn collect_filtered_subtrees(node: &TreeNode, opts: &SnapshotOptions, lines: &mut Vec<String>) {
    let filter = opts.filter.as_deref().unwrap_or("");
    if name_matches_filter(&node.name, filter) {
        // Output this subtree without the filter (so children aren't re-filtered)
        let no_filter_opts = SnapshotOptions {
            filter: None,
            ..opts.clone()
        };
        format_fiber_node(node, 0, &no_filter_opts, lines);
    } else {
        for child in &node.children {
            collect_filtered_subtrees(child, opts, lines);
        }
    }
}

fn has_interactive_descendant(node: &TreeNode) -> bool {
    if !node.is_component {
        if let Some(ref tag) = node.tag {
            if INTERACTIVE_TAGS.contains(&tag.as_str()) {
                return true;
            }
        }
    }
    node.children.iter().any(has_interactive_descendant)
}

fn build_fiber_walker_script(max_depth: usize) -> String {
    format!(
        "globalThis.__MAX_DEPTH = {};\n{}",
        max_depth,
        include_str!("fiber_walker.js")
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    fn default_opts() -> SnapshotOptions {
        SnapshotOptions {
            interactive: false,
            compact: false,
            react: false,
            max_depth: None,
            filter: None,
        }
    }

    fn make_component(name: &str, children: Vec<TreeNode>) -> TreeNode {
        TreeNode {
            name: name.to_string(),
            is_component: true,
            props: serde_json::Map::new(),
            ref_id: None,
            role: None,
            aria_name: None,
            tag: None,
            html_attrs: None,
            children,
        }
    }

    fn make_host(
        tag: &str,
        aria_name: Option<&str>,
        ref_id: Option<&str>,
        children: Vec<TreeNode>,
    ) -> TreeNode {
        TreeNode {
            name: tag.to_string(),
            is_component: false,
            props: serde_json::Map::new(),
            ref_id: ref_id.map(String::from),
            role: None,
            aria_name: aria_name.map(String::from),
            tag: Some(tag.to_string()),
            html_attrs: None,
            children,
        }
    }

    fn format_tree(nodes: &[TreeNode], opts: &SnapshotOptions) -> Vec<String> {
        let mut lines = Vec::new();
        for node in nodes {
            if opts.filter.is_some() {
                collect_filtered_subtrees(node, opts, &mut lines);
            } else {
                format_fiber_node(node, 0, opts, &mut lines);
            }
        }
        lines
    }

    #[test]
    fn test_basic_tree_output() {
        let tree = vec![make_component(
            "App",
            vec![make_component(
                "NavBar",
                vec![make_host("button", Some("Click me"), Some("e1"), vec![])],
            )],
        )];
        let lines = format_tree(&tree, &default_opts());
        assert_eq!(
            lines,
            vec!["- App", "  - NavBar", "    - button \"Click me\" [ref=e1]",]
        );
    }

    #[test]
    fn test_interactive_filter() {
        let tree = vec![make_component(
            "App",
            vec![
                make_host(
                    "div",
                    None,
                    None,
                    vec![make_host("button", Some("OK"), Some("e1"), vec![])],
                ),
                make_host("a", Some("Home"), Some("e2"), vec![]),
            ],
        )];
        // div has no tag in INTERACTIVE_TAGS so it's skipped, but
        // since it's a host element and not interactive, it's excluded
        // and children promoted. But div doesn't have tag in INTERACTIVE_TAGS...
        // Actually: interactive filter only applies to non-component nodes.
        // div is not interactive -> skip, promote children (button).
        // button IS interactive -> include.
        // a IS interactive -> include.
        let opts = SnapshotOptions {
            interactive: true,
            ..default_opts()
        };
        let lines = format_tree(&tree, &opts);
        assert_eq!(
            lines,
            vec![
                "- App",
                "  - button \"OK\" [ref=e1]",
                "  - a \"Home\" [ref=e2]",
            ]
        );
    }

    #[test]
    fn test_compact_filter() {
        let tree = vec![
            make_component(
                "HasButton",
                vec![make_host("button", Some("OK"), Some("e1"), vec![])],
            ),
            make_component("NoInteractive", vec![make_component("Inner", vec![])]),
        ];
        let opts = SnapshotOptions {
            compact: true,
            ..default_opts()
        };
        let lines = format_tree(&tree, &opts);
        // NoInteractive has no interactive descendants -> skipped
        assert_eq!(lines, vec!["- HasButton", "  - button \"OK\" [ref=e1]",]);
    }

    #[test]
    fn test_max_depth() {
        let tree = vec![make_component(
            "L0",
            vec![make_component(
                "L1",
                vec![make_component("L2", vec![make_component("L3", vec![])])],
            )],
        )];
        let opts = SnapshotOptions {
            max_depth: Some(2),
            ..default_opts()
        };
        let lines = format_tree(&tree, &opts);
        assert_eq!(lines, vec!["- L0", "  - L1", "    - L2"]);
    }

    #[test]
    fn test_filter_substring() {
        let tree = vec![make_component(
            "App",
            vec![
                make_component(
                    "ComicCard",
                    vec![make_host("a", Some("Comic 1"), Some("e1"), vec![])],
                ),
                make_component(
                    "NavBar",
                    vec![make_host("button", Some("Menu"), Some("e2"), vec![])],
                ),
                make_component(
                    "ComicList",
                    vec![make_component(
                        "ComicCard",
                        vec![make_host("a", Some("Comic 2"), Some("e3"), vec![])],
                    )],
                ),
            ],
        )];
        let opts = SnapshotOptions {
            filter: Some("ComicCard".to_string()),
            ..default_opts()
        };
        let lines = format_tree(&tree, &opts);
        // Should find both ComicCard instances, each as a root subtree
        assert_eq!(
            lines,
            vec![
                "- ComicCard",
                "  - a \"Comic 1\" [ref=e1]",
                "- ComicCard",
                "  - a \"Comic 2\" [ref=e3]",
            ]
        );
    }

    #[test]
    fn test_filter_case_insensitive() {
        let tree = vec![make_component(
            "NavBar",
            vec![make_host("button", Some("Menu"), Some("e1"), vec![])],
        )];
        let opts = SnapshotOptions {
            filter: Some("navbar".to_string()),
            ..default_opts()
        };
        let lines = format_tree(&tree, &opts);
        assert_eq!(lines, vec!["- NavBar", "  - button \"Menu\" [ref=e1]",]);
    }

    #[test]
    fn test_filter_glob_prefix() {
        let tree = vec![make_component(
            "App",
            vec![
                make_component("ComicCard", vec![]),
                make_component("ComicList", vec![]),
                make_component("NavBar", vec![]),
            ],
        )];
        let opts = SnapshotOptions {
            filter: Some("Comic*".to_string()),
            ..default_opts()
        };
        let lines = format_tree(&tree, &opts);
        assert_eq!(lines, vec!["- ComicCard", "- ComicList"]);
    }

    #[test]
    fn test_filter_glob_suffix() {
        let tree = vec![make_component(
            "App",
            vec![
                make_component("ComicCard", vec![]),
                make_component("ArtistCard", vec![]),
                make_component("NavBar", vec![]),
            ],
        )];
        let opts = SnapshotOptions {
            filter: Some("*Card".to_string()),
            ..default_opts()
        };
        let lines = format_tree(&tree, &opts);
        assert_eq!(lines, vec!["- ComicCard", "- ArtistCard"]);
    }

    #[test]
    fn test_filter_no_match() {
        let tree = vec![make_component(
            "App",
            vec![make_component("NavBar", vec![])],
        )];
        let opts = SnapshotOptions {
            filter: Some("DoesNotExist".to_string()),
            ..default_opts()
        };
        let lines = format_tree(&tree, &opts);
        assert!(lines.is_empty());
    }

    #[test]
    fn test_props_rendering() {
        let mut props = serde_json::Map::new();
        props.insert(
            "slug".to_string(),
            serde_json::Value::String("batman".to_string()),
        );
        props.insert("count".to_string(), serde_json::json!(42));
        props.insert("active".to_string(), serde_json::Value::Bool(true));
        props.insert("data".to_string(), serde_json::json!({"nested": true}));
        let tree = vec![TreeNode {
            name: "Comic".to_string(),
            is_component: true,
            props,
            ref_id: None,
            role: None,
            aria_name: None,
            tag: None,
            html_attrs: None,
            children: vec![],
        }];
        let lines = format_tree(&tree, &default_opts());
        let line = &lines[0];
        assert!(line.contains("slug=\"batman\""));
        assert!(line.contains("count={42}"));
        assert!(line.contains("active={true}"));
        assert!(line.contains("data={...}"));
    }

    #[test]
    fn test_html_attrs_on_host() {
        let mut html_attrs = serde_json::Map::new();
        html_attrs.insert(
            "href".to_string(),
            serde_json::Value::String("/home".to_string()),
        );
        let tree = vec![TreeNode {
            name: "a".to_string(),
            is_component: false,
            props: serde_json::Map::new(),
            ref_id: Some("e1".to_string()),
            role: None,
            aria_name: Some("Home".to_string()),
            tag: Some("a".to_string()),
            html_attrs: Some(html_attrs),
            children: vec![],
        }];
        let lines = format_tree(&tree, &default_opts());
        assert_eq!(lines, vec!["- a \"Home\" [ref=e1] href=\"/home\""]);
    }

    #[test]
    fn test_glob_match_exact() {
        assert!(glob_match("hello", "hello"));
        assert!(!glob_match("hello", "world"));
    }

    #[test]
    fn test_glob_match_wildcard() {
        assert!(glob_match("comic*", "comiccard"));
        assert!(glob_match("*card", "comiccard"));
        assert!(glob_match("*mic*", "comiccard"));
        assert!(glob_match("comic*card", "comiccard"));
        assert!(!glob_match("comic*", "navbarcomic"));
        assert!(!glob_match("*card", "cardnav"));
    }

    #[test]
    fn test_has_interactive_descendant() {
        let tree = make_component(
            "Wrapper",
            vec![make_component(
                "Inner",
                vec![make_host("button", Some("OK"), Some("e1"), vec![])],
            )],
        );
        assert!(has_interactive_descendant(&tree));

        let no_interactive = make_component("Wrapper", vec![make_component("Inner", vec![])]);
        assert!(!has_interactive_descendant(&no_interactive));
    }
}
