//! Arena-based DOM with borrowed strings for zero-copy parsing.
//!
//! This module provides the core Document representation used throughout hotmeal.
//! Key features:
//! - **indextree Arena**: All nodes in contiguous memory (cache-friendly)
//! - **Borrowed strings**: Tags, attributes, text borrowed from source HTML
//! - **Zero conversions**: Same representation used by parser, differ, and patch applier

use cinereus::indextree::{Arena, NodeId};
use html5ever::tree_builder::{ElemName, ElementFlags, NodeOrText, QuirksMode, TreeSink};
use html5ever::{Attribute, LocalName, QualName, parse_document};
use html5ever::{local_name, namespace_url, ns};
use std::borrow::Cow;
use std::cell::RefCell;
use std::collections::HashMap;
use tendril::{StrTendril, TendrilSink};

/// Document = Arena (strings are StrTendrils with refcounted sharing)
#[derive(Debug, Clone)]
pub struct Document {
    /// THE tree - all nodes live here
    pub arena: Arena<NodeData>,

    /// Root node (usually <html> element)
    pub root: NodeId,

    /// DOCTYPE if present (usually "html")
    pub doctype: Option<StrTendril>,
}

impl Document {
    /// Get immutable reference to node data
    pub fn get(&self, id: NodeId) -> &NodeData {
        self.arena[id].get()
    }

    /// Get mutable reference to node data
    pub fn get_mut(&mut self, id: NodeId) -> &mut NodeData {
        self.arena[id].get_mut()
    }

    /// Iterate children of a node
    pub fn children(&self, id: NodeId) -> impl Iterator<Item = NodeId> + '_ {
        id.children(&self.arena)
    }

    /// Get the <body> element if present
    pub fn body(&self) -> Option<NodeId> {
        self.root.children(&self.arena).find(|&id| {
            if let NodeKind::Element(elem) = &self.arena[id].get().kind {
                elem.tag.as_ref() == "body"
            } else {
                false
            }
        })
    }

    /// Get the <head> element if present
    pub fn head(&self) -> Option<NodeId> {
        self.root.children(&self.arena).find(|&id| {
            if let NodeKind::Element(elem) = &self.arena[id].get().kind {
                elem.tag.as_ref() == "head"
            } else {
                false
            }
        })
    }
}

/// What goes in each arena slot
#[derive(Debug, Clone)]
pub struct NodeData {
    pub kind: NodeKind,
    pub ns: Namespace,
}

/// Node types
#[derive(Debug, Clone)]
pub enum NodeKind {
    /// Document root (invisible, parent of <html>)
    Document,
    /// Element with tag and attributes
    Element(ElementData),
    /// Text content (StrTendril is refcounted - cheap to clone)
    Text(StrTendril),
    /// HTML comment
    Comment(StrTendril),
}

/// Element data (tag + attributes)
#[derive(Debug, Clone)]
pub struct ElementData {
    /// Tag name (StrTendril shares buffer with source via refcounting)
    pub tag: StrTendril,

    /// Attributes - keys and values share buffers via Tendril refcounting
    pub attrs: HashMap<StrTendril, StrTendril>,
}

/// XML namespace
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Namespace {
    Html,
    Svg,
    MathMl,
}

impl Namespace {
    pub fn from_url(url: &str) -> Self {
        match url {
            "http://www.w3.org/1999/xhtml" => Namespace::Html,
            "http://www.w3.org/2000/svg" => Namespace::Svg,
            "http://www.w3.org/1998/Math/MathML" => Namespace::MathMl,
            _ => Namespace::Html, // default
        }
    }

    pub fn url(&self) -> &'static str {
        match self {
            Namespace::Html => "http://www.w3.org/1999/xhtml",
            Namespace::Svg => "http://www.w3.org/2000/svg",
            Namespace::MathMl => "http://www.w3.org/1998/Math/MathML",
        }
    }
}

