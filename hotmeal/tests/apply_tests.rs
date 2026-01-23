//! Tests for patch application.

use facet_testhelpers::test;
use hotmeal::Stem;
use hotmeal::arena_dom::parse;
use hotmeal::diff::{InsertContent, NodePath, NodeRef, Patch};

#[test]
fn test_parse_and_serialize_roundtrip() {
    let node = parse("<html><body><p>Hello</p></body></html>");
    assert_eq!(node.to_html(), "<body><p>Hello</p></body>");
}

#[test]
fn test_apply_set_text() {
    // Body has <p>Hello</p>, the text "Hello" is at path [0, 0] (p's first child)
    let mut node = parse("<html><body><p>Hello</p></body></html>");
    node.apply_patches(vec![Patch::SetText {
        path: NodePath(vec![0, 0]), // path to the text node inside <p>
        text: Stem::from("Goodbye"),
    }])
    .unwrap();
    assert_eq!(node.to_html(), "<body><p>Goodbye</p></body>");
}

#[test]
fn test_apply_set_attribute() {
    let mut node = parse("<html><body><div>Content</div></body></html>");
    node.apply_patches(vec![Patch::SetAttribute {
        path: NodePath(vec![0]),
        name: Stem::from("class"),
        value: Stem::from("highlight"),
    }])
    .unwrap();
    assert_eq!(
        node.to_html(),
        "<body><div class=\"highlight\">Content</div></body>"
    );
}

#[test]
fn test_apply_remove() {
    let mut node = parse("<html><body><p>First</p><p>Second</p></body></html>");
    node.apply_patches(vec![Patch::Remove {
        node: NodeRef::Path(NodePath(vec![1])),
    }])
    .unwrap();
    assert_eq!(node.to_html(), "<body><p>First</p></body>");
}

#[test]
fn test_apply_insert_element() {
    let mut node = parse("<html><body><p>First</p></body></html>");
    node.apply_patches(vec![Patch::InsertElement {
        at: NodeRef::Path(NodePath(vec![0])),
        tag: Stem::from("p"),
        attrs: vec![],
        children: vec![],
        detach_to_slot: Some(0), // Chawathe: displace First to slot 0
    }])
    .unwrap();
    // After insert with displacement, First is in slot 0, only empty <p> is in tree
    assert_eq!(node.to_html(), "<body><p></p></body>");
}

#[test]
fn test_apply_insert_element_no_displacement() {
    // Insert at end (no occupant) - no displacement needed
    let mut node = parse("<html><body><p>First</p></body></html>");
    node.apply_patches(vec![Patch::InsertElement {
        at: NodeRef::Path(NodePath(vec![1])),
        tag: Stem::from("p"),
        attrs: vec![],
        children: vec![],
        detach_to_slot: None,
    }])
    .unwrap();
    assert_eq!(node.to_html(), "<body><p>First</p><p></p></body>");
}

#[test]
fn test_apply_insert_element_with_children() {
    // Insert element with text content
    let mut node = parse("<html><body><p>First</p></body></html>");
    node.apply_patches(vec![Patch::InsertElement {
        at: NodeRef::Path(NodePath(vec![1])),
        tag: Stem::from("p"),
        attrs: vec![],
        children: vec![InsertContent::Text(Stem::from("Second"))],
        detach_to_slot: None,
    }])
    .unwrap();
    assert_eq!(node.to_html(), "<body><p>First</p><p>Second</p></body>");
}

#[test]
fn test_apply_insert_element_with_attrs() {
    // Insert element with attribute
    let mut node = parse("<html><body><p>First</p></body></html>");
    node.apply_patches(vec![Patch::InsertElement {
        at: NodeRef::Path(NodePath(vec![1])),
        tag: Stem::from("p"),
        attrs: vec![(Stem::from("class"), Stem::from("highlight"))],
        children: vec![InsertContent::Text(Stem::from("Second"))],
        detach_to_slot: None,
    }])
    .unwrap();
    assert_eq!(
        node.to_html(),
        "<body><p>First</p><p class=\"highlight\">Second</p></body>"
    );
}

#[test]
fn test_apply_insert_text() {
    let mut node = parse("<html><body><p>First</p></body></html>");
    node.apply_patches(vec![Patch::InsertText {
        at: NodeRef::Path(NodePath(vec![1])),
        text: Stem::from("Hello"),
        detach_to_slot: None,
    }])
    .unwrap();
    assert_eq!(node.to_html(), "<body><p>First</p>Hello</body>");
}

#[test]
fn test_parse_invalid_html_nesting() {
    // This HTML has a <div> inside a <strong> which is invalid per HTML spec
    // (block element inside inline element), but our arena_dom should handle it fine.
    let html = r#"<html><body><strong><div>nested div</div></strong></body></html>"#;
    let doc = parse(html);

    // Verify the structure is preserved
    let body = doc.body().expect("should have body");
    let strong = doc.first_child(body).expect("body should have children");

    // Check strong tag
    if let hotmeal::arena_dom::NodeKind::Element(elem) = &doc.get(strong).kind {
        assert_eq!(elem.tag.as_ref(), "strong");
    } else {
        panic!("Expected element, got {:?}", doc.get(strong).kind);
    }

    // Check div inside strong
    let div = doc
        .first_child(strong)
        .expect("strong should have children");
    if let hotmeal::arena_dom::NodeKind::Element(elem) = &doc.get(div).kind {
        assert_eq!(elem.tag.as_ref(), "div");
    } else {
        panic!("Expected div element, got {:?}", doc.get(div).kind);
    }

    // Check text content
    let text_node = doc.first_child(div).expect("div should have text");
    if let hotmeal::arena_dom::NodeKind::Text(text) = &doc.get(text_node).kind {
        assert_eq!(text.as_ref(), "nested div");
    } else {
        panic!("Expected text node");
    }
}
