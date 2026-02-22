use crate::snapshot::{
    collapse_dom_tree, collect_filtered_subtrees, flatten_fragments, format_dom_node,
    format_fiber_node, format_mini_node, glob_match, has_interactive_descendant, DomNode,
    SnapshotOptions, TreeNode,
};

fn default_opts() -> SnapshotOptions {
    SnapshotOptions {
        interactive: false,
        compact: false,
        react: false,
        max_depth: None,
        filter: None,
        full: false,
        mini: false,
    }
}

fn make_component(name: &str, children: Vec<TreeNode>) -> TreeNode {
    TreeNode {
        name: name.to_string(),
        is_component: true,
        props: serde_json::Map::new(),
        ref_id: None,
        box_rect: None,
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
        box_rect: None,
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

fn make_dom_element(tag: &str, attrs: Vec<(&str, &str)>, children: Vec<DomNode>) -> DomNode {
    let mut map = serde_json::Map::new();
    for (k, v) in attrs {
        map.insert(k.to_string(), serde_json::Value::String(v.to_string()));
    }
    DomNode { tag: Some(tag.to_string()), text: None, attrs: map, children }
}

fn make_dom_text(text: &str) -> DomNode {
    DomNode {
        tag: None,
        text: Some(text.to_string()),
        attrs: serde_json::Map::new(),
        children: vec![],
    }
}

fn format_dom(node: &DomNode, opts: &SnapshotOptions) -> Vec<String> {
    let mut lines = Vec::new();
    format_dom_node(node, 0, opts, &mut lines);
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
    let opts = SnapshotOptions { interactive: true, ..default_opts() };
    let lines = format_tree(&tree, &opts);
    assert_eq!(
        lines,
        vec!["- App", "  - button \"OK\" [ref=e1]", "  - a \"Home\" [ref=e2]",]
    );
}

#[test]
fn test_compact_filter() {
    let tree = vec![
        make_component("HasButton", vec![make_host("button", Some("OK"), Some("e1"), vec![])]),
        make_component("NoInteractive", vec![make_component("Inner", vec![])]),
    ];
    let opts = SnapshotOptions { compact: true, ..default_opts() };
    let lines = format_tree(&tree, &opts);
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
    let opts = SnapshotOptions { max_depth: Some(2), ..default_opts() };
    let lines = format_tree(&tree, &opts);
    assert_eq!(lines, vec!["- L0", "  - L1", "    - L2"]);
}

#[test]
fn test_filter_substring() {
    let tree = vec![make_component(
        "App",
        vec![
            make_component("ComicCard", vec![make_host("a", Some("Comic 1"), Some("e1"), vec![])]),
            make_component("NavBar", vec![make_host("button", Some("Menu"), Some("e2"), vec![])]),
            make_component(
                "ComicList",
                vec![make_component(
                    "ComicCard",
                    vec![make_host("a", Some("Comic 2"), Some("e3"), vec![])],
                )],
            ),
        ],
    )];
    let opts = SnapshotOptions { filter: Some("ComicCard".to_string()), ..default_opts() };
    let lines = format_tree(&tree, &opts);
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
    let tree =
        vec![make_component("NavBar", vec![make_host("button", Some("Menu"), Some("e1"), vec![])])];
    let opts = SnapshotOptions { filter: Some("navbar".to_string()), ..default_opts() };
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
    let opts = SnapshotOptions { filter: Some("Comic*".to_string()), ..default_opts() };
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
    let opts = SnapshotOptions { filter: Some("*Card".to_string()), ..default_opts() };
    let lines = format_tree(&tree, &opts);
    assert_eq!(lines, vec!["- ComicCard", "- ArtistCard"]);
}

#[test]
fn test_filter_no_match() {
    let tree =
        vec![make_component("App", vec![make_component("NavBar", vec![])])];
    let opts = SnapshotOptions { filter: Some("DoesNotExist".to_string()), ..default_opts() };
    let lines = format_tree(&tree, &opts);
    assert!(lines.is_empty());
}

#[test]
fn test_props_rendering() {
    let mut props = serde_json::Map::new();
    props.insert("slug".to_string(), serde_json::Value::String("batman".to_string()));
    props.insert("count".to_string(), serde_json::json!(42));
    props.insert("active".to_string(), serde_json::Value::Bool(true));
    props.insert("data".to_string(), serde_json::json!({"nested": true}));
    let tree = vec![TreeNode {
        name: "Comic".to_string(),
        is_component: true,
        props,
        ref_id: None,
        box_rect: None,
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
    html_attrs.insert("href".to_string(), serde_json::Value::String("/home".to_string()));
    let tree = vec![TreeNode {
        name: "a".to_string(),
        is_component: false,
        props: serde_json::Map::new(),
        ref_id: Some("e1".to_string()),
        box_rect: None,
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
        vec![make_component("Inner", vec![make_host("button", Some("OK"), Some("e1"), vec![])])],
    );
    assert!(has_interactive_descendant(&tree));

    let no_interactive = make_component("Wrapper", vec![make_component("Inner", vec![])]);
    assert!(!has_interactive_descendant(&no_interactive));
}

// DOM snapshot tests

#[test]
fn test_dom_basic_tree() {
    let root = make_dom_element(
        "html",
        vec![],
        vec![make_dom_element(
            "body",
            vec![],
            vec![make_dom_element(
                "div",
                vec![("id", "main")],
                vec![make_dom_element("p", vec![], vec![make_dom_text("Hello world")])],
            )],
        )],
    );
    let lines = format_dom(&root, &default_opts());
    assert_eq!(
        lines,
        vec![
            "- html",
            "  - body",
            "    - div id=\"main\"",
            "      - p",
            "        - \"Hello world\"",
        ]
    );
}

#[test]
fn test_dom_attrs_rendered() {
    let node = make_dom_element("a", vec![("href", "/home"), ("data-testid", "nav-link")], vec![]);
    let lines = format_dom(&node, &default_opts());
    assert_eq!(lines.len(), 1);
    assert!(lines[0].contains("href=\"/home\""));
    assert!(lines[0].contains("data-testid=\"nav-link\""));
}

#[test]
fn test_dom_depth_limit() {
    let root = make_dom_element(
        "div",
        vec![],
        vec![make_dom_element(
            "section",
            vec![],
            vec![make_dom_element("p", vec![], vec![make_dom_text("deep")])],
        )],
    );
    let opts = SnapshotOptions { max_depth: Some(1), ..default_opts() };
    let lines = format_dom(&root, &opts);
    assert_eq!(lines, vec!["- div", "  - section"]);
}

// Mini snapshot tests

fn collapse_and_format_mini(tree: DomNode, opts: &SnapshotOptions) -> Vec<String> {
    let collapsed = collapse_dom_tree(tree).unwrap();
    let roots = if collapsed.tag.is_none() && collapsed.text.is_none() {
        flatten_fragments(collapsed.children)
    } else {
        vec![collapsed]
    };
    let mut lines = Vec::new();
    for root in &roots {
        format_mini_node(root, 0, opts, &mut lines);
    }
    lines
}

#[test]
fn test_mini_collapse_single_child_chain() {
    // div > div > div > a href="/" with text "Home"
    // Should collapse to just: a href="/" "Home"
    let tree = make_dom_element(
        "div",
        vec![],
        vec![make_dom_element(
            "div",
            vec![],
            vec![make_dom_element(
                "div",
                vec![],
                vec![make_dom_element("a", vec![("href", "/")], vec![make_dom_text("Home")])],
            )],
        )],
    );
    let opts = SnapshotOptions { mini: true, ..default_opts() };
    let lines = collapse_and_format_mini(tree, &opts);
    assert_eq!(lines, vec!["- a href=\"/\" \"Home\""]);
}

#[test]
fn test_mini_preserves_attrs() {
    // div id="root" > a href="/" > "Home"
    // div has attrs so should NOT be collapsed
    let tree = make_dom_element(
        "div",
        vec![("id", "root")],
        vec![make_dom_element("a", vec![("href", "/")], vec![make_dom_text("Home")])],
    );
    let opts = SnapshotOptions { mini: true, ..default_opts() };
    let lines = collapse_and_format_mini(tree, &opts);
    assert_eq!(lines, vec!["- div id=\"root\"", "  - a href=\"/\" \"Home\"",]);
}

#[test]
fn test_mini_removes_empty() {
    // div id="root" > (empty div, a href="/")
    // empty div removed, a promoted as only child of div id="root"
    let tree = make_dom_element(
        "div",
        vec![("id", "root")],
        vec![
            make_dom_element("div", vec![], vec![]),
            make_dom_element("a", vec![("href", "/")], vec![make_dom_text("Click")]),
        ],
    );
    let opts = SnapshotOptions { mini: true, ..default_opts() };
    let lines = collapse_and_format_mini(tree, &opts);
    assert_eq!(lines, vec!["- div id=\"root\"", "  - a href=\"/\" \"Click\"",]);
}

#[test]
fn test_mini_multi_child_wrapper_promotes_children() {
    // div > (a + button) — bare div promotes both children
    // Wrapping in a div with attrs to test that promotion works
    let tree = make_dom_element(
        "div",
        vec![("id", "root")],
        vec![make_dom_element(
            "div",
            vec![],
            vec![
                make_dom_element("a", vec![("href", "/a")], vec![make_dom_text("Link")]),
                make_dom_element("button", vec![], vec![make_dom_text("Click")]),
            ],
        )],
    );
    let opts = SnapshotOptions { mini: true, ..default_opts() };
    let lines = collapse_and_format_mini(tree, &opts);
    // Inner div collapsed, children promoted into div#root
    assert_eq!(
        lines,
        vec![
            "- div id=\"root\"",
            "  - a href=\"/a\" \"Link\"",
            "  - button \"Click\"",
        ]
    );
}

#[test]
fn test_mini_real_world_nav_link() {
    // Simulates: a > div role="group" > div > (img + div > p > "DC")
    // Should collapse to: a > (img + "DC")
    let tree = make_dom_element(
        "a",
        vec![("aria-label", "DC"), ("href", "/channel/dc")],
        vec![make_dom_element(
            "div",
            vec![("role", "group")],
            vec![make_dom_element(
                "div",
                vec![],
                vec![
                    make_dom_element(
                        "div",
                        vec![],
                        vec![make_dom_element("img", vec![("alt", "icon"), ("src", "x.svg")], vec![])],
                    ),
                    make_dom_element(
                        "div",
                        vec![],
                        vec![make_dom_element("p", vec![], vec![make_dom_text("DC")])],
                    ),
                ],
            )],
        )],
    );
    let opts = SnapshotOptions { mini: true, ..default_opts() };
    let lines = collapse_and_format_mini(tree, &opts);
    // role is not meaningful, so div[role=group] collapses too
    assert_eq!(
        lines,
        vec![
            "- a aria-label=\"DC\" href=\"/channel/dc\"",
            "  - img alt=\"icon\" src=\"x.svg\"",
            "  - \"DC\"",
        ]
    );
}