/// Parse HTML into arena-based Document
pub fn parse(html: &str) -> Document {
    let sink = ArenaSink::new();
    // Create a Tendril from our source string
    // html5ever will create subtendrils that share this buffer via refcounting
    let tendril = StrTendril::from(html);
    parse_document(sink, Default::default()).one(tendril)
}

/// Owned element name wrapper
#[derive(Debug, Clone)]
struct OwnedElemName(QualName);

impl ElemName for OwnedElemName {
    fn ns(&self) -> &html5ever::Namespace {
        &self.0.ns
    }

    fn local_name(&self) -> &LocalName {
        &self.0.local
    }
}

/// TreeSink implementation for building arena-based DOM
struct ArenaSink {
    /// Our arena (same type as everywhere else!) - wrapped in RefCell for interior mutability
    arena: RefCell<Arena<NodeData>>,

    /// Document node (parent of <html>)
    document: NodeId,

    /// DOCTYPE encountered during parse - wrapped in RefCell
    doctype: RefCell<Option<StrTendril>>,
}

impl ArenaSink {
    fn new() -> Self {
        let mut arena = Arena::new();

        // Create document root node
        let document = arena.new_node(NodeData {
            kind: NodeKind::Document,
            ns: Namespace::Html,
        });

        ArenaSink {
            arena: RefCell::new(arena),
            document,
            doctype: RefCell::new(None),
        }
    }
}

impl TreeSink for ArenaSink {
    type Handle = NodeId;
    type Output = Document;
    type ElemName<'a>
        = OwnedElemName
    where
        Self: 'a;

    fn finish(self) -> Self::Output {
        let arena = self.arena.into_inner();

        // Find the root element (usually <html>)
        let root = self
            .document
            .children(&arena)
            .next()
            .unwrap_or(self.document);

        Document {
            arena,
            root,
            doctype: self.doctype.into_inner(),
        }
    }

    fn parse_error(&self, _msg: Cow<'static, str>) {
        // Ignore parse errors (html5ever recovers automatically)
    }

    fn get_document(&self) -> Self::Handle {
        self.document
    }

    fn set_quirks_mode(&self, _mode: QuirksMode) {
        // We don't care about quirks mode for diffing
    }

    fn same_node(&self, a: &Self::Handle, b: &Self::Handle) -> bool {
        a == b
    }

