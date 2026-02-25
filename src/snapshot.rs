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
    pub full: bool,
    pub mini: bool,
}

/// A node in the accessibility or React fiber tree
#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct TreeNode {
    pub(crate) name: String,
    pub(crate) is_component: bool,
    #[serde(default)]
    pub(crate) props: serde_json::Map<String, serde_json::Value>,
    #[serde(rename = "ref")]
    pub(crate) ref_id: Option<String>,
    #[serde(default)]
    pub(crate) box_rect: Option<BoxRect>,
    #[allow(dead_code)]
    pub(crate) role: Option<String>,
    pub(crate) aria_name: Option<String>,
    pub(crate) tag: Option<String>,
    #[serde(default)]
    pub(crate) html_attrs: Option<serde_json::Map<String, serde_json::Value>>,
    #[serde(default)]
    pub(crate) children: Vec<TreeNode>,
}

#[derive(Deserialize, Clone, Copy)]
#[serde(rename_all = "camelCase")]
pub(crate) struct BoxRect {
    pub(crate) x: f64,
    pub(crate) y: f64,
    pub(crate) width: f64,
    pub(crate) height: f64,
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

pub(crate) const INTERACTIVE_ROLES: &[&str] = &[
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

pub(crate) const INTERACTIVE_TAGS: &[&str] = &[
    "a", "button", "input", "select", "textarea", "details", "summary",
];

/// A node in the full DOM tree
#[derive(Deserialize)]
pub(crate) struct DomNode {
    pub(crate) tag: Option<String>,
    pub(crate) text: Option<String>,
    #[serde(default)]
    pub(crate) attrs: serde_json::Map<String, serde_json::Value>,
    #[serde(default)]
    pub(crate) children: Vec<DomNode>,
}

pub async fn take_snapshot(
    cdp: &mut CdpConnection,
    opts: &SnapshotOptions,
) -> anyhow::Result<String> {
    if opts.mini {
        take_mini_snapshot(cdp, opts).await
    } else if opts.full {
        take_full_snapshot(cdp, opts).await
    } else if opts.react {
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

    let tree = build_ax_tree(nodes);
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

/// Find root node IDs — nodes not referenced as children by any other node.
fn find_ax_root_ids(nodes: &[AXNode]) -> Vec<String> {
    let all_child_ids: std::collections::HashSet<&str> = nodes
        .iter()
        .flat_map(|n| n.child_ids.iter().map(|s| s.as_str()))
        .collect();

    let mut roots: Vec<String> = nodes
        .iter()
        .filter(|n| !all_child_ids.contains(n.node_id.as_str()))
        .map(|n| n.node_id.clone())
        .collect();

    if roots.is_empty() {
        roots.push(nodes[0].node_id.clone());
    }
    roots
}

/// Recursively extract a node and its children from the flat index.
fn extract_ax_node(
    id: &str,
    by_id: &mut std::collections::HashMap<String, AXNode>,
) -> Option<AXNode> {
    let mut node = by_id.remove(id)?;
    if !node.child_ids.is_empty() {
        let cids: Vec<String> = node.child_ids.clone();
        node.children = Some(
            cids.iter()
                .filter_map(|cid| extract_ax_node(cid, by_id))
                .collect(),
        );
    }
    Some(node)
}

/// Reconstruct nested tree from flat CDP array using child_ids references.
fn build_ax_tree(nodes: Vec<AXNode>) -> Vec<AXNode> {
    if nodes.is_empty() {
        return vec![];
    }

    let root_ids = find_ax_root_ids(&nodes);
    let mut by_id: std::collections::HashMap<String, AXNode> = nodes
        .into_iter()
        .map(|n| (n.node_id.clone(), n))
        .collect();

    root_ids
        .iter()
        .filter_map(|id| extract_ax_node(id, &mut by_id))
        .collect()
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

    if role == "none" || role == "Ignored" || role == "generic" {
        if let Some(children) = &node.children {
            for child in children {
                format_ax_node(child, depth, opts, lines);
            }
        }
        return;
    }

    if opts.interactive && !INTERACTIVE_ROLES.contains(&role.as_str()) {
        if let Some(children) = &node.children {
            for child in children {
                format_ax_node(child, depth, opts, lines);
            }
        }
        return;
    }

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

pub(crate) fn format_fiber_node(
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

    if opts.interactive && !node.is_component {
        let tag = node.tag.as_deref().unwrap_or("");
        if !INTERACTIVE_TAGS.contains(&tag) {
            for child in &node.children {
                format_fiber_node(child, depth, opts, lines);
            }
            return;
        }
    }

    if opts.compact && node.is_component && !has_interactive_descendant(node) {
        return;
    }

    let indent = "  ".repeat(depth);
    let mut line = format!("{}- {}", indent, node.name);

    if !node.is_component {
        if let Some(ref name) = node.aria_name {
            line.push_str(&format!(" \"{}\"", name));
        }
    }

    if let Some(ref r) = node.ref_id {
        line.push_str(&format!(" [ref={}]", r));
    }

    if let Some(b) = node.box_rect {
        let x = b.x.round() as i64;
        let y = b.y.round() as i64;
        let w = b.width.round() as i64;
        let h = b.height.round() as i64;
        line.push_str(&format!(" [x={} y={} w={} h={}]", x, y, w, h));
    }

    for (key, value) in &node.props {
        match value {
            serde_json::Value::String(s) => line.push_str(&format!(" {}=\"{}\"", key, s)),
            serde_json::Value::Number(n) => line.push_str(&format!(" {}={{{}}}", key, n)),
            serde_json::Value::Bool(b) => line.push_str(&format!(" {}={{{}}}", key, b)),
            serde_json::Value::Null => line.push_str(&format!(" {}={{null}}", key)),
            _ => line.push_str(&format!(" {}={{...}}", key)),
        }
    }

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

pub(crate) fn glob_match(pattern: &str, text: &str) -> bool {
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
                return false;
            }
            pos += idx + part.len();
        } else {
            return false;
        }
    }
    !parts.last().is_some_and(|p| !p.is_empty()) || pos == text.len()
}

/// Walk tree looking for nodes matching filter, output each match as a root subtree
pub(crate) fn collect_filtered_subtrees(
    node: &TreeNode,
    opts: &SnapshotOptions,
    lines: &mut Vec<String>,
) {
    let filter = opts.filter.as_deref().unwrap_or("");
    if name_matches_filter(&node.name, filter) {
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

pub(crate) fn has_interactive_descendant(node: &TreeNode) -> bool {
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

async fn take_full_snapshot(
    cdp: &mut CdpConnection,
    opts: &SnapshotOptions,
) -> anyhow::Result<String> {
    let script = build_dom_walker_script();
    let result = cdp.eval(&script).await?;
    let root: DomNode = serde_json::from_value(result)?;
    let mut lines = Vec::new();
    format_dom_node(&root, 0, opts, &mut lines);
    if lines.is_empty() {
        Ok("(empty page)".to_string())
    } else {
        Ok(lines.join("\n"))
    }
}

pub(crate) fn format_dom_node(
    node: &DomNode,
    depth: usize,
    opts: &SnapshotOptions,
    lines: &mut Vec<String>,
) {
    if let Some(max) = opts.max_depth {
        if depth > max {
            return;
        }
    }

    let indent = "  ".repeat(depth);

    if let Some(ref text) = node.text {
        lines.push(format!("{}- \"{}\"", indent, text));
        return;
    }

    let tag = node.tag.as_deref().unwrap_or("?");
    let mut line = format!("{}- {}", indent, tag);
    for (key, value) in &node.attrs {
        if let Some(s) = value.as_str() {
            line.push_str(&format!(" {}=\"{}\"", key, s));
        }
    }
    lines.push(line);

    for child in &node.children {
        format_dom_node(child, depth + 1, opts, lines);
    }
}

fn build_dom_walker_script() -> String {
    r#"(() => {
  function walk(node) {
    if (node.nodeType === 3) {
      const t = node.textContent.trim();
      if (!t) return null;
      return { text: t.length > 80 ? t.slice(0, 80) + '...' : t };
    }
    if (node.nodeType !== 1) return null;
    const tag = node.tagName.toLowerCase();
    if (['script','style','noscript','link','head','meta'].includes(tag)) return null;
    const attrs = {};
    for (const a of node.attributes) {
      if (a.name === 'style' || a.name === 'class') continue;
      if (a.name.startsWith('data-') && !a.name.startsWith('data-testid') && !a.name.startsWith('data-gc-')) continue;
      attrs[a.name] = a.value.length > 100 ? a.value.slice(0, 100) + '...' : a.value;
    }
    if (tag === 'svg') return { tag, attrs, children: [] };
    const children = [];
    for (const child of node.childNodes) {
      const c = walk(child);
      if (c) children.push(c);
    }
    return { tag, attrs, children };
  }
  return walk(document.documentElement);
})()"#
        .to_string()
}

const STRUCTURAL_TAGS: &[&str] = &[
    "div",
    "span",
    "p",
    "section",
    "main",
    "article",
    "header",
    "footer",
    "nav",
    "aside",
    "figure",
    "figcaption",
    "ul",
    "ol",
    "li",
    "dl",
    "dt",
    "dd",
    "table",
    "tbody",
    "thead",
    "tfoot",
    "tr",
    "td",
    "th",
    "center",
    "fieldset",
    "form",
];

fn is_structural(tag: &str) -> bool {
    STRUCTURAL_TAGS.contains(&tag)
}

/// Check if a node has attrs that are structurally meaningful (not just ARIA/a11y decoration).
fn has_meaningful_attrs(node: &DomNode) -> bool {
    node.attrs.keys().any(|k| {
        !k.starts_with("aria-")
            && !matches!(k.as_str(), "role" | "tabindex" | "hidden" | "dir" | "lang")
    })
}

/// Inline fragment nodes (tag=None, text=None) by promoting their children.
pub(crate) fn flatten_fragments(nodes: Vec<DomNode>) -> Vec<DomNode> {
    let mut result = Vec::new();
    for node in nodes {
        if node.tag.is_none() && node.text.is_none() {
            // Fragment — promote its children (recursively flatten)
            result.extend(flatten_fragments(node.children));
        } else {
            result.push(node);
        }
    }
    result
}

/// Collapse a DOM tree by removing empty structural nodes and collapsing single-child wrappers.
/// Returns None if the node should be entirely removed.
pub(crate) fn collapse_dom_tree(node: DomNode) -> Option<DomNode> {
    // Text nodes: keep as-is (they have no children to process)
    if node.tag.is_none() {
        // Remove empty text (no text content)
        if node.text.as_ref().map_or(true, |t| t.is_empty()) && !node.text.is_some() {
            return None;
        }
        return Some(node);
    }

    let tag = node.tag.as_deref().unwrap_or("");
    let structural = is_structural(tag);
    let no_meaningful_attrs = !has_meaningful_attrs(&node);

    // Process children recursively first (bottom-up)
    let collapsed_children: Vec<DomNode> = node
        .children
        .into_iter()
        .filter_map(collapse_dom_tree)
        .collect();

    // Rule 3 & 4: Remove empty structural elements with no attrs, no children, no text
    if structural && no_meaningful_attrs && collapsed_children.is_empty() && node.text.is_none() {
        return None;
    }

    // Rule 1: Structural wrapper collapse
    // Structural tag + no attrs + no text → promote children to parent
    if structural && no_meaningful_attrs && node.text.is_none() {
        // Return a fragment: tag=None, children hold the promoted nodes
        return Some(DomNode {
            tag: None,
            text: None,
            attrs: serde_json::Map::new(),
            children: collapsed_children,
        });
    }

    // Flatten any fragment children (from collapsed structural wrappers)
    let children = flatten_fragments(collapsed_children);

    Some(DomNode {
        tag: node.tag,
        text: node.text,
        attrs: node.attrs,
        children,
    })
}

async fn take_mini_snapshot(
    cdp: &mut CdpConnection,
    opts: &SnapshotOptions,
) -> anyhow::Result<String> {
    let script = build_dom_walker_script();
    let result = cdp.eval(&script).await?;
    let root: DomNode = serde_json::from_value(result)?;
    let collapsed = match collapse_dom_tree(root) {
        Some(node) => node,
        None => return Ok("(empty page)".to_string()),
    };
    // If root itself became a fragment, format each promoted child
    let roots = if collapsed.tag.is_none() && collapsed.text.is_none() {
        flatten_fragments(collapsed.children)
    } else {
        vec![collapsed]
    };
    let mut lines = Vec::new();
    for root in &roots {
        format_mini_node(root, 0, opts, &mut lines);
    }
    if lines.is_empty() {
        Ok("(empty page)".to_string())
    } else {
        Ok(lines.join("\n"))
    }
}

pub(crate) fn format_mini_node(
    node: &DomNode,
    depth: usize,
    opts: &SnapshotOptions,
    lines: &mut Vec<String>,
) {
    if let Some(max) = opts.max_depth {
        if depth > max {
            return;
        }
    }

    let indent = "  ".repeat(depth);

    // Text node
    if let Some(ref text) = node.text {
        lines.push(format!("{}- \"{}\"", indent, text));
        return;
    }

    let tag = node.tag.as_deref().unwrap_or("?");
    let mut line = format!("{}- {}", indent, tag);
    for (key, value) in &node.attrs {
        if let Some(s) = value.as_str() {
            line.push_str(&format!(" {}=\"{}\"", key, s));
        }
    }

    // Rule 2: Text promotion — single text child gets inlined
    if node.children.len() == 1 {
        if let Some(ref text) = node.children[0].text {
            if node.children[0].tag.is_none() {
                line.push_str(&format!(" \"{}\"", text));
                lines.push(line);
                return;
            }
        }
    }

    lines.push(line);

    for child in &node.children {
        format_mini_node(child, depth + 1, opts, lines);
    }
}
