use browser_proto::{DomAttr, DomNode};
use hotmeal::Document;

/// Convert hotmeal's Document body to DomNode tree for comparison with browser.
pub fn document_body_to_dom_node(doc: &Document) -> DomNode {
    let body = doc.body().expect("document has no body");
    node_to_dom_node(doc, body)
}

fn node_to_dom_node(doc: &Document, node_id: hotmeal::NodeId) -> DomNode {
    let node = doc.get(node_id);
    match &node.kind {
        hotmeal::NodeKind::Element(elem) => {
            let tag = elem.tag.to_string().to_ascii_lowercase();
            let mut attrs: Vec<DomAttr> = elem
                .attrs
                .iter()
                .map(|(qname, value)| DomAttr {
                    name: qname.local.to_string(),
                    value: value.as_ref().to_string(),
                })
                .collect();
            attrs.sort_by(|a, b| a.name.cmp(&b.name));
            let children: Vec<DomNode> = doc
                .children(node_id)
                .map(|child_id| node_to_dom_node(doc, child_id))
                .collect();
            DomNode::Element {
                tag,
                attrs,
                children,
            }
        }
        hotmeal::NodeKind::Text(text) => DomNode::Text(text.as_ref().to_string()),
        hotmeal::NodeKind::Comment(text) => DomNode::Comment(text.as_ref().to_string()),
        hotmeal::NodeKind::Document => DomNode::Text(String::new()),
    }
}

/// Pretty-print a DomNode tree for diffing.
pub fn pretty_print_dom(node: &DomNode, indent: usize) -> String {
    let mut out = String::new();
    let prefix = "  ".repeat(indent);
    match node {
        DomNode::Element {
            tag,
            attrs,
            children,
        } => {
            out.push_str(&format!("{}<{}", prefix, tag));
            for attr in attrs {
                out.push_str(&format!(" {}={:?}", attr.name, attr.value));
            }
            out.push_str(">\n");
            for child in children {
                out.push_str(&pretty_print_dom(child, indent + 1));
            }
            out.push_str(&format!("{}</{}>\n", prefix, tag));
        }
        DomNode::Text(text) => out.push_str(&format!("{}TEXT: {:?}\n", prefix, text)),
        DomNode::Comment(text) => out.push_str(&format!("{}COMMENT: {:?}\n", prefix, text)),
    }
    out
}