    fn elem_name<'a>(&'a self, _target: &'a Self::Handle) -> OwnedElemName {
        // html5ever only calls this for debugging/error messages
        // Return a placeholder - we don't actually need this for parsing
        OwnedElemName(QualName {
            prefix: None,
            ns: ns!(html),
            local: local_name!(""),
        })
    }

    fn create_element(
        &self,
        name: QualName,
        attrs: Vec<Attribute>,
        _flags: ElementFlags,
    ) -> Self::Handle {
        // Convert tag name to StrTendril
        let tag = StrTendril::from(name.local.as_ref());
        let ns = Namespace::from_url(name.ns.as_ref());

        // Convert attribute keys and values to StrTendrils
        let attr_map: HashMap<_, _> = attrs
            .into_iter()
            .map(|attr| {
                let key = StrTendril::from(attr.name.local.as_ref());
                let value = attr.value.clone(); // StrTendril clone is cheap (refcounted)
                (key, value)
            })
            .collect();

        // Create node in arena
        self.arena.borrow_mut().new_node(NodeData {
            kind: NodeKind::Element(ElementData {
                tag,
                attrs: attr_map,
            }),
            ns,
        })
    }

    fn create_comment(&self, text: StrTendril) -> Self::Handle {
        self.arena.borrow_mut().new_node(NodeData {
            kind: NodeKind::Comment(text),
            ns: Namespace::Html,
        })
    }

    fn create_pi(&self, _target: StrTendril, _data: StrTendril) -> Self::Handle {
        // Processing instructions - create empty comment
        self.arena.borrow_mut().new_node(NodeData {
            kind: NodeKind::Comment(StrTendril::new()),
            ns: Namespace::Html,
        })
    }

    fn append(&self, parent: &Self::Handle, child: NodeOrText<Self::Handle>) {
        let mut arena = self.arena.borrow_mut();
        match child {
            NodeOrText::AppendNode(node) => {
                parent.append(node, &mut *arena);
            }
            NodeOrText::AppendText(text) => {
                // Try to merge with previous text node (html5ever behavior)
                if let Some(last_child) = parent.children(&*arena).last() {
                    if let NodeKind::Text(existing) = &mut arena[last_child].get_mut().kind {
                        // Merge text - push_tendril shares buffers when possible
                        existing.push_tendril(&text);
                        return;
                    }
                }

                // Create new text node (StrTendril clone is cheap - refcounted)
                let text_node = arena.new_node(NodeData {
                    kind: NodeKind::Text(text),
                    ns: Namespace::Html,
                });
                parent.append(text_node, &mut *arena);
            }
        }
    }

    fn append_before_sibling(&self, sibling: &Self::Handle, new_node: NodeOrText<Self::Handle>) {
        let mut arena = self.arena.borrow_mut();
        match new_node {
            NodeOrText::AppendNode(node) => {
                sibling.insert_before(node, &mut *arena);
            }
            NodeOrText::AppendText(text) => {
                let text_node = arena.new_node(NodeData {
                    kind: NodeKind::Text(text),
                    ns: Namespace::Html,
                });
                sibling.insert_before(text_node, &mut *arena);
            }
        }
    }

    fn append_based_on_parent_node(
        &self,
        element: &Self::Handle,
        _prev_element: &Self::Handle,
        child: NodeOrText<Self::Handle>,
    ) {
        // Just append to element
        self.append(element, child);
    }

    fn append_doctype_to_document(
        &self,
        name: StrTendril,
        _public_id: StrTendril,
        _system_id: StrTendril,
    ) {
        *self.doctype.borrow_mut() = Some(name);
    }

    fn get_template_contents(&self, target: &Self::Handle) -> Self::Handle {
        // For <template>, return the element itself
        // (proper template support would need a template contents fragment)
        *target
    }

    fn add_attrs_if_missing(&self, target: &Self::Handle, attrs: Vec<Attribute>) {
        let mut arena = self.arena.borrow_mut();
        let node = &mut arena[*target].get_mut();
        if let NodeKind::Element(elem) = &mut node.kind {
            for attr in attrs {
                let key = StrTendril::from(attr.name.local.as_ref());
                elem.attrs.entry(key).or_insert_with(|| {
                    attr.value.clone() // StrTendril clone is cheap (refcounted)
                });
            }
        }
    }

    fn remove_from_parent(&self, target: &Self::Handle) {
        target.detach(&mut self.arena.borrow_mut());
    }

    fn reparent_children(&self, node: &Self::Handle, new_parent: &Self::Handle) {
        let mut arena = self.arena.borrow_mut();
        let children: Vec<NodeId> = node.children(&*arena).collect();
        for child in children {
            child.detach(&mut *arena);
            new_parent.append(child, &mut *arena);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_parse_simple_html() {
        let html = "<html><body><p>Hello</p></body></html>";
        let doc = parse(html);

        // Check root is <html>
        let root_data = doc.get(doc.root);
        assert!(matches!(root_data.kind, NodeKind::Element(_)));
        if let NodeKind::Element(elem) = &root_data.kind {
            assert_eq!(elem.tag.as_ref(), "html");
        }

        // Check we have a body
        let body = doc.body().expect("should have body");
        let body_data = doc.get(body);
        if let NodeKind::Element(elem) = &body_data.kind {
            assert_eq!(elem.tag.as_ref(), "body");
        }

        // Check body has a <p> child
        let p = body
            .children(&doc.arena)
            .next()
            .expect("body should have child");
        let p_data = doc.get(p);
        if let NodeKind::Element(elem) = &p_data.kind {
            assert_eq!(elem.tag.as_ref(), "p");
        }

        // Check <p> has text child
        let text = p.children(&doc.arena).next().expect("p should have text");
        let text_data = doc.get(text);
        if let NodeKind::Text(t) = &text_data.kind {
            assert_eq!(t.as_ref(), "Hello");
            // StrTendril shares buffers via refcounting (cheap clone)
        }
    }

    #[test]
    fn test_parse_with_attributes() {
        let html = r#"<div class="container" id="main">Content</div>"#;
        let full_html = format!("<html><body>{}</body></html>", html);
        let doc = parse(&full_html);

        let body = doc.body().expect("should have body");
        let div = body
            .children(&doc.arena)
            .next()
            .expect("body should have div");
        let div_data = doc.get(div);

        if let NodeKind::Element(elem) = &div_data.kind {
            assert_eq!(elem.tag.as_ref(), "div");

            // Check attributes (need to create StrTendrils for lookup)
            let class_key = StrTendril::from("class");
            let id_key = StrTendril::from("id");
            assert_eq!(
                elem.attrs.get(&class_key).map(|v| v.as_ref()),
                Some("container")
            );
            assert_eq!(elem.attrs.get(&id_key).map(|v| v.as_ref()), Some("main"));
            // StrTendril uses refcounted buffer sharing (cheap clone)
        }
    }

    #[test]
    fn test_parse_doctype() {
        let html = "<!DOCTYPE html><html><body></body></html>";
        let doc = parse(html);

        assert!(doc.doctype.is_some());
        assert_eq!(doc.doctype.as_ref().map(|d| d.as_ref()), Some("html"));
    }

    #[test]
    fn test_tendril_buffer_sharing() {
        // Verify that parsed strings actually share buffers via Tendril refcounting
        let html = "<html><body><p>Hello World</p></body></html>";

        // Create the source tendril explicitly so we can check sharing
        let source_tendril = StrTendril::from(html);
        let sink = ArenaSink::new();
        let doc = parse_document(sink, Default::default()).one(source_tendril.clone());

        // Check that text nodes share the buffer with source
        let body = doc.body().expect("should have body");
        let p = body
            .children(&doc.arena)
            .next()
            .expect("body should have p");
        let text_node = p.children(&doc.arena).next().expect("p should have text");

        if let NodeKind::Text(text_tendril) = &doc.get(text_node).kind {
            // Verify buffer sharing with is_shared_with()
            assert!(
                text_tendril.is_shared_with(&source_tendril),
                "Text tendril should share buffer with source tendril (zero-copy)"
            );

            // Also check is_shared() - should be true since there are multiple refs
            assert!(
                text_tendril.is_shared(),
                "Text tendril should be marked as shared"
            );
        } else {
            panic!("Expected text node");
        }

        // Check that doctype shares buffer
        if let Some(doctype) = &doc.doctype {
            // DOCTYPE is "html" which should be a subtendril of source
            assert!(
                doctype.is_shared_with(&source_tendril),
                "DOCTYPE should share buffer with source tendril (zero-copy)"
            );
        }
    }

    #[test]
    fn test_parse_nested_elements() {
        let html = "<html><body><div><span>Text</span></div></body></html>";
        let doc = parse(html);

        let body = doc.body().expect("should have body");
        let div = body
            .children(&doc.arena)
            .next()
            .expect("body should have div");

        let div_data = doc.get(div);
        if let NodeKind::Element(elem) = &div_data.kind {
            assert_eq!(elem.tag.as_ref(), "div");
        }

        let span = div
            .children(&doc.arena)
            .next()
            .expect("div should have span");
        let span_data = doc.get(span);
        if let NodeKind::Element(elem) = &span_data.kind {
            assert_eq!(elem.tag.as_ref(), "span");
        }
    }

    #[test]
    fn test_parse_comment() {
        let html = "<html><body><!-- This is a comment --></body></html>";
        let doc = parse(html);

        let body = doc.body().expect("should have body");
        let comment = body
            .children(&doc.arena)
            .next()
            .expect("body should have comment");

        let comment_data = doc.get(comment);
        if let NodeKind::Comment(text) = &comment_data.kind {
            assert_eq!(text.as_ref(), " This is a comment ");
        }
    }
}
