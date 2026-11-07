//! Small DOM helpers shared by the rule executor and the presentation layer.

use std::cell::RefCell;

use html5ever::tendril::StrTendril;
use html5ever::{ns, Attribute, LocalName, QualName};
use markup5ever_rcdom::{Handle, Node, NodeData};

/// Build an element node with 'attrs'.
pub(crate) fn element(name: &str, attrs: Vec<Attribute>) -> Handle {
    Node::new(NodeData::Element {
        name: QualName::new(None, ns!(html), LocalName::from(name)),
        attrs: RefCell::new(attrs),
        template_contents: RefCell::new(None),
        mathml_annotation_xml_integration_point: false,
    })
}

/// Build a single attribute.
pub(crate) fn make_attr(name: &str, value: &str) -> Attribute {
    Attribute {
        name: QualName::new(None, ns!(), LocalName::from(name)),
        value: StrTendril::from(value),
    }
}

/// Set 'name' to 'value', replacing any existing attribute.
pub(crate) fn set_attr(node: &Handle, name: &str, value: &str) {
    let NodeData::Element { attrs, .. } = &node.data else {
        return;
    };
    let mut attrs = attrs.borrow_mut();
    if let Some(existing) = attrs
        .iter_mut()
        .find(|a| a.name.local.as_ref().eq_ignore_ascii_case(name))
    {
        existing.value = StrTendril::from(value);
    } else {
        attrs.push(make_attr(name, value));
    }
}

/// Drop every attribute whose name 'keep' rejects.
pub(crate) fn retain_attrs(node: &Handle, mut keep: impl FnMut(&str) -> bool) {
    let NodeData::Element { attrs, .. } = &node.data else {
        return;
    };
    attrs.borrow_mut().retain(|a| keep(a.name.local.as_ref()));
}

/// Drop 'name' when present.
pub(crate) fn remove_attr(node: &Handle, name: &str) {
    let NodeData::Element { attrs, .. } = &node.data else {
        return;
    };
    attrs
        .borrow_mut()
        .retain(|a| !a.name.local.as_ref().eq_ignore_ascii_case(name));
}

/// Replace the child at 'index'.
pub(crate) fn replace_at(parent: &Handle, index: usize, new: Handle) {
    let mut children = parent.children.borrow_mut();
    if index < children.len() {
        children[index] = new;
    }
}

/// Remove and return the child at 'index'.
pub(crate) fn remove_at(parent: &Handle, index: usize) -> Option<Handle> {
    let mut children = parent.children.borrow_mut();
    if index < children.len() {
        Some(children.remove(index))
    } else {
        None
    }
}

/// Detach every child of 'node' and return them.
pub(crate) fn take_children(node: &Handle) -> Vec<Handle> {
    std::mem::take(&mut *node.children.borrow_mut())
}

/// Append 'child' to 'parent'.
pub(crate) fn append(parent: &Handle, child: Handle) {
    parent.children.borrow_mut().push(child);
}

/// Insert 'child' at 'index' (clamped to the end).
pub(crate) fn insert_at(parent: &Handle, index: usize, child: Handle) {
    let mut children = parent.children.borrow_mut();
    let index = index.min(children.len());
    children.insert(index, child);
}

/// Build a text node.
pub(crate) fn text_node(text: &str) -> Handle {
    Node::new(NodeData::Text {
        contents: RefCell::new(StrTendril::from(text)),
    })
}
